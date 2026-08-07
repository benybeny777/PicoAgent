// layout.rs - 表示形式（画面構成）の切り替え。
//
// 1つの透明ウィンドウで「待機」「会話」「長い作業」を全部引き受けると、待機中でも
// フキダシが最大サイズで居座り、履歴を見るには全画面にするしかない。用途ごとに
// 見た目だけを差し替えられるようにして、キャラクター描画・履歴・通知・音声・許可は
// すべて共通のまま使う。
//
// フロントは `<main id="petApp" data-layout="…">` を見て CSS で配置を変えるだけ。
// バックエンド側の仕事は「どれが選ばれているか」と「ウィンドウの寸法」の2つに絞る。

use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, LogicalSize, Manager};

use crate::commands::AppState;

/// 既定の表示形式。未知の値が保存されていた場合もここへ戻す
/// （設定ファイルが壊れてもキャラクターの居ない画面にはしない）。
pub const DEFAULT: &str = "pet";

/// 表示形式の定義: (ID, 表示名, 既定の幅, 既定の高さ)。
///
/// 寸法は「その形式が成立する最小限」を既定にする。ユーザーが手で変えた寸法は
/// 形式ごとに覚えるので（`PICOAGENT_WINDOW_LAYOUTS`）、ここは初回の見え方を決める値。
pub const LAYOUTS: [(&str, &str, f64, f64); 8] = [
    ("mascot", "マスコット（キャラだけ）", 240.0, 380.0),
    ("pet", "ペット（キャラ＋フキダシ）", 680.0, 430.0),
    ("chat", "チャット（履歴つきパネル）", 380.0, 640.0),
    ("talk", "トーク（メッセージアプリ風）", 380.0, 640.0),
    ("bar", "バー（1行入力）", 640.0, 300.0),
    ("workspace", "ワークスペース（3ペイン）", 1100.0, 700.0),
    ("stream", "配信（キャラを大きく）", 780.0, 460.0),
    ("widget", "ウィジェット（今日の情報）", 320.0, 440.0),
];

/// ウィンドウの最小寸法。`tauri.conf.json` の `minWidth` / `minHeight` と揃える必要がある
/// （小さい形式ほどここで頭打ちになるため、設定値より小さい既定は持たせない）。
pub const MIN_WIDTH: f64 = 200.0;
pub const MIN_HEIGHT: f64 = 240.0;
/// 起動準備中だけ使う共通VTuber画面の寸法。保存中の表示形式には影響させない。
pub const BOOT_WIDTH: f64 = 680.0;
pub const BOOT_HEIGHT: f64 = 430.0;

/// 形式ごとのウィンドウ寸法を覚えておく設定キー。
/// `{"chat":{"w":380,"h":640},…}` の形。位置は形式をまたいで共有する
/// （置き場所はユーザーの習慣で、形式ごとに変えたいものではないため）。
pub(crate) const SIZES_KEY: &str = "PICOAGENT_WINDOW_LAYOUTS";

/// 指定 ID が定義済みかどうか。
pub fn is_known(id: &str) -> bool {
    LAYOUTS.iter().any(|(layout, _, _, _)| *layout == id)
}

/// 選択中の表示形式。未知の値は既定へ丸める。
pub fn current(config: &crate::config::Config) -> String {
    let id = config.get("PICOAGENT_LAYOUT", DEFAULT);
    if is_known(&id) {
        id
    } else {
        DEFAULT.to_string()
    }
}

/// 表示形式の既定寸法。
fn default_size(id: &str) -> (f64, f64) {
    LAYOUTS
        .iter()
        .find(|(layout, _, _, _)| *layout == id)
        .map(|(_, _, width, height)| (*width, *height))
        .unwrap_or((680.0, 430.0))
}

