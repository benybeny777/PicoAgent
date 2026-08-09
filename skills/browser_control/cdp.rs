// skills/browser_control/cdp.rs - Chrome DevTools Protocol の最小クライアント。
//
// Node.js（Playwright / Puppeteer）はアプリの依存にしない決まりなので、ブラウザ操作は
// ユーザーの Chrome / Edge を `--remote-debugging-port` で起動し、CDP を直接叩いて行う。
// 使うのは `Page.navigate` と `Runtime.evaluate` の2つだけで、クリックや入力も
// ページ内 JS 経由に寄せている。CDP の面を最小に保つほどブラウザ更新で壊れにくい。
//
// 起動したブラウザは複数回の呼び出しにまたがって使い回すため、子プロセスは
// プロセス全体で1つだけ静的に保持する（同梱エンジンと同じ考え方）。

use std::path::PathBuf;
use std::process::Child;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

/// PicoAgent が起動したブラウザの子プロセス。`close` とアプリ終了時に停止する。
fn child_slot() -> &'static Mutex<Option<Child>> {
    static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    CHILD.get_or_init(|| Mutex::new(None))
}

fn lock_child_slot() -> std::sync::MutexGuard<'static, Option<Child>> {
    match child_slot().lock() {
        Ok(slot) => slot,
        // ブラウザのハンドルを失わないよう、poison 後も停止・回収できる状態を保つ。
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 既定のデバッグポート。同梱エンジン 11435 / 同梱 Aivis 11436 / ローカル API 11500 と
/// 重ならない値にする（重なると起動順によってどちらかが黙って使えなくなる）。
pub const DEFAULT_PORT: u16 = 11437;

fn probe_client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_millis(1200))
                .build()
                .ok()
        })
        .as_ref()
}

fn devtools_client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .ok()
        })
        .as_ref()
}

/// `--user-data-dir` に使う隔離プロファイルの置き場所。
fn isolated_profile_dir() -> Result<PathBuf, String> {
    Ok(dirs::data_dir()
        .ok_or_else(|| "エラー: ブラウザ用プロファイルの保存先を取得できません".to_string())?
        .join("PicoAgent")
        .join("browser-profile"))
}

/// 実行ファイルの候補（設定が空のときに順に探す）。
#[cfg(windows)]
fn browser_candidates() -> Vec<String> {
    let mut out = Vec::new();
    for base in [
        std::env::var("PROGRAMFILES").ok(),
        std::env::var("PROGRAMFILES(X86)").ok(),
        std::env::var("LOCALAPPDATA").ok(),
    ]
    .into_iter()
    .flatten()
    {
        out.push(format!("{base}\\Google\\Chrome\\Application\\chrome.exe"));
        out.push(format!("{base}\\Microsoft\\Edge\\Application\\msedge.exe"));
    }
    out
}

#[cfg(target_os = "macos")]
fn browser_candidates() -> Vec<String> {
    vec![
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".to_string(),
        "/Applications/Chromium.app/Contents/MacOS/Chromium".to_string(),
    ]
}

#[cfg(not(any(windows, target_os = "macos")))]
fn browser_candidates() -> Vec<String> {
    Vec::new()
}

/// 使うブラウザの実行パスを決める。優先順: 明示設定 → OS ごとの既定候補。
pub fn resolve_browser(configured: &str) -> Result<String, String> {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        if !PathBuf::from(trimmed).exists() {
            return Err(format!(
                "エラー: PICOAGENT_BROWSER_PATH のパスが見つかりません: {trimmed}"
            ));
        }
        return Ok(trimmed.to_string());
    }
    browser_candidates()
        .into_iter()
        .find(|path| PathBuf::from(path).exists())
        .ok_or_else(|| {
            "エラー: Chrome / Edge が見つかりません。設定画面の PICOAGENT_BROWSER_PATH に実行ファイルのパスを入力してください。"
                .to_string()
        })
}

/// デバッグポートが応答するか（既に起動済みか）を調べる。
async fn debug_port_alive(port: u16) -> bool {
    let Some(client) = probe_client() else {
        return false;
    };
    let Ok(response) = client
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .send()
        .await
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<Value>()
        .await
        .ok()
        .is_some_and(|value| {
            value.get("Browser").and_then(Value::as_str).is_some()
                && value
                    .get("webSocketDebuggerUrl")
                    .and_then(Value::as_str)
                    .is_some()
        })
}

