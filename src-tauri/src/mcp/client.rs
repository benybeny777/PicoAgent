// mcp/client.rs - 外部 MCP サーバ（stdio）クライアント。
//
// 設定 PICOAGENT_MCP_SERVERS（1行1件「名前|コマンド 引数…」）で指定されたサーバを
// 起動時に spawn して initialize / tools/list を行い、得られたツールを
// `mcp__<サーバ名>__<ツール名>` としてエージェントのツール候補へ動的登録する。
// 実行（tools/call）は tools::dispatch から本モジュールへフォールバックされる。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use super::{sanitize_name, PROTOCOL_VERSION};

const INIT_TIMEOUT_SECS: u64 = 30;
const CALL_TIMEOUT_SECS: u64 = 120;

/// 動的登録された外部 MCP ツールの情報。
pub struct McpToolInfo {
    /// 接続キー（正規化済みサーバ名）。
    pub server_key: String,
    /// サーバが公開している生のツール名（tools/call に渡す）。
    pub tool_raw: String,
    /// エージェントへ見せる完全名（mcp__<サーバ>__<ツール>）。
    pub full_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// MCP サーバの1行あたりの受信上限。
///
/// `read_line` には長さの上限が無く、改行を出さずに送り続けるサーバがいると
/// 応答待ちのタイムアウトが切れるまでメモリを伸ばし続ける。外部から来る入力は
/// 他の経路（更新フィード256KB、ローカルAPIボディ1MB、コマンド出力16KB）と同じく
/// 必ず上限を持たせる。ツール結果を運ぶので、それらより緩めに取る。
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

struct Connection {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

/// 改行までを上限付きで読む。`None` は相手が終了したことを表す。
///
/// `read_line` は上限を持たないため、改行を出さないサーバに付き合ってメモリを
/// 伸ばし続けてしまう。ここで打ち切り、接続は呼び出し元が破棄する。
async fn read_line_limited<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<String>, String> {
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        let (chunk, used, finished) = {
            let available = reader
                .fill_buf()
                .await
                .map_err(|error| format!("MCP サーバからの読み取りに失敗: {error}"))?;
            if available.is_empty() {
                (Vec::new(), 0, true)
            } else {
                match available.iter().position(|byte| *byte == b'\n') {
                    Some(position) => (available[..position].to_vec(), position + 1, true),
                    None => (available.to_vec(), available.len(), false),
                }
            }
        };
        reader.consume(used);
        if used == 0 && finished {
            return Ok(if bytes.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&bytes).into_owned())
            });
        }
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!(
                "MCP サーバの応答が1行 {max_bytes} バイトを超えました。接続を切ります。"
            ));
        }
        bytes.extend_from_slice(&chunk);
        if finished {
            // 行はここで完結している。JSON-RPC はUTF-8だが、サーバのログ行が
            // 混ざることもあるので不正バイトは置換して読み飛ばし判定へ回す。
            return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
        }
    }
}

fn tools_registry() -> &'static RwLock<Vec<McpToolInfo>> {
    static TOOLS: OnceLock<RwLock<Vec<McpToolInfo>>> = OnceLock::new();
    TOOLS.get_or_init(|| RwLock::new(Vec::new()))
}

#[allow(clippy::type_complexity)]
fn connections() -> &'static RwLock<HashMap<String, Arc<Mutex<Connection>>>> {
    static CONNECTIONS: OnceLock<RwLock<HashMap<String, Arc<Mutex<Connection>>>>> = OnceLock::new();
    CONNECTIONS.get_or_init(|| RwLock::new(HashMap::new()))
}

impl Connection {
    /// JSON-RPC リクエストを送り、同じ id の応答が返るまで読む（改行区切り JSON）。
    /// 改行までを上限付きで読む。`None` は相手が終了したことを表す。
    async fn read_line_limited(&mut self) -> Result<Option<String>, String> {
        read_line_limited(&mut self.reader, MAX_LINE_BYTES).await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.write_line(&message).await?;
        loop {
            let Some(line) = self.read_line_limited().await? else {
                return Err("MCP サーバが終了しました".to_string());
            };
            let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                continue; // JSON でない行（サーバのログ等）は読み飛ばす
            };
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // サーバ発の通知・他応答は読み飛ばす
            }
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("不明なエラー");
                return Err(format!("MCP エラー: {message}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn notify(&mut self, method: &str) -> Result<(), String> {
        let message = json!({ "jsonrpc": "2.0", "method": method });
        self.write_line(&message).await
    }

    async fn write_line(&mut self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("MCP サーバへの書き込みに失敗: {error}"))
    }
}