/// 形式ごとに覚えている寸法。無ければ既定寸法。
pub fn size_for(config: &crate::config::Config, id: &str) -> (f64, f64) {
    let (default_width, default_height) = default_size(id);
    let saved = config
        .get_all()
        .get(SIZES_KEY)
        .and_then(|value| value.get(id).cloned());
    let Some(saved) = saved else {
        return (default_width, default_height);
    };
    let width = saved
        .get("w")
        .and_then(Value::as_f64)
        .unwrap_or(default_width);
    let height = saved
        .get("h")
        .and_then(Value::as_f64)
        .unwrap_or(default_height);
    // 560×430 は旧既定、600×430 は口パク実機確認中に一度だけ使った暫定既定。
    // ユーザーが明示的に広げた寸法は残しつつ、既定のまま使っていた環境だけ新しい
    // 680×430へ移す。これが無いと形式切替時に記録された旧既定が永久に優先される。
    if id == "pet" && height == 430.0 && matches!(width as u32, 560 | 600) {
        return (default_width, default_height);
    }
    (width.max(MIN_WIDTH), height.max(MIN_HEIGHT))
}

/// 形式ごとの寸法を覚える（ユーザーがウィンドウを手で変えたとき）。
pub fn remember_size(app: &AppHandle, id: &str, width: f64, height: f64) -> Result<(), String> {
    if !is_known(id) {
        return Ok(());
    }
    // 設定画面保存と右クリック切替が重なっても、別形式の寸法を消し合わない。
    let _guard = crate::store::lock_state(SIZES_KEY);
    let state = app.state::<AppState>();
    let mut sizes = match state.config.get_all().get(SIZES_KEY) {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    sizes.insert(
        id.to_string(),
        json!({ "w": width.round(), "h": height.round() }),
    );
    let mut patch = Map::new();
    patch.insert(SIZES_KEY.to_string(), Value::Object(sizes));
    state
        .config
        .set_all(patch)
        .map_err(|error| format!("表示形式ごとのウィンドウ寸法の保存に失敗しました: {error}"))
}

/// 現在の表示形式とウィンドウ寸法を、切り替え前の値として覚える。
/// 右クリックメニューと設定画面のどちらもこの共通経路を使う。
pub fn remember_current_size(app: &AppHandle) -> Result<(), String> {
    let previous = {
        let state = app.state::<AppState>();
        current(&state.config)
    };
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let (Ok(size), Ok(scale)) = (window.outer_size(), window.scale_factor()) else {
        return Ok(());
    };
    let scale = scale.max(1.0);
    remember_size(
        app,
        &previous,
        size.width as f64 / scale,
        size.height as f64 / scale,
    )
}

/// 覚えている寸法を捨てて既定へ戻す。`id` が `None` なら全形式。
pub fn forget_sizes(app: &AppHandle, id: Option<&str>) -> Result<(), String> {
    // リサイズ記憶の保存とリセットが並列になっても、古い一覧を戻さない。
    let _guard = crate::store::lock_state(SIZES_KEY);
    let state = app.state::<AppState>();
    let sizes = match (id, state.config.get_all().get(SIZES_KEY)) {
        // 特定の形式だけ忘れる場合は、他の形式の記憶を残す。
        (Some(id), Some(Value::Object(map))) => {
            let mut map = map.clone();
            map.remove(id);
            Value::Object(map)
        }
        _ => Value::Object(Map::new()),
    };
    let mut patch = Map::new();
    patch.insert(SIZES_KEY.to_string(), sizes);
    state
        .config
        .set_all(patch)
        .map_err(|error| format!("表示形式ごとのウィンドウ寸法の初期化に失敗しました: {error}"))
}

/// 選択中の表示形式に合わせてウィンドウ寸法を当て、フロントへ通知する。
/// 起動時と切り替え時の両方から呼ぶ。
pub fn apply(app: &AppHandle, id: &str) -> Result<(), String> {
    let id = if is_known(id) { id } else { DEFAULT };
    let (width, height) = {
        let state = app.state::<AppState>();
        size_for(&state.config, id)
    };

    apply_window_size(app, width, height, Some(id))
}

/// 保存中の表示形式にかかわらず、起動準備中は共通のVTuber画面へ広げる。
pub fn apply_boot(app: &AppHandle) -> Result<(), String> {
    apply_window_size(app, BOOT_WIDTH, BOOT_HEIGHT, None)
}

fn apply_window_size(
    app: &AppHandle,
    width: f64,
    height: f64,
    emit_layout: Option<&str>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        // 画面より大きな寸法は要求しない（ワークスペースのように大きい形式で、
        // 小さいディスプレイでもつまみに手が届かなくならないように）。
        let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
        let (width, height) = match window.current_monitor() {
            Ok(Some(monitor)) => (
                width.min(monitor.size().width as f64 / scale),
                height.min(monitor.size().height as f64 / scale),
            ),
            _ => (width, height),
        };
        let (width, height) = (width.max(MIN_WIDTH), height.max(MIN_HEIGHT));
        window
            .set_size(LogicalSize::new(width, height))
            .map_err(|error| format!("ウィンドウ寸法の変更に失敗しました: {error}"))?;
        // 大きくした結果ウィンドウが画面外へはみ出した場合に備え、見える位置へ引き戻す。
        // `set_size` 直後の `outer_size` は反映前の古い値を返しうるので、当てた寸法を渡す。
        crate::pull_back_on_screen(
            &window,
            (width * scale).round() as u32,
            (height * scale).round() as u32,
        );
        if let Some(id) = emit_layout {
            let _ = window.emit("pet:layout", id);
        }
    }
    Ok(())
}

