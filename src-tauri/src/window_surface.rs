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
pub fn surface_main_window(app: &AppHandle) {
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
