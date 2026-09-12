// boot_screen.rs - フロントエンドを読み込めなかったときに、文字だけの診断画面へ差し替える。
//
// webview（WebView2 / WKWebView）はロードに失敗すると各エンジン固有のエラーページを出す。
// あの画面は HTML として差し替えられないため、「一定時間フロントから準備完了の通知が来ない」
// ことをバックエンド側で検知し、埋め込みの案内ページへ `navigate` し直す。
//
// 案内ページは配信元（dev サーバ / `ui-dist`）が死んでいるときにこそ出る画面なので、
// HTML をバイナリへ埋め込み、専用 URI スキームから返す。配信経路には依存しない。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

/// 案内ページを返す専用 URI スキーム。
pub const SCHEME: &str = "picoboot";

/// フロントの開始通知、または開始後の進捗通知が途絶えてから失敗とする時間（ミリ秒）。
/// 初回はモデルや音声の取得とウイルス検査で長くなるため、2分の無通信だけを停止扱いにする。
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// 読込開始通知（`pet_frontend_ready`）だけを待つ時間（ミリ秒）。
/// webview がページを出せない障害は起動準備の長さと無関係なので、進捗監視と同じだけ待たせない。
const START_TIMEOUT_MS: u64 = 12_000;
/// 待ち時間の下限・上限（設定値が極端でも実用範囲へ収める）。
const MIN_TIMEOUT_MS: u64 = 3_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
/// 生存通知の間隔の下限・上限（詰まりすぎず、しきい値より十分に短く保つ）。
const MIN_PROGRESS_INTERVAL_MS: u64 = 1_000;
const MAX_PROGRESS_INTERVAL_MS: u64 = 10_000;
// 画面を出せない障害の案内を、長い準備の監視と同じだけ待たせない（実用範囲にも収める）。
const _: () = assert!(START_TIMEOUT_MS < DEFAULT_TIMEOUT_MS && START_TIMEOUT_MS >= MIN_TIMEOUT_MS);

const PAGE_HTML: &str = include_str!("../assets/boot_screen/page.html");

/// フロントの準備状況と、案内ページに載せる情報。
#[derive(Default)]
pub struct BootScreenState {
    /// フロントが `pet_frontend_ready` を呼び、JavaScriptの実行を開始したか。
    started: AtomicBool,
    /// フロントが `pet_finish_boot` まで到達し、起動画面を終了したか。
    finished: AtomicBool,
    /// 起動シーケンスの世代。webview を読み込み直すたびに進め、古い監視を終わらせる。
    generation: AtomicU64,
    /// 起動準備中のフロントから最後に生存通知が届いた時刻。
    last_progress: Mutex<Option<Instant>>,
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

/// フロントのJavaScript実行開始を記録する。これだけでは起動完了扱いにしない。
///
/// 設定反映後の再初期化（`restart_app` → `pet:reload`）でも呼ばれる。そのとき窓は
/// `layout::restart_boot` で非表示へ戻るため、**停止監視をここで必ず張り直す**。
/// 張り直さないと、2回目以降の起動が止まったときに案内ページも窓も出ず、
/// 生きているプロセスを終了させる以外に戻す手段が無くなる。
pub fn mark_started(app: &AppHandle) {
    {
        let state = app.state::<BootScreenState>();
        state.started.store(true, Ordering::SeqCst);
        state.finished.store(false, Ordering::SeqCst);
        // 前回の起動で案内ページを出していても、次の起動では改めて検出できるようにする。
        state.shown.store(false, Ordering::SeqCst);
        *state.last_progress.lock().unwrap() = Some(Instant::now());
    }
    watch(app.clone());
}

pub fn is_finished(app: &AppHandle) -> bool {
    app.state::<BootScreenState>()
        .finished
        .load(Ordering::SeqCst)
}

/// フロントが起動準備を継続できていることを記録する。
pub fn mark_progress(app: &AppHandle) {
    let state = app.state::<BootScreenState>();
    *state.last_progress.lock().unwrap() = Some(Instant::now());
}

/// フロントが起動画面を終了したことを記録する。
pub fn mark_finished(app: &AppHandle) {
    let state = app.state::<BootScreenState>();
    state.finished.store(true, Ordering::SeqCst);
}

/// Windows / Android で案内ページを載せるホスト名。
fn page_host() -> String {
    format!("{SCHEME}.localhost")
}

/// 案内ページの URL。Windows / Android は `http://<scheme>.localhost/`、他は `<scheme>://localhost/`。
fn page_url() -> String {
    if cfg!(any(windows, target_os = "android")) {
        format!("http://{}/", page_host())
    } else {
        format!("{SCHEME}://localhost/")
    }
}

/// 読み込み完了したページが案内ページかどうか。プラットフォームで URL の形が変わるため、
/// 判定は `page_url()` と同じ規則をここへ集約する（呼び出し側でホスト名を書かない）。
pub fn is_page_url(scheme: &str, host: Option<&str>) -> bool {
    scheme == SCHEME || host.is_some_and(|host| host == page_host())
}

/// 専用 URI スキームのハンドラ。仮キャラを含まない診断ページを返す。
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

