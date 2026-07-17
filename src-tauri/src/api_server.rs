// api_server.rs - Ollama 風のローカル HTTP API サーバ。
//
// 外部ツールやスクリプトから PicoAgent のエージェント（スキル込み）を叩けるようにする。
// セキュリティ方針:
//   - 既定で無効（PICOAGENT_API_ENABLED=true で有効化。変更は再起動後に反映）
//   - 127.0.0.1 のみバインド（LAN へは公開しない）
//   - PICOAGENT_API_KEY を設定すると Authorization: Bearer <key> を必須にする
//
// エンドポイント（Ollama 互換風 + OpenAI 互換）:
//   GET  /api/version          … バージョン情報
//   GET  /api/tags             … 選択中モデルの一覧（Ollama の /api/tags 風）
//   POST /api/chat             … {messages:[{role,content}]} → エージェント応答（1問1答）
//   POST /api/generate         … {prompt} → エージェント応答
//   POST /v1/chat/completions  … OpenAI Chat Completions 互換（stream は単一チャンク SSE）
//
// 会話はリクエスト内で完結するステートレス設計（デスクトップの会話履歴とは混ざらない）。

use serde_json::{json, Value};
use tauri::Manager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::commands::AppState;
use crate::message::Message;

/// リクエストボディの上限（画像なしのチャット用途には十分な 1MB）。
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// ヘッダ部の上限。
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// 1 リクエストの処理タイムアウト（エージェントのツール実行を含むため長め）。
const REQUEST_TIMEOUT_SECS: u64 = 300;

/// 設定で有効な場合にローカル API サーバを起動する（無効なら何もしない）。
pub fn start(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    if !state.config.get_bool("PICOAGENT_API_ENABLED", false) {
        return;
    }
    let port_text = state.config.get("PICOAGENT_API_PORT", "11500");
    let port: u16 = match port_text.trim().parse() {
        Ok(port) if port > 0 => port,
        _ => {
            crate::log_paths::append_app_log(
                &app,
                &format!("ローカルAPIを開始できません: ポート番号が不正です: {port_text}"),
            );
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => listener,
            Err(error) => {
                crate::log_paths::append_app_log(
                    &app,
                    &format!("ローカルAPIの起動に失敗しました（ポート {port}）: {error}"),
                );
                return;
            }
        };
        crate::log_paths::append_app_log(
            &app,
            &format!("ローカルAPIを開始しました: http://127.0.0.1:{port}（Ollama風 + OpenAI互換）"),
        );
        loop {
            let (stream, _addr) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    crate::log_paths::append_app_log(
                        &app,
                        &format!("ローカルAPIの接続受付に失敗しました: {error}"),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS),
                    handle_connection(&app, stream),
                )
                .await;
                if let Ok(Err(error)) = result {
                    crate::log_paths::append_app_log(
                        &app,
                        &format!("ローカルAPIのリクエスト処理に失敗しました: {error}"),
                    );
                }
            });
        }
    });
}

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// ヘッダの (名前, 値) ペア一覧。
type HeaderList = Vec<(String, String)>;

/// ヘッダ部（リクエストライン + ヘッダ）を解析する。対応できない形式は None。
fn parse_head(head: &str) -> Option<(String, String, HeaderList)> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_uppercase();
    let path = parts.next()?.to_string();
    // クエリ文字列はルーティングでは使わないため除去する。
    let path = path.split('?').next().unwrap_or(&path).to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    Some((method, path, headers))
}

async fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    // ヘッダ終端（\r\n\r\n）まで読む。
    let head_end = loop {
        if let Some(pos) = find_head_end(&buf) {
            break pos;
        }
        if buf.len() > MAX_HEAD_BYTES {
            return Err("ヘッダが大きすぎます".to_string());
        }
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("読み取りに失敗しました: {error}"))?;
        if n == 0 {
            return Err("接続が閉じられました".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let (method, path, headers) =
        parse_head(&head).ok_or_else(|| "リクエストを解析できません".to_string())?;
    let content_length: usize = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err("ボディが大きすぎます（1MB まで）".to_string());
    }
    let mut body: Vec<u8> = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("ボディの読み取りに失敗しました: {error}"))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

/// PICOAGENT_API_KEY が設定されている場合、Bearer トークンの一致を検査する。
fn authorized(expected_key: &str, auth_header: Option<&str>) -> bool {
    let expected = expected_key.trim();
    if expected.is_empty() {
        return true;
    }
    match auth_header {
        Some(value) => value
            .strip_prefix("Bearer ")
            .map(|token| token.trim() == expected)
            .unwrap_or(false),
        None => false,
    }
}

async fn write_body(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), String> {
    stream
        .write_all(bytes)
        .await
        .map_err(|error| format!("応答の書き込みに失敗しました: {error}"))
}

