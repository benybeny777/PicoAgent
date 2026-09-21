// menu.rs - 右クリックのネイティブコンテキストメニュー。

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, LogicalPosition, Manager, PhysicalPosition};

use crate::commands::{
    clear_conversation_and_history, open_character_settings, open_display_settings, open_settings,
    AppState,
};

/// 表示形式メニュー項目の ID 接頭辞。`layout:chat` のように形式 ID を続ける。
const LAYOUT_ID_PREFIX: &str = "layout:";
const CHARACTER_ID_PREFIX: &str = "character:";
const PLACE_ID_PREFIX: &str = "place:";

#[tauri::command]
pub fn pet_menu(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    let summon = MenuItemBuilder::with_id("summon", "話しかける…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let settings = MenuItemBuilder::with_id("settings", "設定を開く…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let games = MenuItemBuilder::with_id("games", "遊ぶ")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let display_settings = MenuItemBuilder::with_id("display_settings", "画面レイアウト・位置…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let edit_layout = MenuItemBuilder::with_id("edit_layout", "配置を直接調整")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let character_settings = MenuItemBuilder::with_id("character_settings", "キャラクターの設定…")
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
    // 会話履歴だけでなく**これまで覚えたこと（長期メモリ）も消える**。以前のラベルは
    // 「会話履歴を消して」としか言っておらず、失うものを実際より小さく見せていた。
    let reset = MenuItemBuilder::with_id("reset", "消す（元に戻せません）")
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
    let layout_items = layout_items(&app)?;
    let mut layout_menu = SubmenuBuilder::new(&app, "画面レイアウト");
    for item in &layout_items {
        layout_menu = layout_menu.item(item);
    }
    let layout_menu = layout_menu.build().map_err(|e| e.to_string())?;
    let placement_menu = SubmenuBuilder::new(&app, "表示するモニター")
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

    // 会話と記憶の消去は取り消せないので、もう一段めくらないと押せない位置へ置く。
    // 同じ階層に並べていた頃は、キャラ位置を直しに来た利用者の目に、見た目が同じ3項目
    // として並んでいた（2つは無害、1つは全消去）。
    let reset_conversation_menu = SubmenuBuilder::new(&app, "会話履歴と覚えたことを消す")
        .item(&reset)
        .build()
        .map_err(|e| e.to_string())?;
    let reset_menu = SubmenuBuilder::new(&app, "表示と会話をリセット")
        .item(&reset_character)
        .item(&reset_layout)
        .separator()
        .item(&reset_conversation_menu)
        .build()
        .map_err(|e| e.to_string())?;

    // 会話 → 画面の置き方 → 設定画面 → リセット → 終了の順に、分野ごとに区切る。
    // 読み上げの切り替えは会話中によく使う操作なので「話しかける…」と同じまとまりに置く
    // （以前は画面レイアウトや常に手前表示の間に混ざっていた）。
    let menu = MenuBuilder::new(&app)
        .item(&summon)
        .item(&tts)
        .item(&games)
        .separator()
        .item(&layout_menu)
        .item(&placement_menu)
        .item(&edit_layout)
        .item(&ontop)
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

/// 画面レイアウトの選択肢。選択中の形式にチェックを付けるため、メニューを開くたびに作り直す。
/// 右クリックの「画面レイアウト」と上端メニューの「表示」で共有する（片方だけ直すと項目やチェックがずれる）。
fn layout_items(
    app: &tauri::AppHandle,
) -> Result<Vec<tauri::menu::CheckMenuItem<tauri::Wry>>, String> {
    let active = {
        let state = app.state::<AppState>();
        crate::layout::current(&state.config)
    };
    crate::layout::LAYOUTS
        .into_iter()
        .map(|(id, label, _, _)| {
            CheckMenuItemBuilder::with_id(format!("{LAYOUT_ID_PREFIX}{id}"), label)
                .checked(id == active)
                .build(app)
                .map_err(|e| e.to_string())
        })
        .collect()
}

/// 上端メニューの「表示」から、画面レイアウトの一覧をボタン直下へ出す。
/// 設定画面を開いてカードを選び保存するより手数が少ない。一覧の最後から従来の設定画面へも進める。
/// `x` / `y` はウィンドウ左上を原点とする論理座標（メイン画面はOS枠を持たないのでWebViewの座標と一致する）。
#[tauri::command]
pub fn pet_layout_menu(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let layout_items = layout_items(&app)?;
    let display_settings = MenuItemBuilder::with_id("display_settings", "画面レイアウト・位置…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let mut menu = MenuBuilder::new(&app);
    for item in &layout_items {
        menu = menu.item(item);
    }
    let menu = menu
        .separator()
        .item(&display_settings)
        .build()
        .map_err(|e| e.to_string())?;
    window
        .popup_menu_at(&menu, LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 上端メニューの「キャラ選択」から、キャラクターの一覧をボタン直下へ出す。
///
/// 以前は設定画面のキャラ選択タブを開くだけで、隣の「表示」がその場でメニューを出すのと
/// 挙動が揃っておらず、切り替えるだけなのに設定画面まで往復する必要があった。
/// 一覧は同梱キャラと追加キャラを突き合わせたフロント側の一覧をそのまま受け取る
/// （突き合わせを二重に実装すると、片方だけ増減したときに食い違う）。
#[tauri::command]
pub fn pet_character_menu(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    x: f64,
    y: f64,
    characters: Vec<serde_json::Value>,
) -> Result<(), String> {
    let active = {
        let state = app.state::<AppState>();
        state.config.get("PICOAGENT_CHARACTER", "mugi")
    };
    let items: Vec<_> = characters
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id").and_then(serde_json::Value::as_str)?;
            let name = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(id);
            CheckMenuItemBuilder::with_id(format!("{CHARACTER_ID_PREFIX}{id}"), name)
                .checked(id == active)
                .build(&app)
                .ok()
        })
        .collect();
    let settings = MenuItemBuilder::with_id("character_settings", "キャラクターの追加・設定…")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let mut menu = MenuBuilder::new(&app);
    for item in &items {
        menu = menu.item(item);
    }
    let menu = menu
        .separator()
        .item(&settings)
        .build()
        .map_err(|e| e.to_string())?;
    window
        .popup_menu_at(&menu, LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// メニューで選ばれたキャラクターへ切り替える。
/// 設定画面での保存と同じ経路（設定の保存 → 再初期化）を通すため、人格・背景・音声・
/// 履歴の切り替えが設定画面から変えた場合と一致する。エンジンとAivisは動かしたままにする。
fn select_character(app: &tauri::AppHandle, character_id: &str) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut patch = serde_json::Map::new();
        patch.insert(
            "PICOAGENT_CHARACTER".to_string(),
            serde_json::json!(character_id),
        );
        state
            .config
            .set_all(patch)
            .map_err(|error| format!("キャラクターの保存に失敗しました: {error}"))?;
    }
    crate::commands::restart_app(app.clone(), app.state::<AppState>(), false, false)?;
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
    if let Some(character_id) = id.strip_prefix(CHARACTER_ID_PREFIX) {
        if let Err(error) = select_character(app, character_id) {
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
                if let Err(error) = win.show() {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("メニューから再表示できません: {error}"),
                    );
                }
                if let Err(error) = win.set_focus() {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("メニューから前面化できません: {error}"),
                    );
                }
                if let Err(error) = win.emit("pet:hotkey", "summon") {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("メニューから入力欄を開けません: {error}"),
                    );
                }
            }
        }
        "settings" => open_settings(app),
        "games" => {
            if let Some(win) = app.get_webview_window("main") {
                if let Err(error) = win.emit("pet:game-open", ()) {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("ゲーム選択画面を開けません: {error}"),
                    );
                }
            }
        }
        "display_settings" => open_display_settings(app),
        "character_settings" => open_character_settings(app),
        "edit_layout" => {
            if let Some(win) = app.get_webview_window("main") {
                if let Err(error) = win.emit("pet:layout-edit", "character") {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("配置編集を画面へ開始できません: {error}"),
                    );
                }
            }
        }
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
                // 設定ウィンドウはメインと同じ最前面状態で開く（`open_settings_tab`）。
                // ここで揃えないと、開いている間にメインを通常表示へ戻しても設定側だけが
                // 最前面のまま居座り、閉じるまで下ろせなくなる。
                if let Some(settings) = app.get_webview_window("settings") {
                    if let Err(error) = settings.set_always_on_top(next) {
                        crate::log_paths::append_app_log(
                            app,
                            &format!("設定画面の常に手前を揃えられません: {error}"),
                        );
                    }
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
                if let Err(error) = win.emit("pet:ui", serde_json::json!({ "ttsEnabled": next })) {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("読み上げ表示を更新できません: {error}"),
                    );
                }
            }
        }
        "reset" => {
            if let Err(error) = clear_conversation_and_history(app) {
                crate::log_paths::append_app_log(
                    app,
                    &format!("会話のリセットに失敗しました: {error}"),
                );
                if let Some(win) = app.get_webview_window("main") {
                    if let Err(notice_error) = win.emit(
                        "pet:notice",
                        serde_json::json!({
                            "kind": "error",
                            "title": "会話をリセットできませんでした",
                            "message": error,
                        }),
                    ) {
                        crate::log_paths::append_app_log(
                            app,
                            &format!("会話リセット失敗の画面通知も送れません: {notice_error}"),
                        );
                    }
                }
            }
        }
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
    let stored_framing = state
        .config
        .get_all()
        .get("PICOAGENT_CHARACTER_FRAMING")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let framing = match crate::character_framing::normalize_checked(&stored_framing) {
        Ok(framing) => framing,
        Err(error) => {
            crate::log_paths::append_app_log(app, &error);
            return;
        }
    };
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
        if let Err(error) = win.emit(
            "pet:character-reset",
            serde_json::json!({ "characterId": character_id, "layout": layout }),
        ) {
            crate::log_paths::append_app_log(
                app,
                &format!("立ち絵リセットを画面へ反映できません: {error}"),
            );
        }
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
        if let Err(error) = win.set_size(tauri::LogicalSize::new(width, height)) {
            crate::log_paths::append_app_log(
                app,
                &format!("表示リセットでウィンドウサイズを戻せませんでした: {error}"),
            );
        }
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
            if let Err(error) = win.set_position(PhysicalPosition::new(x, y)) {
                crate::log_paths::append_app_log(
                    app,
                    &format!("表示リセットでウィンドウ位置を戻せませんでした: {error}"),
                );
            }
        }
        if let Err(error) = win.emit("pet:layout-reset", ()) {
            crate::log_paths::append_app_log(
                app,
                &format!("表示リセットを画面へ反映できません: {error}"),
            );
        }
    }
}

#[cfg(test)]
mod order_guard {
    /// 読み上げの切り替えを表示系の項目の間へ戻すと、別分野の操作が混ざって探しにくくなる。
    #[test]
    fn 読み上げの切り替えは話しかけるの直後に置く() {
        let source = include_str!("menu.rs");
        let builder = source
            .split("let menu = MenuBuilder::new(&app)")
            .nth(1)
            .expect("右クリックメニューの組み立てが見つからない");
        let position = |needle: &str| {
            builder
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} が見つからない"))
        };
        assert!(position(".item(&summon)") < position(".item(&tts)"));
        assert!(position(".item(&tts)") < position(".separator()"));
        assert!(position(".separator()") < position(".item(&layout_menu)"));
    }
}