/// 設定文字列を (名前, コマンドライン) の一覧へ解析する。
/// 形式: 1行1件「名前|コマンド 引数…」。空行と # 始まりはコメントとして無視。
pub fn parse_server_lines(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (name, command) = line.split_once('|')?;
            let name = name.trim();
            let command = command.trim();
            if name.is_empty() || command.is_empty() {
                return None;
            }
            Some((name.to_string(), command.to_string()))
        })
        .collect()
}

/// 1つの MCP サーバを起動して初期化し、ツールを登録する。登録できたツール数を返す。
pub async fn connect(name: &str, command_line: &str) -> Result<usize, String> {
    let mut parts = command_line.split_whitespace();
    let program = parts.next().ok_or("コマンドが空です")?;
    let args: Vec<&str> = parts.collect();
    // stdio は JSON-RPC のため個別に指定する（stdin も開く）。窓抑止・kill_on_drop は共通層。
    let mut command = crate::tools::process::command(program);
    command
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("MCP サーバを起動できません（{program}）: {error}"))?;
    let stdin = child.stdin.take().ok_or("stdin を取得できません")?;
    let stdout = child.stdout.take().ok_or("stdout を取得できません")?;
    let mut connection = Connection {
        child,
        stdin,
        reader: BufReader::new(stdout),
        next_id: 1,
    };

    let init = tokio::time::timeout(
        Duration::from_secs(INIT_TIMEOUT_SECS),
        connection.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "PicoAgent", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
    )
    .await
    .map_err(|_| "initialize がタイムアウトしました".to_string())??;
    let _ = init; // サーバ情報は現状未使用
    connection.notify("notifications/initialized").await?;

    let listed = tokio::time::timeout(
        Duration::from_secs(INIT_TIMEOUT_SECS),
        connection.request("tools/list", json!({})),
    )
    .await
    .map_err(|_| "tools/list がタイムアウトしました".to_string())??;

    let server_key = sanitize_name(name);
    let mut added = 0usize;
    {
        let mut registry = tools_registry().write().unwrap();
        // 同名サーバの再接続時は旧登録を除去する。
        registry.retain(|info| info.server_key != server_key);
        for tool in listed
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let Some(tool_raw) = tool.get("name").and_then(Value::as_str) else {
                continue;
            };
            let full_name = format!("mcp__{}__{}", server_key, sanitize_name(tool_raw));
            registry.push(McpToolInfo {
                server_key: server_key.clone(),
                tool_raw: tool_raw.to_string(),
                full_name,
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                input_schema: tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object" })),
            });
            added += 1;
        }
    }
    connections()
        .write()
        .unwrap()
        .insert(server_key, Arc::new(Mutex::new(connection)));
    Ok(added)
}

/// 設定に基づいて全 MCP サーバを起動する（設定が空なら何もしない）。
pub fn start(app: tauri::AppHandle) {
    let state = app.state::<crate::commands::AppState>();
    let raw = state.config.get("PICOAGENT_MCP_SERVERS", "");
    let servers = parse_server_lines(&raw);
    if servers.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        for (name, command_line) in servers {
            match connect(&name, &command_line).await {
                Ok(count) => crate::log_paths::append_app_log(
                    &app,
                    &format!("MCP サーバ「{name}」を接続しました（ツール {count} 件）。"),
                ),
                Err(error) => crate::log_paths::append_app_log(
                    &app,
                    &format!("MCP サーバ「{name}」の接続に失敗しました: {error}"),
                ),
            }
        }
    });
}