/// OpenAI 互換の chat.completion 応答 JSON を組み立てる。
fn openai_completion_json(model: &str, reply: &str) -> Value {
    let now = chrono::Utc::now().timestamp();
    json!({
        "id": format!("chatcmpl-picoagent-{now}"),
        "object": "chat.completion",
        "created": now,
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": reply },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
    })
}

/// リクエスト JSON の messages 配列をエージェント用 Message 列へ変換する。
fn parse_messages(body: &Value) -> Vec<Message> {
    body.get("messages")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let role = item.get("role").and_then(Value::as_str)?;
                    let content = item.get("content").and_then(Value::as_str)?;
                    if !matches!(role, "system" | "user" | "assistant") {
                        return None;
                    }
                    Some(Message::new(role, content))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// エージェント（スキル込み）を 1 リクエスト分だけステートレスに実行する。
async fn run_agent(
    app: &tauri::AppHandle,
    mut messages: Vec<Message>,
    model_override: Option<&str>,
) -> Result<(String, String), String> {
    let state = app.state::<AppState>();
    let provider_id = state.provider_id.lock().unwrap().clone();
    let selected_model = state.model.lock().unwrap().clone();
    let model = model_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or(selected_model);
    let provider = crate::providers::get(&provider_id)
        .ok_or_else(|| format!("プロバイダ \"{provider_id}\" が見つかりません。"))?;
    if messages.is_empty() {
        return Err("messages または prompt が指定されていません。".to_string());
    }
    let reply = crate::agent::run(
        &state.config,
        provider,
        &model,
        &mut messages,
        5,
        None,
        None,
    )
    .await
    .map_err(|error| format!("エージェント実行に失敗しました: {error}"))?;
    Ok((model, reply))
}

async fn handle_connection(app: &tauri::AppHandle, mut stream: TcpStream) -> Result<(), String> {
    let request = read_request(&mut stream).await?;
    let state = app.state::<AppState>();
    let api_key = state.config.get("PICOAGENT_API_KEY", "");
    if !authorized(&api_key, request.header("authorization")) {
        return respond(&mut stream, 401, &json!({ "error": "認証に失敗しました。Authorization: Bearer <PICOAGENT_API_KEY> を付けてください。" })).await;
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/version") => {
            respond(&mut stream, 200, &json!({
                "app": "PicoAgent",
                "version": env!("CARGO_PKG_VERSION"),
            })).await
        }
        ("GET", "/api/tags") => {
            let provider_id = state.provider_id.lock().unwrap().clone();
            let model = state.model.lock().unwrap().clone();
            respond(&mut stream, 200, &json!({
                "models": [{
                    "name": model,
                    "model": model,
                    "details": { "provider": provider_id }
                }]
            })).await
        }
        ("POST", "/api/chat") => {
            let body: Value = match serde_json::from_slice(&request.body) {
                Ok(body) => body,
                Err(error) => {
                    return respond(&mut stream, 400, &json!({
                        "error": format!("JSON を解析できません: {error}")
                    })).await;
                }
            };
            let messages = parse_messages(&body);
            let model_override = body.get("model").and_then(Value::as_str);
            match run_agent(app, messages, model_override).await {
                Ok((model, reply)) => respond(&mut stream, 200, &json!({
                    "model": model,
                    "message": { "role": "assistant", "content": reply },
                    "done": true
                })).await,
                Err(error) => respond(&mut stream, 400, &json!({ "error": error })).await,
            }
        }
        ("POST", "/api/generate") => {
            let body: Value = match serde_json::from_slice(&request.body) {
                Ok(body) => body,
                Err(error) => {
                    return respond(&mut stream, 400, &json!({
                        "error": format!("JSON を解析できません: {error}")
                    })).await;
                }
            };
            let prompt = body.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let messages = if prompt.is_empty() { Vec::new() } else { vec![Message::new("user", &prompt)] };
            let model_override = body.get("model").and_then(Value::as_str);
            match run_agent(app, messages, model_override).await {
                Ok((model, reply)) => respond(&mut stream, 200, &json!({
                    "model": model,
                    "response": reply,
                    "done": true
                })).await,
                Err(error) => respond(&mut stream, 400, &json!({ "error": error })).await,
            }
        }
        ("POST", "/v1/chat/completions") => {
            let body: Value = match serde_json::from_slice(&request.body) {
                Ok(body) => body,
                Err(error) => {
                    return respond(&mut stream, 400, &json!({
                        "error": {
                            "message": format!("JSON を解析できません: {error}"),
                            "type": "invalid_request_error"
                        }
                    })).await;
                }
            };
            let messages = parse_messages(&body);
            let model_override = body.get("model").and_then(Value::as_str);
            let wants_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
            match run_agent(app, messages, model_override).await {
                Ok((model, reply)) => {
                    if wants_stream {
                        // ストリーミング要求には、全文を単一チャンクで返す最小 SSE 実装で応える。
                        write_sse_completion(&mut stream, &model, &reply).await
                    } else {
                        respond(&mut stream, 200, &openai_completion_json(&model, &reply)).await
                    }
                }
                Err(error) => respond(&mut stream, 400, &json!({ "error": { "message": error, "type": "invalid_request_error" } })).await,
            }
        }
        (_, "/api/version") | (_, "/api/tags") | (_, "/api/chat") | (_, "/api/generate") | (_, "/v1/chat/completions") => {
            respond(&mut stream, 405, &json!({ "error": "このメソッドは使えません。" })).await
        }
        _ => respond(&mut stream, 404, &json!({ "error": "エンドポイントが見つかりません。/api/version /api/tags /api/chat /api/generate /v1/chat/completions が使えます。" })).await,
    }
}

