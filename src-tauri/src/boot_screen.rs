// boot_screen.rs - フロントエンドを読み込めなかったときに、キャラクター付きの案内画面へ差し替える。
//
// webview（WebView2 / WKWebView）はロードに失敗すると各エンジン固有のエラーページを出す。
// あの画面は HTML として差し替えられないため、「一定時間フロントから準備完了の通知が来ない」
// ことをバックエンド側で検知し、埋め込みの案内ページへ `navigate` し直す。
//
// 案内ページは配信元（dev サーバ / `ui-dist`）が死んでいるときにこそ出る画面なので、
// HTML もキャラクター画像もバイナリへ埋め込み、専用 URI スキームから返す。配信経路には依存しない。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};

/// 案内ページを返す専用 URI スキーム。
pub const SCHEME: &str = "picoboot";

/// フロントから準備完了が来るまでの既定待ち時間（ミリ秒）。
/// 初回起動は webview の初期化とアセット読み込みで数秒かかるため、余裕を持たせる。
const DEFAULT_TIMEOUT_MS: u64 = 12_000;
/// 待ち時間の下限・上限（設定値が極端でも実用範囲へ収める）。
const MIN_TIMEOUT_MS: u64 = 3_000;
const MAX_TIMEOUT_MS: u64 = 120_000;

const PAGE_HTML: &str = include_str!("../assets/boot_screen/page.html");
const CHARACTER_PNG: &[u8] = include_bytes!("../assets/boot_screen/character.png");

/// フロントの準備状況と、案内ページに載せる情報。
#[derive(Default)]
pub struct BootScreenState {
    /// フロントが `pet_frontend_ready` を呼んだか。
    ready: AtomicBool,
    /// 案内ページへ切り替え済みか（多重 navigate を避ける）。
    shown: AtomicBool,
    /// 本来表示するはずだったアプリの URL（「もう一度読み込む」の遷移先）。
    app_url: Mutex<String>,
    /// 検知した状況の説明（案内ページの詳細欄に出す）。
    detail: Mutex<String>,
}

impl BootScreenState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// フロントの準備完了を記録する。以後、案内ページへは切り替えない。
pub fn mark_ready(app: &AppHandle) {
    let state = app.state::<BootScreenState>();
    state.ready.store(true, Ordering::SeqCst);
}

/// 案内ページの URL。Windows / Android は `http://<scheme>.localhost/`、他は `<scheme>://localhost/`。
fn page_url() -> String {
    if cfg!(any(windows, target_os = "android")) {
        format!("http://{SCHEME}.localhost/")
    } else {
        format!("{SCHEME}://localhost/")
    }
}

/// 専用 URI スキームのハンドラ。案内ページ（キャラクター画像を data URI で内包）を返す。
pub fn serve(app: &AppHandle) -> tauri::http::Response<Vec<u8>> {
    let state = app.state::<BootScreenState>();
    let app_url = state.app_url.lock().unwrap().clone();
    let detail = state.detail.lock().unwrap().clone();
    let log_dir = crate::log_paths::log_dir(app).display().to_string();

    let detail_html = format!(
        "<b>状況</b>  {}\n<b>読み込み先</b>  {}\n<b>ログ</b>  {}",
        escape(&detail),
        escape(if app_url.is_empty() {
            "（取得できませんでした）"
        } else {
            &app_url
        }),
        escape(&log_dir),
    );

    // 遷移先を確定できないときに「もう一度読み込む」を出すと、押しても何も起きない
    // 死んだボタンになる。その場合はボタンを出さず、確実にできる操作だけを案内する。
    let actions_html = if app_url.is_empty() {
        "<span class=\"hint\">右クリックメニューから終了して、起動し直してね。</span>".to_string()
    } else {
        format!(
            // 遷移先は属性値なので、`\"` を含むと属性を抜け出せる。必ずエスケープする。
            "<a class=\"btn\" href=\"{}\">もう一度読み込む</a>\
             <span class=\"hint\">直らないときは右クリックメニューから終了して起動し直してね。</span>",
            escape(&app_url)
        )
    };

    let body = PAGE_HTML
        .replace("__CHARACTER__", &character_data_uri())
        .replace("__HEADLINE__", "あれ…？ うまく起きられなかったみたい")
        .replace(
            "__MESSAGE__",
            "画面の中身を読み込めませんでした。少し待ってからもう一度読み込んでみてね。",
        )
        .replace("__DETAIL__", &detail_html)
        .replace("__ACTIONS__", &actions_html);

    tauri::http::Response::builder()
        .status(tauri::http::StatusCode::OK)
        .header(
            tauri::http::header::CONTENT_TYPE,
            "text/html; charset=utf-8",
        )
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .body(body.into_bytes())
        .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

/// 埋め込みキャラクター画像を data URI にする（配信経路に依存せず表示するため）。
fn character_data_uri() -> String {
    use base64::Engine as _;
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(CHARACTER_PNG)
    )
}