pub struct LaunchOptions {
    pub browser_path: String,
    pub port: u16,
    pub headless: bool,
    /// true なら `--user-data-dir` を渡さず、ブラウザの既定プロファイル（＝ログイン済み）を使う。
    pub use_default_profile: bool,
    pub startup_timeout_secs: u64,
}

fn stop_child(child: &mut std::process::Child) -> Result<(), String> {
    if let Err(error) = child.kill() {
        if error.kind() != std::io::ErrorKind::InvalidInput {
            return Err(format!("ブラウザを停止できません: {error}"));
        }
    }
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("ブラウザの終了ハンドルを回収できません: {error}"))
}

fn managed_child_running() -> Result<bool, String> {
    let mut slot = lock_child_slot();
    match slot.as_mut() {
        Some(child) => match child.try_wait() {
            Ok(None) => Ok(true),
            Ok(Some(_)) => {
                *slot = None;
                Ok(false)
            }
            Err(error) => {
                let cleanup = stop_child(child);
                *slot = None;
                cleanup?;
                Err(format!("ブラウザの終了状態を確認できません: {error}"))
            }
        },
        None => Ok(false),
    }
}

/// ブラウザを起動する（既に応答していれば何もしない）。
pub async fn ensure_running(options: &LaunchOptions) -> Result<(), String> {
    if debug_port_alive(options.port).await {
        if managed_child_running()? {
            return Ok(());
        }
        return Err(format!(
            "エラー: デバッグポート {} は PicoAgent が管理していないブラウザに使われています。既存ブラウザのタブへ無断で接続しないため停止しました。ブラウザを閉じるか PICOAGENT_BROWSER_PORT を変更してください。",
            options.port
        ));
    }
    // 死んだ子プロセスが残っていれば片付けてから起動し直す。
    {
        let mut slot = lock_child_slot();
        if let Some(child) = slot.as_mut() {
            stop_child(child)?;
        }
        *slot = None;
    }

    let mut command = crate::tools::process::command_sync(&options.browser_path);
    command
        .arg(format!("--remote-debugging-port={}", options.port))
        .arg("--remote-debugging-address=127.0.0.1")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-popup-blocking");
    if !options.use_default_profile {
        let profile = isolated_profile_dir()?;
        if let Err(error) = std::fs::create_dir_all(&profile) {
            return Err(format!(
                "エラー: ブラウザ用プロファイルを作成できません（{}）: {error}",
                profile.display()
            ));
        }
        command.arg(format!("--user-data-dir={}", profile.display()));
    }
    if options.headless {
        command.arg("--headless=new");
    }
    command.arg("about:blank");
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|error| format!("エラー: ブラウザを起動できません: {error}"))?;
    {
        let mut slot = lock_child_slot();
        if slot.is_some() {
            stop_child(&mut child)?;
            return Err("エラー: PicoAgent 管理のブラウザは既に起動中です。".to_string());
        }
        *slot = Some(child);
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(options.startup_timeout_secs);
    while std::time::Instant::now() < deadline {
        if debug_port_alive(options.port).await {
            return Ok(());
        }
        // 既に同じブラウザが通常起動していると、起動要求は既存インスタンスへ引き継がれ、
        // こちらのプロセスはデバッグポートを開かないまますぐ終了する。待ち続けても
        // タイムアウトするだけなので、子プロセスの終了を見て原因を名指しで返す。
        if child_exited() {
            let mut error = format!(
                "エラー: ブラウザは既に通常起動しているため、デバッグポート（{}）付きの起動要求が既存のウィンドウへ引き継がれました。ブラウザをすべて閉じてからやり直すか、PICOAGENT_BROWSER_PROFILE=isolated（既定）で専用プロファイルを使ってください。",
                options.port
            );
            if let Err(cleanup) = stop() {
                error.push_str(&format!("\n{cleanup}"));
            }
            return Err(error);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let mut error = format!(
        "エラー: ブラウザのデバッグポート（{}）が応答しません。ブラウザの実行パス（PICOAGENT_BROWSER_PATH）と、ポートを塞いでいるプロセスが無いかを確認してください。",
        options.port
    );
    if let Err(cleanup) = stop() {
        error.push_str(&format!("\n{cleanup}"));
    }
    Err(error)
}

/// 起動した子プロセスが既に終了しているか。
fn child_exited() -> bool {
    let mut slot = lock_child_slot();
    match slot.as_mut() {
        Some(child) => matches!(child.try_wait(), Ok(Some(_))),
        None => false,
    }
}

/// 起動したブラウザを停止する（アプリ終了時と `close` から呼ぶ）。
///
/// **PicoAgent が起動してハンドルを保持しているブラウザだけ**を止める。
pub fn stop() -> Result<(), String> {
    let mut slot = lock_child_slot();
    if let Some(child) = slot.as_mut() {
        stop_child(child)?;
    }
    *slot = None;
    Ok(())
}

#[derive(Clone)]
pub struct Target {
    pub id: String,
    pub url: String,
    pub ws_url: String,
}

async fn devtools_get(port: u16, path: &str) -> Result<Value, String> {
    let client = devtools_client()
        .ok_or_else(|| "エラー: DevTools への接続準備に失敗しました。".to_string())?;
    let response = client
        .get(format!("http://127.0.0.1:{port}{path}"))
        .send()
        .await
        .map_err(|error| format!("エラー: DevTools への接続に失敗しました: {error}"))?;
    response
        .json::<Value>()
        .await
        .map_err(|error| format!("エラー: DevTools の応答を解析できません: {error}"))
}

async fn devtools_put(port: u16, path: &str) -> Result<Value, String> {
    let client = devtools_client()
        .ok_or_else(|| "エラー: DevTools への接続準備に失敗しました。".to_string())?;
    let response = client
        .put(format!("http://127.0.0.1:{port}{path}"))
        .send()
        .await
        .map_err(|error| format!("エラー: DevTools への接続に失敗しました: {error}"))?;
    response
        .json::<Value>()
        .await
        .map_err(|error| format!("エラー: DevTools の応答を解析できません: {error}"))
}

fn parse_target(entry: &Value) -> Option<Target> {
    let ws_url = entry.get("webSocketDebuggerUrl")?.as_str()?.to_string();
    Some(Target {
        id: entry.get("id")?.as_str()?.to_string(),
        url: entry
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        ws_url,
    })
}

/// 操作対象のページタブ一覧（`type == "page"` のみ）。
pub async fn pages(port: u16) -> Result<Vec<Target>, String> {
    let list = devtools_get(port, "/json/list").await?;
    let entries = list.as_array().cloned().unwrap_or_default();
    Ok(entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(parse_target)
        .collect())
}

/// 新しいタブを開く。Chrome 111 以降の DevTools HTTP は `/json/new` に PUT を要求する。
pub async fn new_page(port: u16) -> Result<Target, String> {
    let created = devtools_put(port, "/json/new?about:blank").await?;
    parse_target(&created)
        .ok_or_else(|| "エラー: 新しいタブを開けませんでした。".to_string())
}

/// 1つのページへ張った WebSocket 接続。ページ単位の接続なので、
/// `Target.attachToTarget` のセッション管理を持たずに Page / Runtime を直接呼べる。
pub struct PageConn {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
    timeout: Duration,
}

impl PageConn {
    pub async fn connect(ws_url: &str, timeout_secs: u64) -> Result<Self, String> {
        let connect = tokio_tungstenite::connect_async(ws_url);
        let (socket, _) = tokio::time::timeout(Duration::from_secs(timeout_secs), connect)
            .await
            .map_err(|_| "エラー: ブラウザへの接続がタイムアウトしました。".to_string())?
            .map_err(|error| format!("エラー: ブラウザへ接続できません: {error}"))?;
        Ok(PageConn {
            socket,
            next_id: 1,
            timeout: Duration::from_secs(timeout_secs),
        })
    }

    /// CDP コマンドを1つ送り、同じ id の応答が返るまでイベントを読み飛ばす。
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({ "id": id, "method": method, "params": params });
        self.socket
            .send(Message::Text(payload.to_string()))
            .await
            .map_err(|error| format!("エラー: ブラウザへの送信に失敗しました: {error}"))?;

        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(format!("エラー: {method} の応答がタイムアウトしました。"));
            }
            let next = tokio::time::timeout(remaining, self.socket.next())
                .await
                .map_err(|_| format!("エラー: {method} の応答がタイムアウトしました。"))?;
            let message = match next {
                Some(Ok(message)) => message,
                Some(Err(error)) => {
                    return Err(format!("エラー: ブラウザとの通信に失敗しました: {error}"))
                }
                None => return Err("エラー: ブラウザとの接続が切れました。".to_string()),
            };
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => return Err("エラー: ブラウザとの接続が切れました。".to_string()),
                _ => continue,
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            // id を持たないものは CDP イベント。今は使わないので読み飛ばす。
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                let detail = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("詳細不明");
                return Err(format!("エラー: {method} に失敗しました: {detail}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// ページ内で式を評価して値を受け取る。
    pub async fn eval(&mut self, expression: &str) -> Result<Value, String> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                    // ページ側の CSP に関係なく実行できる（script タグ注入ではないため）。
                    "userGesture": true
                }),
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception
                .get("exception")
                .and_then(|value| value.get("description"))
                .and_then(Value::as_str)
                .or_else(|| exception.get("text").and_then(Value::as_str))
                .unwrap_or("詳細不明");
            return Err(format!("エラー: ページ内での実行に失敗しました: {text}"));
        }
        Ok(result
            .get("result")
            .and_then(|value| value.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn navigate(&mut self, url: &str) -> Result<(), String> {
        let result = self.call("Page.navigate", json!({ "url": url })).await?;
        if let Some(error_text) = result.get("errorText").and_then(Value::as_str) {
            return Err(format!("エラー: ページを開けませんでした: {error_text}"));
        }
        Ok(())
    }

    /// 読み込み完了（`document.readyState == "complete"`）まで待つ。
    /// CDP のイベント購読ではなくポーリングにしているのは、イベント順序に依存せず
    /// 遷移途中に接続しても正しく待てるため。
    pub async fn wait_ready(&mut self, timeout_secs: u64) -> Result<(), String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            if let Ok(state) = self.eval("document.readyState").await {
                if state.as_str() == Some("complete") {
                    return Ok(());
                }
            }
            if std::time::Instant::now() >= deadline {
                // 完全に読み終わらなくても、その時点の内容は読める方が実用的。
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_running, new_page, resolve_browser, stop, LaunchOptions, PageConn};

    /// 既定ポートが他の常駐プロセスとぶつかっていないこと。
    /// 重なると起動順によってどちらかが黙って使えなくなり、原因が非常に分かりにくい。
    #[test]
    fn 既定ポートは他の同梱プロセスと重ならない() {
        for (port, owner) in [(11435, "同梱エンジン"), (11436, "同梱 Aivis"), (11500, "ローカル API")] {
            assert_ne!(super::DEFAULT_PORT, port, "{owner} と同じポートを既定にしない");
        }
    }

    /// 実ブラウザを起動して CDP 経由でページを読めることを通しで確かめる。
    /// Chrome / Edge / Chromium が無い環境ではスキップする（CI で落とさないため）。
    #[test]
    fn ブラウザを起動してページ内容を読める() {
        struct BrowserGuard;
        impl Drop for BrowserGuard {
            fn drop(&mut self) {
                let _ = stop();
            }
        }
        let Ok(browser_path) = resolve_browser("") else {
            eprintln!("ブラウザが無いためスキップ");
            return;
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let options = LaunchOptions {
                browser_path,
                // 実運用の既定ポートと衝突しないテスト用ポート。
                port: 11446,
                headless: true,
                use_default_profile: false,
                startup_timeout_secs: 60,
            };
            ensure_running(&options).await.expect("ブラウザを起動できる");
            let _guard = BrowserGuard;
            let target = new_page(options.port).await.expect("タブを開ける");
            let mut conn = PageConn::connect(&target.ws_url, 30)
                .await
                .expect("ページへ接続できる");
            // charset を明示しないと data: URL は latin-1 として解釈され、日本語が化ける。
            conn.navigate("data:text/html;charset=utf-8,<title>ピコ</title><body><p>本文です</p><a href='/next'>次へ</a></body>")
                .await
                .expect("ページを開ける");
            conn.wait_ready(20).await.expect("読み込み完了を待てる");

            let raw = conn
                .eval(super::super::extract::EXTRACT_JS)
                .await
                .expect("抽出スクリプトを実行できる");
            let page = super::super::extract::parse(&raw).expect("抽出結果を解析できる");
            assert_eq!(page.title, "ピコ");
            assert!(page.text.contains("本文です"), "本文: {}", page.text);
            assert!(
                page.elements.iter().any(|element| element.label == "次へ"),
                "リンクが要素一覧に出る"
            );
        });
    }
}
