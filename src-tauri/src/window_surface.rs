// window_surface.rs - メインウィンドウを利用者の目の前へ出す共通手順。
//
// 「表示 → 最小化解除 → 前面化」は順番に意味がある（隠れている・最小化されている
// のどちらでも確実に出す）。ホットキー呼び出しと多重起動時の前面化が同じ手順を
// 使うよう1箇所へ置く。

use tauri::{AppHandle, Manager};

/// メインウィンドウを表示して前面へ出す。各段の失敗は診断ログへ残して続行する
/// （どれか1つが失敗しても、残りで前面化できることがあるため）。
pub fn surface_main_window(app: &AppHandle) {
    if !crate::boot_screen::is_finished(app) {
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