    let log_html = format!("<b>直近のログ</b>{}", escape(&recent_log(app)));

    let body = PAGE_HTML
        .replace("__HEADLINE__", "あれ…？ うまく起きられなかったみたい")
        .replace(
            "__MESSAGE__",
            "画面の中身を読み込めませんでした。少し待ってからもう一度読み込んでみてね。",
        )
        .replace("__DETAIL__", &detail_html)
        .replace("__ACTIONS__", &actions_html)
        .replace("__LOG__", &log_html);

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

/// HTML のテキスト・属性値へ埋めるためのエスケープ。
/// 案内画面へ出すログの行数と読み取り上限。巨大ログでも一定時間で表示する。
const LOG_TAIL_LINES: usize = 12;
const LOG_TAIL_BYTES: u64 = 8 * 1024;

/// 当日の `app.log` の末尾を読む。読めない場合も案内画面自体は必ず出す。
///
/// **この画面は利用者がそのまま撮って送る前提のものなので、APIキー等は診断情報の
/// 書き出しと同じ規則で伏せる。** 設定を読めない状況では伏せられないため、本文を出さない。
fn recent_log(app: &AppHandle) -> String {
    let Some(tail) = crate::commands::redacted_log_tail(app, LOG_TAIL_LINES, LOG_TAIL_BYTES) else {
        return "設定を読み込めないため、ログの内容は表示しません。".to_string();
    };
    if tail.trim().is_empty() {
        return "まだ記録がありません。".to_string();
    }
    tail
}

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

/// 進捗通知が途絶えたと判定するまでの残り時間。`None` は停止扱い。
fn progress_remaining(last_progress: Option<Instant>, timeout: Duration) -> Option<Duration> {
    let elapsed = last_progress?.elapsed();
    timeout.checked_sub(elapsed).filter(|rest| !rest.is_zero())
}

/// フロントの読込開始を待ち、開始後は進捗通知が途絶えた場合だけ案内ページへ切り替える。
/// 進捗が途絶えたと判定するまでの時間（設定値を実用範囲へ収めたもの）。
pub fn timeout_ms(app: &AppHandle) -> u64 {
    let config = &app.state::<crate::commands::AppState>().config;
    config
        .get("PICOAGENT_BOOT_TIMEOUT_MS", "")
        .parse::<u64>()
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
}

/// フロントが生存通知を送る間隔。しきい値から決めるため、設定を短くしても
/// 「正常な起動なのに通知が間に合わず案内画面へ落ちる」が起きない。
/// 4分割にするのは、通知が1回落ちてもすぐには停止扱いにしないため。
pub fn progress_interval_ms(timeout_ms: u64) -> u64 {
    (timeout_ms / 4).clamp(MIN_PROGRESS_INTERVAL_MS, MAX_PROGRESS_INTERVAL_MS)
}

/// 自分より新しい起動シーケンスが始まっていれば真（古い監視は何もせず終わる）。
fn outdated(app: &AppHandle, generation: u64) -> bool {
    app.state::<BootScreenState>()
        .generation
        .load(Ordering::SeqCst)
        != generation
}

pub fn watch(app: AppHandle) {
    let timeout_ms = timeout_ms(&app);

    // 読込開始が来ない障害（配信元の停止・スクリプトの構文エラー）は準備の長さと無関係なので、
    // 案内ページまで2分待たせない。設定で待ち時間を短くした場合はそちらへ合わせる。
    let start_timeout_ms = timeout_ms.min(START_TIMEOUT_MS);
    // 再読込のたびに張り直すため、古い監視が新しい起動を誤って停止扱いにしないよう世代で分ける。
    let generation = app
        .state::<BootScreenState>()
        .generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(start_timeout_ms)).await;
        if outdated(&app, generation) {
            return;
        }
        let started = app
            .state::<BootScreenState>()
            .started
            .load(Ordering::SeqCst);
        if !started {
            show_failure(
                &app,
                start_timeout_ms,
                format!(
                    "{start_timeout_ms} ミリ秒以内にフロントエンドの読込開始通知が届きませんでした"
                ),
            );
            return;
        }
        // モデル・音声の取得は2分を超えることがある。完了までの総時間ではなく、
        // JavaScriptからの生存通知が1区間途絶えた場合だけ停止と判定する。
        let timeout = Duration::from_millis(timeout_ms);
        loop {
            let remaining = {
                let state = app.state::<BootScreenState>();
                if state.generation.load(Ordering::SeqCst) != generation
                    || state.finished.load(Ordering::SeqCst)
                {
                    return;
                }
                // 最後の通知からの経過分を差し引き、停止の検出を1区間ぶん遅らせない。
                let last_progress = *state.last_progress.lock().unwrap();
                progress_remaining(last_progress, timeout)
            };
            let Some(rest) = remaining else { break };
            tokio::time::sleep(rest).await;
        }
        if outdated(&app, generation) {
            return;
        }
        show_failure(
            &app,
            timeout_ms,
            format!("フロントエンドの起動進捗通知が{timeout_ms} ミリ秒以上届きませんでした"),
        );
    });
}

