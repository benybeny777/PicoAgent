// menu.rs - 右クリックのネイティブコンテキストメニュー。

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::{Emitter, LogicalSize, Manager, PhysicalPosition};

use crate::commands::{clear_conversation_and_history, open_settings, AppState};

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
    let reset = MenuItemBuilder::with_id("reset", "会話をリセット")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let reset_layout = MenuItemBuilder::with_id("reset_layout", "フキダシ位置とサイズをリセット")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "PicoAgent を終了")
        .build(&app)
        .map_err(|e| e.to_string())?;

    let menu = MenuBuilder::new(&app)
        .item(&settings)
        .separator()
        .item(&ontop)
        .item(&reset)
        .item(&reset_layout)
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
                        crate::log_paths::append_app_log(
                            app,
                            &format!("常に手前の状態取得に失敗しました: {error}"),
                        );
                        return;
                    }
                };
                let next = !now;
                if let Err(error) = win.set_always_on_top(next) {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("常に手前の切り替えに失敗しました: {error}"),
                    );
                    return;
                }
                let mut patch = serde_json::Map::new();
                patch.insert(
                    "PICOAGENT_ALWAYS_ON_TOP".to_string(),
                    serde_json::Value::Bool(next),
                );
                let state = app.state::<AppState>();
                if let Err(error) = state.config.set_all(patch) {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("常に手前の設定保存に失敗しました: {error}"),
                    );
                }
            }
        }
        "reset" => clear_conversation_and_history(app),
        "reset_layout" => reset_bubble_and_window_layout(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

fn reset_bubble_and_window_layout(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let mut patch = serde_json::Map::new();
    for key in [
        "PICOAGENT_BUBBLE_WIDTH_PX",
        "PICOAGENT_BUBBLE_HEIGHT_PX",
        "PICOAGENT_BUBBLE_LEFT_PX",
        "PICOAGENT_BUBBLE_TOP_PX",
        "PICOAGENT_WINDOW_WIDTH_PX",
        "PICOAGENT_WINDOW_HEIGHT_PX",
        "PICOAGENT_WINDOW_LEFT_PX",
        "PICOAGENT_WINDOW_TOP_PX",
    ] {
        patch.insert(key.to_string(), serde_json::Value::String(String::new()));
    }
    if let Err(error) = state.config.set_all(patch) {
        crate::log_paths::append_app_log(
            app,
            &format!("フキダシ位置とサイズのリセット保存に失敗しました: {error}"),
        );
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_size(LogicalSize::new(560.0, 430.0));
        // 起動時のデフォルトと同じ右下オフセットで配置する（解像度非依存）。
        if let (Ok(Some(monitor)), Ok(size)) = (win.current_monitor(), win.outer_size()) {
            let area = monitor.size();
            let (x, y) =
                crate::default_window_position(area.width, area.height, size.width, size.height);
            let _ = win.set_position(PhysicalPosition::new(x, y));
        }
        let _ = win.emit("pet:layout-reset", ());
    }
}