async fn respond(stream: &mut TcpStream, status: u16, body: &Value) -> Result<(), String> {
    let payload = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    write_body(stream, head.as_bytes()).await?;
    write_body(stream, payload.as_bytes()).await
}

/// OpenAI 互換ストリーミング（SSE）: 全文を 1 チャンク + [DONE] で返す最小実装。
async fn write_sse_completion(
    stream: &mut TcpStream,
    model: &str,
    reply: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    let chunk = json!({
        "id": format!("chatcmpl-picoagent-{now}"),
        "object": "chat.completion.chunk",
        "created": now,
        "model": model,
        "choices": [{ "index": 0, "delta": { "role": "assistant", "content": reply }, "finish_reason": null }]
    });
    let finish = json!({
        "id": format!("chatcmpl-picoagent-{now}"),
        "object": "chat.completion.chunk",
        "created": now,
        "model": model,
        "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
    });
    let payload = format!("data: {chunk}\n\ndata: {finish}\n\ndata: [DONE]\n\n");
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    write_body(stream, head.as_bytes()).await?;
    write_body(stream, payload.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::{authorized, find_head_end, openai_completion_json, parse_head, parse_messages};
    use serde_json::json;

    #[test]
    fn リクエストヘッダを解析できる() {
        let head = "POST /api/chat?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 12\r\nAuthorization: Bearer abc";
        let (method, path, headers) = parse_head(head).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/api/chat"); // クエリは除去される
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[2].1, "Bearer abc");
        assert!(parse_head("").is_none());
    }

    #[test]
    fn ヘッダ終端を検出できる() {
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n\r\nbody"), Some(14));
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn 認証はキー設定時のみ必須になる() {
        assert!(authorized("", None)); // キー未設定なら常に許可
        assert!(authorized("secret", Some("Bearer secret")));
        assert!(!authorized("secret", Some("Bearer wrong")));
        assert!(!authorized("secret", Some("secret"))); // Bearer 形式のみ
        assert!(!authorized("secret", None));
    }

    #[test]
    fn openai互換応答を組み立てる() {
        let value = openai_completion_json("gemma3", "こんにちは");
        assert_eq!(value["object"], "chat.completion");
        assert_eq!(value["choices"][0]["message"]["content"], "こんにちは");
        assert_eq!(value["choices"][0]["finish_reason"], "stop");
    }

    // 実 TCP ソケットで read_request → respond の往復を検証する（AppHandle 不要の HTTP 層のみ）。
    #[test]
    fn http層の読み書きができる() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = super::read_request(&mut stream).await.unwrap();
                assert_eq!(request.method, "POST");
                assert_eq!(request.path, "/api/chat");
                assert_eq!(request.header("authorization"), Some("Bearer k"));
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["messages"][0]["content"], "やあ");
                super::respond(&mut stream, 200, &json!({ "ok": true })).await.unwrap();
            });
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            let body = r#"{"messages":[{"role":"user","content":"やあ"}]}"#;
            let request = format!(
                "POST /api/chat HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer k\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            client.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).await.unwrap();
            assert!(response.starts_with("HTTP/1.1 200 OK"), "response: {response}");
            assert!(response.contains(r#"{"ok":true}"#), "response: {response}");
            server.await.unwrap();
        });
    }

    #[test]
    fn messages配列を変換できる() {
        let body = json!({ "messages": [
            { "role": "system", "content": "あなたは親切" },
            { "role": "user", "content": "やあ" },
            { "role": "tool", "content": "無視される" },
            { "role": "user" }
        ]});
        let messages = parse_messages(&body);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].content, "やあ");
        assert!(parse_messages(&json!({})).is_empty());
    }
}