fn show_failure(app: &AppHandle, timeout_ms: u64, detail: String) {
    let state = app.state::<BootScreenState>();
    // 二重に navigate しない（再読み込み後に再度タイムアウトした場合の保険）。
    if state.shown.swap(true, Ordering::SeqCst) {
        return;
    }
    *state.detail.lock().unwrap() = detail;

    let Some(window) = app.get_webview_window("main") else {
        crate::log_paths::append_app_log(
            app,
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
                    app,
                    &format!("フロント読み込みの案内画面へ切り替えられませんでした: {error}"),
                );
                return;
            }
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "フロントエンドの起動が完了しなかったため、案内画面へ切り替えました（監視区間 {timeout_ms}ms）。"
                ),
            );
        }
        Err(error) => crate::log_paths::append_app_log(
            app,
            &format!("案内画面の URL を組み立てられませんでした: {error}"),
        ),
    }
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
            .replace("__ACTIONS__", "")
            .replace("__LOG__", "");
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

    #[test]
    fn 読込開始だけでは起動完了扱いにしない() {
        let state = BootScreenState::new();
        state.started.store(true, Ordering::SeqCst);
        assert!(state.started.load(Ordering::SeqCst));
        assert!(
            !state.finished.load(Ordering::SeqCst),
            "JavaScript開始後の例外も監視するため、完了通知は別に必要"
        );
    }

    #[test]
    fn 生存通知の間隔はしきい値から決める() {
        // 既定（2分）はこれまでと同じ10秒。
        assert_eq!(progress_interval_ms(DEFAULT_TIMEOUT_MS), 10_000);
        // しきい値を縮めても通知が間に合う（1区間に4回送れる）。
        assert_eq!(progress_interval_ms(MIN_TIMEOUT_MS), 1_000);
        assert_eq!(progress_interval_ms(20_000), 5_000);
        // 上限・下限を超えない。
        assert_eq!(progress_interval_ms(MAX_TIMEOUT_MS), 10_000);
        assert_eq!(progress_interval_ms(0), MIN_PROGRESS_INTERVAL_MS);
        for timeout in [MIN_TIMEOUT_MS, 20_000, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS] {
            assert!(
                progress_interval_ms(timeout) < timeout,
                "通知間隔がしきい値以上だと正常な起動でも失敗になる: {timeout}"
            );
        }
    }

    #[test]
    fn 案内ページの判定はプラットフォームごとの形式を取りこぼさない() {
        // 非Windowsの `picoboot://localhost/`。
        assert!(is_page_url(SCHEME, Some("localhost")));
        // Windows / Android の `http://picoboot.localhost/`。
        assert!(is_page_url("http", Some(&page_host())));
        // 通常のアプリ画面は対象外（ここで真になると起動準備中の窓を出してしまう）。
        assert!(!is_page_url("http", Some("tauri.localhost")));
        assert!(!is_page_url("http", Some("localhost")));
        assert!(!is_page_url("https", None));
    }

    #[test]
    fn 進捗が続く間は長い起動準備を失敗にしない() {
        let now = Instant::now();
        let timeout = Duration::from_secs(120);
        assert!(progress_remaining(Some(now), timeout).is_some());
        assert!(progress_remaining(None, timeout).is_none());
        assert!(progress_remaining(Some(now - Duration::from_secs(121)), timeout).is_none());
        // 最後の通知からの経過分だけを待ち直す（1区間ぶん余計に待たない）。
        let rest = progress_remaining(Some(now - Duration::from_secs(100)), timeout).unwrap();
        assert!(rest <= Duration::from_secs(20));
    }
}