/// HTML のテキスト・属性値へ埋めるためのエスケープ。
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// フロントの準備完了を待ち、時間内に来なければ案内ページへ切り替える。
pub fn watch(app: AppHandle) {
    let timeout_ms = {
        let config = &app.state::<crate::commands::AppState>().config;
        config
            .get("PICOAGENT_BOOT_TIMEOUT_MS", "")
            .parse::<u64>()
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
    };

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
        let state = app.state::<BootScreenState>();
        if state.ready.load(Ordering::SeqCst) {
            return;
        }
        // 二重に navigate しない（再読み込み後に再度タイムアウトした場合の保険）。
        if state.shown.swap(true, Ordering::SeqCst) {
            return;
        }
        *state.detail.lock().unwrap() =
            format!("{timeout_ms} ミリ秒以内にフロントエンドの準備完了が届きませんでした");

        let Some(window) = app.get_webview_window("main") else {
            crate::log_paths::append_app_log(
                &app,
                "フロント読み込みの案内画面を出せませんでした（main ウィンドウがありません）。",
            );
            return;
        };

        // 本来の表示先（案内ページの「もう一度読み込む」の遷移先）は**ここで**控える。
        // `setup` の時点では webview がまだ遷移を始めておらず `about:blank` しか返らない。
        if let Ok(url) = window.url() {
            let url = url.to_string();
            if url != "about:blank" {
                *state.app_url.lock().unwrap() = url;
            }
        }
        match tauri::Url::parse(&page_url()) {
            Ok(url) => {
                if let Err(error) = window.navigate(url) {
                    crate::log_paths::append_app_log(
                        &app,
                        &format!("フロント読み込みの案内画面へ切り替えられませんでした: {error}"),
                    );
                    return;
                }
                crate::log_paths::append_app_log(
                    &app,
                    &format!(
                        "フロントエンドを {timeout_ms}ms 以内に読み込めなかったため、案内画面へ切り替えました。"
                    ),
                );
            }
            Err(error) => crate::log_paths::append_app_log(
                &app,
                &format!("案内画面の URL を組み立てられませんでした: {error}"),
            ),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 詳細欄へ埋める文字列はエスケープする() {
        assert_eq!(
            escape(r#"<img src=x onerror="alert(1)">"#),
            "&lt;img src=x onerror=&quot;alert(1)&quot;&gt;"
        );
    }

    #[test]
    fn 案内ページのプレースホルダはすべて置換される() {
        let filled = PAGE_HTML
            .replace("__CHARACTER__", "")
            .replace("__HEADLINE__", "")
            .replace("__MESSAGE__", "")
            .replace("__DETAIL__", "")
            .replace("__ACTIONS__", "");
        assert!(
            !filled.contains("__"),
            "置換されていないプレースホルダが残っています"
        );
    }

    #[test]
    fn 案内ページはインラインスクリプトを持たない() {
        // CSP が script-src 'self' のため、インライン <script> は実行されず操作不能になる。
        assert!(
            !PAGE_HTML.contains("<script"),
            "案内ページは JavaScript なしで完結させてください"
        );
    }
}