/// アプリ終了時に全 MCP サーバの子プロセスを停止する。
pub fn stop_all() {
    let mut map = connections().write().unwrap();
    let drained = map.drain().collect::<Vec<_>>();
    drop(map);
    tauri::async_runtime::block_on(async move {
        for (name, connection) in drained {
            let Ok(lock) = tokio::time::timeout(Duration::from_secs(2), connection.lock()).await
            else {
                crate::log_paths::log_detached(&format!(
                    "MCP サーバ「{name}」は処理中のため終了待ちがタイムアウトしました"
                ));
                continue;
            };
            let mut guard = lock;
            match guard.child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if let Err(error) = guard.child.kill().await {
                        crate::log_paths::log_detached(&format!(
                            "MCP サーバ「{name}」の停止と回収に失敗しました: {error}"
                        ));
                    }
                }
                Err(error) => {
                    crate::log_paths::log_detached(&format!(
                        "MCP サーバ「{name}」の終了状態を確認できません: {error}"
                    ));
                }
            }
        }
    });
    tools_registry().write().unwrap().clear();
}

/// この名前が外部 MCP ツールかどうか。
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp__")
}

/// エージェントへ渡す function スキーマ（OpenAI/Ollama 共通形式）を返す。
pub fn schemas() -> Vec<Value> {
    tools_registry()
        .read()
        .unwrap()
        .iter()
        .map(|info| {
            json!({
                "type": "function",
                "function": {
                    "name": info.full_name,
                    "description": format!("[MCP:{}] {}", info.server_key, info.description),
                    "parameters": info.input_schema,
                }
            })
        })
        .collect()
}

/// 外部 MCP ツールを実行し、テキスト結果を返す（失敗はエラーメッセージ文字列）。
pub async fn dispatch(full_name: &str, args: Value) -> String {
    let (server_key, tool_raw) = {
        let registry = tools_registry().read().unwrap();
        match registry.iter().find(|info| info.full_name == full_name) {
            Some(info) => (info.server_key.clone(), info.tool_raw.clone()),
            None => {
                return format!("エラー: 未登録の MCP ツール \"{full_name}\" が呼び出されました。")
            }
        }
    };
    let connection = {
        let map = connections().read().unwrap();
        match map.get(&server_key) {
            Some(connection) => connection.clone(),
            None => return format!("エラー: MCP サーバ「{server_key}」に接続していません。"),
        }
    };
    let mut guard = connection.lock().await;
    let result = tokio::time::timeout(
        Duration::from_secs(CALL_TIMEOUT_SECS),
        guard.request("tools/call", json!({ "name": tool_raw, "arguments": args })),
    )
    .await;
    // タイムアウトや I/O 失敗のあとは接続を再利用しない。tokio の read_line は
    // キャンセル安全ではなく、途中で打ち切ると読みかけの行が失われてストリームが
    // ずれるため、以降の応答が別リクエストの結果と混ざる恐れがある。
    let cleanup_error = if !matches!(result, Ok(Ok(_))) {
        let cleanup_error = match guard.child.try_wait() {
            Ok(Some(_)) => None,
            Ok(None) => guard
                .child
                .kill()
                .await
                .err()
                .map(|error| format!("MCP サーバの停止と回収にも失敗しました: {error}")),
            Err(error) => Some(format!("MCP サーバの終了状態を確認できません: {error}")),
        };
        drop(guard);
        connections().write().unwrap().remove(&server_key);
        cleanup_error
    } else {
        None
    };
    let mut output = match result {
        Ok(Ok(value)) => {
            let text: String = value
                .get("content")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let is_error = value.get("isError").and_then(Value::as_bool).unwrap_or(false);
            if is_error {
                format!("エラー: MCP ツールが失敗を返しました:\n{text}")
            } else if text.is_empty() {
                "（MCP ツールはテキストを返しませんでした）".to_string()
            } else {
                text
            }
        }
        Ok(Err(error)) => format!("エラー: {error}（MCP サーバ「{server_key}」との接続を切断しました。再接続には再起動が必要です）"),
        Err(_) => format!(
            "エラー: MCP ツール実行が {CALL_TIMEOUT_SECS} 秒でタイムアウトしました（MCP サーバ「{server_key}」との接続を切断しました）。"
        ),
    };
    if let Some(cleanup_error) = cleanup_error {
        output.push_str(&format!("\n{cleanup_error}"));
    }
    output
}

