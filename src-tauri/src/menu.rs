// menu.rs - 右クリックの統一メニューと、上端メニューの一覧。
//
// どこで右クリックしても同じメニューを出す。画面は右クリックした場所の文脈（選択文字・入力欄・
// リンク・キャラ）と窓内座標だけを渡し、ここは文脈に応じた項目を先頭に足すだけにする。区切り線より下の
// 共通部分は文脈に依らず同じ並びで、項目は下の定義表から組み立てる（項目の追加・並べ替えは表の1行）。

use serde::Deserialize;
use tauri::menu::{
    CheckMenuItemBuilder, IsMenuItem, MenuBuilder, MenuItemBuilder, MenuItemKind,
    PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{Emitter, LogicalPosition, Manager, PhysicalPosition, Wry};

use crate::commands::{
    clear_conversation_and_history, open_character_settings, open_display_settings, open_settings,
    AppState,
};

/// 表示形式メニュー項目の ID 接頭辞。`layout:pet` のように形式 ID を続ける。
const LAYOUT_ID_PREFIX: &str = "layout:";
const CHARACTER_ID_PREFIX: &str = "character:";
const PLACE_ID_PREFIX: &str = "place:";
/// 文脈項目の ID 接頭辞。選ばれたら `pet:context-action` で画面へ返す。
const CONTEXT_ID_PREFIX: &str = "ctx:";

/// 右クリックした場所の文脈。文字列そのもの（選択文字・URL）は受け取らない。
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct MenuContext {
    /// 選択中の文字がある。
    pub selection: bool,
    /// 入力欄の上で開いた。
    pub editable: bool,
    /// リンクの上で開いた。
    pub link: bool,
    /// キャラの上で開いた。
    pub character: bool,
}

/// メニューの1要素。定義表として `const` に並べ、組み立ては `build_entries` の1か所で行う。
enum Entry {
    /// 通常の項目（ID, 表示名）。
    Item(&'static str, &'static str),
    /// 読み上げの切り替え（表示名が状態で変わるチェック項目）。
    Tts,
    /// 常に手前に表示（チェック項目）。
    OnTop,
    /// 表示形式の一覧（`AVAILABLE_LAYOUTS` から作り、選択中にチェック）。
    Layouts,
    /// 区切り線。
    Separator,
    /// 子を持つ項目（表示名, 子）。
    Submenu(&'static str, &'static [Entry]),
}

/// 文脈ごとに先頭へ足す項目。
struct ContextItem {
    when: fn(&MenuContext) -> bool,
    entry: Entry,
}

const CONTEXT_ITEMS: &[ContextItem] = &[
    // 入力欄では編集操作一式、それ以外で文字を選んでいればコピーだけ出す。
    ContextItem {
        when: |c| c.editable,
        entry: Entry::Item("ctx:cut", "切り取り"),
    },
    ContextItem {
        when: |c| c.editable || c.selection,
        entry: Entry::Item("ctx:copy", "コピー"),
    },
    ContextItem {
        when: |c| c.editable,
        entry: Entry::Item("ctx:paste", "貼り付け"),
    },
    ContextItem {
        when: |c| c.editable,
        entry: Entry::Item("ctx:select-all", "すべて選択"),
    },
    ContextItem {
        when: |c| c.link,
        entry: Entry::Item("ctx:open-link", "リンクを開く"),
    },
    ContextItem {
        when: |c| c.link,
        entry: Entry::Item("ctx:copy-link", "リンクをコピー"),
    },
    ContextItem {
        when: |c| c.character,
        entry: Entry::Item("ctx:edit-character", "キャラの位置とサイズを調整"),
    },
    ContextItem {
        when: |c| c.character,
        entry: Entry::Submenu(
            "構図",
            &[
                Entry::Item("ctx:framing:full", "全身"),
                Entry::Item("ctx:framing:upper", "上半身"),
                Entry::Item("ctx:framing:bust", "胸上"),
            ],
        ),
    },
    ContextItem {
        when: |c| c.character,
        entry: Entry::Item("ctx:reset-character", "キャラを自動配置へ戻す"),
    },
];

/// 文脈に依らず同じ並びの共通部分。会話 → 画面の置き方 → 設定画面 → リセット → 終了の順に区切る。
/// 読み上げの切り替えは会話中によく使う操作なので「話しかける…」と同じまとまりに置く。
/// 会話と記憶の消去は取り消せないので、もう一段めくらないと押せない位置へ置く。
const COMMON_ITEMS: &[Entry] = &[
    Entry::Item("summon", "話しかける…"),
    Entry::Tts,
    Entry::Item("games", "遊ぶ"),
    Entry::Separator,
    Entry::Submenu("画面レイアウト", &[Entry::Layouts]),
    Entry::Submenu(
        "表示するモニター",
        &[
            Entry::Item("place:left", "左画面の中央"),
            Entry::Item("place:right", "右画面の中央"),
            Entry::Item("place:current", "現在の画面の中央"),
            Entry::Item("place:primary", "主画面の中央"),
        ],
    ),
    Entry::Item("edit_layout", "配置を編集（OBS風）"),
    Entry::OnTop,
    Entry::Separator,
    Entry::Item("display_settings", "画面レイアウト・位置…"),
    Entry::Item("character_settings", "キャラクターの設定…"),
    Entry::Item("settings", "設定を開く…"),
    Entry::Separator,
    Entry::Submenu(
        "表示と会話をリセット",
        &[
            Entry::Item("ctx:reset-character", "キャラを自動配置へ戻す"),
            Entry::Item("reset_layout", "今の画面位置・サイズを戻す"),
            Entry::Separator,
            Entry::Submenu(
                "会話履歴と覚えたことを消す",
                &[Entry::Item("reset", "消す（元に戻せません）")],
            ),
        ],
    ),
    Entry::Separator,
    Entry::Item("quit", "終了"),
];

fn err(error: tauri::Error) -> String {
    error.to_string()
}

/// 定義表の要素を実際のメニュー項目へ組み立てる。サブメニューは再帰で組む。
fn build_entries(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    entries: &[&Entry],
) -> Result<Vec<MenuItemKind<Wry>>, String> {
    let mut items = Vec::new();
    for entry in entries {
        match entry {
            Entry::Item(id, label) => items.push(MenuItemKind::MenuItem(
                MenuItemBuilder::with_id(*id, *label)
                    .build(app)
                    .map_err(err)?,
            )),
            Entry::Tts => {
                let enabled = app
                    .state::<AppState>()
                    .config
                    .get_bool("PICOAGENT_TTS_ENABLED", false);
                let label = if enabled {
                    "読み上げ：有効"
                } else {
                    "読み上げ：ミュート中"
                };
                items.push(MenuItemKind::Check(
                    CheckMenuItemBuilder::with_id("tts_toggle", label)
                        .checked(enabled)
                        .build(app)
                        .map_err(err)?,
                ));
            }
            Entry::OnTop => items.push(MenuItemKind::Check(
                CheckMenuItemBuilder::with_id("ontop", "常に手前に表示")
                    .checked(window.is_always_on_top().map_err(err)?)
                    .build(app)
                    .map_err(err)?,
            )),
            Entry::Layouts => items.extend(layout_items(app)?.into_iter().map(MenuItemKind::Check)),
            Entry::Separator => items.push(MenuItemKind::Predefined(
                PredefinedMenuItem::separator(app).map_err(err)?,
            )),
            Entry::Submenu(label, children) => {
                let children: Vec<&Entry> = children.iter().collect();
                let built = build_entries(app, window, &children)?;
                let refs: Vec<&dyn IsMenuItem<Wry>> = built
                    .iter()
                    .map(|item| item as &dyn IsMenuItem<Wry>)
                    .collect();
                items.push(MenuItemKind::Submenu(
                    SubmenuBuilder::new(app, *label)
                        .items(&refs)
                        .build()
                        .map_err(err)?,
                ));
            }
        }
    }
    Ok(items)
}

/// 文脈項目（該当するものだけ）→ 区切り線 → 共通部分、の順に並べる。
fn menu_entries(context: &MenuContext) -> Vec<&'static Entry> {
    let mut entries: Vec<&Entry> = CONTEXT_ITEMS
        .iter()
        .filter(|item| (item.when)(context))
        .map(|item| &item.entry)
        .collect();
    if !entries.is_empty() {
        entries.push(&Entry::Separator);
    }
    entries.extend(COMMON_ITEMS.iter());
    entries
}

