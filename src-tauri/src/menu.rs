// menu.rs - 右クリックのネイティブコンテキストメニュー。
// 「設定…」「常に手前に表示」「終了」だけを表示する。

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::Manager;

use crate::commands::{open_settings, AppState};

#[tauri::command]
pub fn pet_menu(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    let settings = MenuItemBuilder::with_id("settings", "設定…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let is_always_on_top = window.is_always_on_top().map_err(|e| e.to_string())?;
    let ontop = CheckMenuItemBuilder::with_id("ontop", "常に手前に表示")
        .checked(is_always_on_top)
        .build(&app)
        .map_err(|e| e.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "PicoAgent を終了")
        .build(&app)
        .map_err(|e| e.to_string())?;

    let menu = MenuBuilder::new(&app)
        .item(&settings)
        .separator()
        .item(&ontop)
        .item(&quit)
        .build()
        .map_err(|e| e.to_string())?;

    window.popup_menu(&menu).map_err(|e| e.to_string())?;
    Ok(())
}

/// メニュー選択イベントの処理（main.rs の on_menu_event から呼ぶ）。
pub fn handle_event(app: &tauri::AppHandle, id: &str) {
    match id {
        "settings" => open_settings(app),
        "ontop" => {
            if let Some(win) = app.get_webview_window("main") {
                let now = match win.is_always_on_top() {
                    Ok(now) => now,
                    Err(error) => {
                        eprintln!("常に手前の状態取得に失敗しました: {error}");
                        return;
                    }
                };
                let next = !now;
                if let Err(error) = win.set_always_on_top(next) {
                    eprintln!("常に手前の切り替えに失敗しました: {error}");
                    return;
                }
                let mut patch = serde_json::Map::new();
                patch.insert(
                    "CODA_ALWAYS_ON_TOP".to_string(),
                    serde_json::Value::Bool(next),
                );
                let state = app.state::<AppState>();
                if let Err(error) = state.config.set_all(patch) {
                    eprintln!("常に手前の設定保存に失敗しました: {error}");
                }
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}