#[cfg(test)]
mod tests {
    /// 上限が無いと、改行を出さないサーバに付き合ってメモリを伸ばし続ける。
    #[test]
    fn 上限を超える行は読み切らずに断る() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 改行あり: そのまま1行として返す。
            let mut reader = tokio::io::BufReader::new(&b"{\"id\":1}\nrest"[..]);
            let line = super::read_line_limited(&mut reader, 64).await.unwrap();
            assert_eq!(line.as_deref(), Some("{\"id\":1}"));

            // 改行なしで上限超え: エラーで打ち切る。
            let huge = [b'x'; 200];
            let mut reader = tokio::io::BufReader::new(&huge[..]);
            let error = super::read_line_limited(&mut reader, 64)
                .await
                .expect_err("上限を超えた行を通した");
            assert!(error.contains("64 バイトを超えました"), "error: {error}");

            // 相手が何も出さずに終了: None（終了として扱う）。
            let mut reader = tokio::io::BufReader::new(&b""[..]);
            assert!(super::read_line_limited(&mut reader, 64)
                .await
                .unwrap()
                .is_none());

            // 改行なしで終了した最終行も読み落とさない。
            let mut reader = tokio::io::BufReader::new(&b"tail"[..]);
            let line = super::read_line_limited(&mut reader, 64).await.unwrap();
            assert_eq!(line.as_deref(), Some("tail"));
        });
    }

    use super::{connect, dispatch, is_mcp_tool, parse_server_lines, schemas};
    use serde_json::json;

    #[test]
    fn サーバ設定行を解析できる() {
        let raw = "memory|npx -y @modelcontextprotocol/server-memory\n# コメント\n\nfs | node server.js --root .\n壊れた行";
        let servers = parse_server_lines(raw);
        assert_eq!(servers.len(), 2);
        assert_eq!(
            servers[0],
            (
                "memory".to_string(),
                "npx -y @modelcontextprotocol/server-memory".to_string()
            )
        );
        assert_eq!(servers[1].0, "fs");
        assert!(is_mcp_tool("mcp__memory__store"));
        assert!(!is_mcp_tool("web_search"));
    }

    // node がある環境だけで走る E2E（無ければ即成功で抜ける）。
    // 最小のスタブ MCP サーバ（echo ツール1件）を stdio で起動し、
    // initialize → tools/list → tools/call の一連の流れを検証する。
    #[test]
    fn スタブmcpサーバと接続してツールを実行できる() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("node が無いためスキップ");
            return;
        }
        let dir = std::env::temp_dir().join(format!("picoagent-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stub = dir.join("stub-mcp.js");
        std::fs::write(&stub, r#"
const readline = require('readline');
const rl = readline.createInterface({ input: process.stdin });
rl.on('line', (line) => {
  let m; try { m = JSON.parse(line); } catch { return; }
  const reply = (result) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\n');
  if (m.method === 'initialize') reply({ protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'stub', version: '1.0.0' } });
  else if (m.method === 'tools/list') reply({ tools: [{ name: 'echo', description: 'そのまま返す', inputSchema: { type: 'object', properties: { text: { type: 'string' } } } }] });
  else if (m.method === 'tools/call') reply({ content: [{ type: 'text', text: 'echo: ' + ((m.params.arguments || {}).text || '') }] });
});
"#).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let count = connect("stub", &format!("node {}", stub.display()))
                .await
                .expect("接続できるはず");
            assert_eq!(count, 1);
            let schemas = schemas();
            assert!(
                schemas
                    .iter()
                    .any(|schema| schema["function"]["name"] == "mcp__stub__echo"),
                "schemas: {schemas:?}"
            );
            let result = dispatch("mcp__stub__echo", json!({ "text": "やあ" })).await;
            assert_eq!(result, "echo: やあ");
            let missing = dispatch("mcp__stub__nothing", json!({})).await;
            assert!(missing.contains("未登録"), "missing: {missing}");
        });
        super::stop_all();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