/// 右クリックの統一メニュー。キャラ上・会話パネル上・入力欄上・リンク上のどこでもこれを出す。
#[tauri::command]
pub fn pet_menu(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    context: Option<MenuContext>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let entries = menu_entries(&context.unwrap_or_default());
    let items = build_entries(&app, &window, &entries)?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = items
        .iter()
        .map(|item| item as &dyn IsMenuItem<Wry>)
        .collect();
    let menu = MenuBuilder::new(&app).items(&refs).build().map_err(err)?;
    window
        .popup_menu_at(&menu, LogicalPosition::new(x, y))
        .map_err(err)?;
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
    crate::layout::AVAILABLE_LAYOUTS
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
    let edit_layout = MenuItemBuilder::with_id("edit_layout", "配置を編集（OBS風）")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let mut menu = MenuBuilder::new(&app);
    for item in &layout_items {
        menu = menu.item(item);
    }
    let menu = menu
        .separator()
        .item(&edit_layout)
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

/// 文脈項目の ID を画面へ返す中身へ変える（`ctx:framing:bust` → `{ action: "set-framing", framing: "bust" }`）。
fn context_action(id: &str) -> Option<serde_json::Value> {
    let action = id.strip_prefix(CONTEXT_ID_PREFIX)?;
    if let Some(framing) = action.strip_prefix("framing:") {
        return Some(serde_json::json!({ "action": "set-framing", "framing": framing }));
    }
    Some(serde_json::json!({ "action": action }))
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
    if let Some(payload) = context_action(id) {
        // 選択範囲・リンク・キャラの扱いは右クリックした時点の状態を知っている画面側が行う。
        if let Some(win) = app.get_webview_window("main") {
            if let Err(error) = win.emit("pet:context-action", payload) {
                crate::log_paths::append_app_log(
                    app,
                    &format!("右クリックの操作を画面へ渡せません: {error}"),
                );
            }
        }
        return;
    }
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
                if let Err(error) = win.emit("pet:layout-edit", "all") {
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
                crate::notice::emit(
                    app,
                    crate::notice::Notice::info(
                        "error",
                        "会話をリセットできませんでした",
                        &error,
                        false,
                    )
                    .detail("message", serde_json::json!(error)),
                );
            }
        }
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
    // 会話パネルやキャラの配置も、選択中の形式のぶんだけ既定へ戻す（他の形式は残す）。
    let _scene_guard = crate::store::lock_state(crate::scene::CONFIG_KEY);
    let scenes = crate::scene::remove_layout(&crate::scene::load(&state.config), &active);
    patch.insert(crate::scene::CONFIG_KEY.to_string(), scenes.clone());
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
        if let Err(error) = win.emit("pet:layout-reset", serde_json::json!({ "scenes": scenes })) {
            crate::log_paths::append_app_log(
                app,
                &format!("表示リセットを画面へ反映できません: {error}"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{menu_entries, Entry, MenuContext, COMMON_ITEMS};

    fn ids(entries: &[&Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Item(id, _) => id.to_string(),
                Entry::Tts => "tts".into(),
                Entry::OnTop => "ontop".into(),
                Entry::Layouts => "layouts".into(),
                Entry::Separator => "-".into(),
                Entry::Submenu(label, children) => {
                    let children: Vec<&Entry> = children.iter().collect();
                    format!("{label}[{}]", ids(&children).join(","))
                }
            })
            .collect()
    }

    /// 右クリックした場所によって共通部分が変わると、場所ごとに違うメニューに見える。
    /// 文脈項目を足しても、区切り線より下は必ず同じ並びであること。
    #[test]
    fn 共通部分は文脈に依らず同じ並びにする() {
        let common: Vec<&Entry> = COMMON_ITEMS.iter().collect();
        let common = ids(&common);
        for context in [
            MenuContext::default(),
            MenuContext {
                selection: true,
                ..Default::default()
            },
            MenuContext {
                editable: true,
                ..Default::default()
            },
            MenuContext {
                link: true,
                selection: true,
                ..Default::default()
            },
            MenuContext {
                character: true,
                ..Default::default()
            },
        ] {
            let entries = ids(&menu_entries(&context));
            assert!(
                entries.ends_with(&common),
                "共通部分が文脈 {context:?} で変わっています: {entries:?}"
            );
            let added = entries.len() - common.len();
            if added > 0 {
                assert_eq!(entries[added - 1], "-", "文脈項目と共通部分の間は区切る");
            }
        }
    }

    #[test]
    fn 文脈ごとの項目を先頭へ足す() {
        let context = |c: MenuContext| {
            let entries = menu_entries(&c);
            let common = COMMON_ITEMS.len();
            ids(&entries[..entries.len() - common])
        };
        assert!(context(MenuContext::default()).is_empty());
        assert_eq!(
            context(MenuContext {
                selection: true,
                ..Default::default()
            }),
            ["ctx:copy", "-"]
        );
        assert_eq!(
            context(MenuContext {
                editable: true,
                selection: true,
                ..Default::default()
            }),
            ["ctx:cut", "ctx:copy", "ctx:paste", "ctx:select-all", "-"]
        );
        assert_eq!(
            context(MenuContext {
                link: true,
                ..Default::default()
            }),
            ["ctx:open-link", "ctx:copy-link", "-"]
        );
        let character = context(MenuContext {
            character: true,
            ..Default::default()
        });
        assert_eq!(
            character.first().map(String::as_str),
            Some("ctx:edit-character")
        );
        assert!(character
            .iter()
            .any(|id| id.starts_with("構図[ctx:framing:full")));
    }

    /// 読み上げの切り替えを表示系の項目の間へ戻すと、別分野の操作が混ざって探しにくくなる。
    #[test]
    fn 読み上げの切り替えは話しかけるの直後に置く() {
        let common: Vec<&Entry> = COMMON_ITEMS.iter().collect();
        let common = ids(&common);
        assert_eq!(&common[..4], ["summon", "tts", "games", "-"]);
    }

    #[test]
    fn 文脈項目のidを画面の操作名へ変える() {
        assert_eq!(
            super::context_action("ctx:framing:bust"),
            Some(serde_json::json!({ "action": "set-framing", "framing": "bust" }))
        );
        assert_eq!(
            super::context_action("ctx:copy"),
            Some(serde_json::json!({ "action": "copy" }))
        );
        assert_eq!(super::context_action("settings"), None);
    }
}
