// menu.rs - 右クリックのネイティブコンテキストメニュー。

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager, PhysicalPosition};

use crate::commands::{clear_conversation_and_history, open_settings, AppState};

/// 表示形式メニュー項目の ID 接頭辞。`layout:chat` のように形式 ID を続ける。
const LAYOUT_ID_PREFIX: &str = "layout:";

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
    let reset_layout = MenuItemBuilder::with_id("reset_layout", "表示位置とサイズをリセット")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "PicoAgent を終了")
        .build(&app)
        .map_err(|e| e.to_string())?;

    // 表示形式は作業に合わせてその場で変えるものなので、設定画面を開かずに選べるようにする。
    // 選択中の形式にチェックを付けるため、項目は毎回この場で作り直す。
    let active = {
        let state = app.state::<AppState>();
        crate::layout::current(&state.config)
    };
    let mut layout_menu = SubmenuBuilder::new(&app, "表示形式");
    let mut layout_items = Vec::new();
    for (id, label, _, _) in crate::layout::LAYOUTS {
        layout_items.push(
            CheckMenuItemBuilder::with_id(format!("{LAYOUT_ID_PREFIX}{id}"), label)
                .checked(id == active)
                .build(&app)
                .map_err(|e| e.to_string())?,
        );
    }
    for item in &layout_items {
        layout_menu = layout_menu.item(item);
    }
    let layout_menu = layout_menu.build().map_err(|e| e.to_string())?;

    let menu = MenuBuilder::new(&app)
        .item(&settings)
        .separator()
        .item(&layout_menu)
        .item(&ontop)
        .separator()
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
    if let Some(layout_id) = id.strip_prefix(LAYOUT_ID_PREFIX) {
        if let Err(error) = crate::layout::select(app, layout_id) {
            crate::log_paths::append_app_log(app, &error);
        }
        return;
    }
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

/// フキダシとウィンドウのレイアウト（位置・サイズ）を初期化する。
/// 右クリックメニューと設定画面の「画面位置を初期化」ボタンの両方から呼ぶ。
///
/// **表示形式そのものは変えない。** 「今の見た目のまま位置とサイズだけ戻す」のが
/// この操作の意味で、選び直した形式まで巻き戻すと戻す手段が分かりにくくなる。
/// 寸法は選択中の形式の既定へ戻す（形式ごとに適した大きさが違うため）。
pub(crate) fn reset_bubble_and_window_layout(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let mut patch = serde_json::Map::new();
    for key in [
        "PICOAGENT_BUBBLE_WIDTH_PX",
        "PICOAGENT_BUBBLE_HEIGHT_PX",
        "PICOAGENT_BUBBLE_LEFT_PX",
        "PICOAGENT_BUBBLE_TOP_PX",
        "PICOAGENT_WINDOW_WIDTH_PX",
        "PICOAGENT_WINDOW_HEIGHT_PX",
        // 現在保存している絶対座標。
        "PICOAGENT_WINDOW_LEFT_PX",
        "PICOAGENT_WINDOW_TOP_PX",
        // 旧版が保存していた相対オフセットのキーも掃除する。
        "PICOAGENT_WINDOW_RIGHT_PX",
        "PICOAGENT_WINDOW_BOTTOM_PX",
    ] {
        patch.insert(key.to_string(), serde_json::Value::String(String::new()));
    }
    // 覚えているレイアウト（サイズ・絶対座標）も初期化する。
    *app.state::<crate::WindowLayoutCache>()
        .inner
        .lock()
        .unwrap() = None;
    let active = crate::layout::current(&state.config);
    if let Err(error) = state.config.set_all(patch) {
        crate::log_paths::append_app_log(
            app,
            &format!("表示位置とサイズのリセット保存に失敗しました: {error}"),
        );
    }
    // 手で変えた寸法の記憶も、選択中の形式のぶんだけ捨てる（他の形式の記憶は残す）。
    if let Err(error) = crate::layout::forget_sizes(app, Some(&active)) {
        crate::log_paths::append_app_log(app, &error);
    }

    if let Some(win) = app.get_webview_window("main") {
        // 寸法は選択中の表示形式の既定へ戻す（形式ごとに適した大きさが違う）。
        let (width, height) = {
            let state = app.state::<AppState>();
            crate::layout::size_for(&state.config, &active)
        };
        let _ = win.set_size(tauri::LogicalSize::new(width, height));
        // 起動時のデフォルトと同じ、いま居るモニタの右下へ配置する。
        // 位置計算は適用後の物理サイズを使う（set_size 反映前の outer_size は古い値のため）。
        if let (Ok(Some(monitor)), Ok(scale)) = (win.current_monitor(), win.scale_factor()) {
            let scale = scale.max(1.0);
            let rect = crate::MonitorRect {
                x: monitor.position().x,
                y: monitor.position().y,
                width: monitor.size().width,
                height: monitor.size().height,
            };
            let win_w = (width * scale).round() as u32;
            let win_h = (height * scale).round() as u32;
            let (x, y) = crate::default_window_position(rect, win_w, win_h);
            let _ = win.set_position(PhysicalPosition::new(x, y));
        }
        let _ = win.emit("pet:layout-reset", ());
    }
}
