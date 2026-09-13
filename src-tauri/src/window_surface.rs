// window_surface.rs - メインウィンドウを利用者の目の前へ出す共通手順。
//
// 「表示 → 最小化解除 → 前面化」は順番に意味がある（隠れている・最小化されている
// のどちらでも確実に出す）。ホットキー呼び出しと多重起動時の前面化が同じ手順を
// 使うよう1箇所へ置く。

use tauri::{AppHandle, Manager};

/// メインウィンドウを表示して前面へ出す。各段の失敗は診断ログへ残して続行する
/// （どれか1つが失敗しても、残りで前面化できることがあるため）。
///
/// 呼び出し元はホットキーの呼び出しと多重起動だけで、どちらも利用者が明示的に
/// 「窓を出して」と操作した場合である。起動準備中は窓を隠すが、ここで隠したままにすると
/// 準備が長引いたときも止まったときも呼び戻す手段が無くなるため、準備中は
/// 起動画面（キャラクターを出さない待ち画面）を出して応答する。
///
/// ただしフロントがまだ動き出していない間（webviewの読込中）は、出しても中身の無い
/// 白い窓になるので出さない。その状態が続く場合は `boot_screen` の監視が
/// 文字だけの診断ページへ切り替え、そこで窓が出る。
pub fn surface_main_window(app: &AppHandle) {
    if !crate::boot_screen::is_started(app) && !crate::boot_screen::is_finished(app) {
        crate::log_paths::append_app_log(app, "起動画面の準備前なので前面化を見送ります。");
        return;
    }
    let Some(window) = app.get_webview_window("main") else {
        crate::log_paths::append_app_log(app, "前面化するメインウィンドウがありません。");
        return;
    };
    if !window.is_visible().unwrap_or(true) {
        if let Err(error) = window.show() {
            crate::log_paths::append_app_log(app, &format!("ウィンドウを表示できません: {error}"));
        }
    }
    if let Err(error) = window.unminimize() {
        crate::log_paths::append_app_log(app, &format!("最小化を解除できません: {error}"));
    }
    if let Err(error) = window.set_focus() {
        crate::log_paths::append_app_log(app, &format!("ウィンドウを前面へ出せません: {error}"));
    }
}