/// 表示形式を切り替えて保存する（右クリックメニューの入口）。
pub fn select(app: &AppHandle, id: &str) -> Result<(), String> {
    if !is_known(id) {
        return Err(format!("未知の表示形式です: {id}"));
    }
    // 切り替え前の寸法を、その形式のものとして覚えておく
    // （手で広げたチャットへ戻ったとき、広げたままにするため）。
    remember_current_size(app)?;
    {
        let mut patch = Map::new();
        patch.insert("PICOAGENT_LAYOUT".to_string(), json!(id));
        let state = app.state::<AppState>();
        state
            .config
            .set_all(patch)
            .map_err(|error| format!("表示形式の保存に失敗しました: {error}"))?;
    }
    apply(app, id)
}

#[cfg(test)]
mod tests {
    use super::{
        default_size, is_known, BOOT_HEIGHT, BOOT_WIDTH, DEFAULT, LAYOUTS, MIN_HEIGHT, MIN_WIDTH,
    };

    /// 既定の表示形式が定義一覧に含まれていること（未知の値のフォールバック先が実在する）。
    #[test]
    fn 既定の表示形式は定義済み() {
        assert!(is_known(DEFAULT));
    }

    #[test]
    fn 起動画面は小型形式より広い共通寸法を使う() {
        let (chat_width, _) = default_size("chat");
        let (bar_width, _) = default_size("bar");
        assert_eq!((BOOT_WIDTH, BOOT_HEIGHT), (680.0, 430.0));
        assert!(BOOT_WIDTH > chat_width);
        assert!(BOOT_HEIGHT > default_size("bar").1);
        assert!(BOOT_WIDTH >= bar_width);
    }

    /// ID が重複していないこと（重複するとメニューの選択状態が二重に立つ）。
    #[test]
    fn 表示形式のidは重複しない() {
        let mut ids: Vec<&str> = LAYOUTS.iter().map(|(id, _, _, _)| *id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "表示形式の ID が重複しています");
    }

    /// どの形式の既定寸法も最小寸法を下回らないこと
    /// （下回るとウィンドウ側で丸められ、意図した見た目にならない）。
    #[test]
    fn 既定寸法は最小寸法以上() {
        for (id, _, width, height) in LAYOUTS {
            assert!(width >= MIN_WIDTH, "{id} の既定幅が最小幅より小さい");
            assert!(height >= MIN_HEIGHT, "{id} の既定高さが最小高さより小さい");
        }
    }

    /// 未知の ID には既定（ペット）の寸法を返すこと。
    #[test]
    fn 未知の表示形式は既定寸法へ丸める() {
        assert!(!is_known("unknown"));
        assert_eq!(default_size("unknown"), default_size(DEFAULT));
    }
}
