// menu.rs - 右クリックのネイティブコンテキストメニュー。

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager, PhysicalPosition};

use crate::commands::{
    clear_conversation_and_history, open_character_settings, open_display_settings, open_settings,
    AppState,
};

/// 表示形式メニュー項目の ID 接頭辞。`layout:chat` のように形式 ID を続ける。
const LAYOUT_ID_PREFIX: &str = "layout:";
const PLACE_ID_PREFIX: &str = "place:";

#[tauri::command]
pub fn pet_menu(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    let summon = MenuItemBuilder::with_id("summon", "話しかける…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let settings = MenuItemBuilder::with_id("settings", "すべての設定…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let display_settings = MenuItemBuilder::with_id("display_settings", "画面・操作の設定…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let character_settings =
        MenuItemBuilder::with_id("character_settings", "キャラクター・音声の設定…")
            .build(&app)
            .map_err(|e| e.to_string())?;
    let is_always_on_top = window.is_always_on_top().map_err(|e| e.to_string())?;
    let ontop = CheckMenuItemBuilder::with_id("ontop", "常に手前に表示")
        .checked(is_always_on_top)
        .build(&app)
        .map_err(|e| e.to_string())?;
    let tts_enabled = {
        let state = app.state::<AppState>();
        state.config.get_bool("PICOAGENT_TTS_ENABLED", false)
    };
    let tts = CheckMenuItemBuilder::with_id(
        "tts_toggle",
        if tts_enabled {
            "読み上げ：有効"
        } else {
            "読み上げ：ミュート中"
        },
    )
    .checked(tts_enabled)
    .build(&app)
    .map_err(|e| e.to_string())?;
    let reset = MenuItemBuilder::with_id("reset", "会話履歴を消してリセット")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let reset_layout = MenuItemBuilder::with_id("reset_layout", "今の画面位置・サイズを戻す")
        .build(&app)
        .map_err(|e| e.to_string())?;
    // 立ち絵をドラッグでずらしすぎたときの戻し先。位置とサイズのリセットにも含まれるが、
    // 「キャラを動かしたら戻せなくなった」が一番起きやすい操作ミスなので単独で出す。
    let reset_character = MenuItemBuilder::with_id("reset_character", "キャラ位置を中央へ戻す")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "終了")
        .build(&app)
        .map_err(|e| e.to_string())?;

    // 表示形式は作業に合わせてその場で変えるものなので、設定画面を開かずに選べるようにする。
    // 選択中の形式にチェックを付けるため、項目は毎回この場で作り直す。
    let active = {
        let state = app.state::<AppState>();
        crate::layout::current(&state.config)
    };
    let mut layout_menu = SubmenuBuilder::new(&app, "表示スタイル");
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
    let placement_menu = SubmenuBuilder::new(&app, "ウィンドウを置く画面")
        .item(
            &MenuItemBuilder::with_id("place:left", "左画面の中央")
                .build(&app)
                .map_err(|e| e.to_string())?,
        )
        .item(
            &MenuItemBuilder::with_id("place:right", "右画面の中央")
                .build(&app)
                .map_err(|e| e.to_string())?,
        )
        .item(
            &MenuItemBuilder::with_id("place:current", "現在の画面の中央")
                .build(&app)
                .map_err(|e| e.to_string())?,
        )
        .item(
            &MenuItemBuilder::with_id("place:primary", "主画面の中央")
                .build(&app)
                .map_err(|e| e.to_string())?,
        )
        .build()
        .map_err(|e| e.to_string())?;

    // 取り消しにくい会話消去を通常操作から離し、表示を整える操作と同じサブメニューへまとめる。
    let reset_menu = SubmenuBuilder::new(&app, "表示と会話をリセット")
        .item(&reset_character)
        .item(&reset_layout)
        .separator()
        .item(&reset)
        .build()
        .map_err(|e| e.to_string())?;

    let menu = MenuBuilder::new(&app)
        .item(&summon)
        .separator()
        .item(&layout_menu)
        .item(&placement_menu)
        .item(&ontop)
        .item(&tts)
        .separator()
        .item(&display_settings)
        .item(&character_settings)
        .item(&settings)
        .separator()
        .item(&reset_menu)
        .separator()
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
    if let Some(position) = id.strip_prefix(PLACE_ID_PREFIX) {
        if let Err(error) = crate::commands::pet_place_window(position.to_string(), app.clone()) {
            crate::log_paths::append_app_log(app, &error);
        }
        return;
    }
    match id {
        "summon" => {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
                let _ = win.emit("pet:hotkey", "summon");
            }
        }
        "settings" => open_settings(app),
        "display_settings" => open_display_settings(app),
        "character_settings" => open_character_settings(app),
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
        "tts_toggle" => {
            let state = app.state::<AppState>();
            let next = !state.config.get_bool("PICOAGENT_TTS_ENABLED", false);
            let mut patch = serde_json::Map::new();
            patch.insert(
                "PICOAGENT_TTS_ENABLED".to_string(),
                serde_json::Value::Bool(next),
            );
            if let Err(error) = state.config.set_all(patch) {
                crate::log_paths::append_app_log(
                    app,
                    &format!("読み上げ設定の保存に失敗しました: {error}"),
                );
                return;
            }
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.emit("pet:ui", serde_json::json!({ "ttsEnabled": next }));
            }
        }
        "reset" => clear_conversation_and_history(app),
        "reset_layout" => reset_bubble_and_window_layout(app),
        "reset_character" => reset_character_offset(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// 現在のキャラ・表示形式の構図と互換用移動量を既定へ戻す。
/// ウィンドウの位置・サイズ・表示形式には触らない。
pub(crate) fn reset_character_offset(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let mut patch = serde_json::Map::new();
    for key in [
        "PICOAGENT_CHARACTER_OFFSET_X",
        "PICOAGENT_CHARACTER_OFFSET_Y",
    ] {
        patch.insert(key.to_string(), serde_json::Value::String(String::new()));
    }
    let character_id = state.config.get("PICOAGENT_CHARACTER", "mugi");
    let layout = crate::layout::current(&state.config);
    let _framing_guard = crate::store::lock_state("PICOAGENT_CHARACTER_FRAMING");
    let framing = state
        .config
        .get_all()
        .get("PICOAGENT_CHARACTER_FRAMING")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    patch.insert(
        "PICOAGENT_CHARACTER_FRAMING".to_string(),
        crate::character_framing::remove(&framing, &character_id, &layout),
    );
    if let Err(error) = state.config.set_all(patch) {
        crate::log_paths::append_app_log(
            app,
            &format!("立ち絵の位置のリセット保存に失敗しました: {error}"),
        );
        return;
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.emit(
            "pet:character-reset",
            serde_json::json!({ "characterId": character_id, "layout": layout }),
        );
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
        // ドラッグでずらした立ち絵の位置も既定（上半身が出る位置）へ戻す。
        "PICOAGENT_CHARACTER_OFFSET_X",
        "PICOAGENT_CHARACTER_OFFSET_Y",
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
