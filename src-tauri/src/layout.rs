// layout.rs - 表示形式（画面構成）の切り替え。
//
// 1つの透明ウィンドウで「待機」「会話」「長い作業」を全部引き受けると、待機中でも
// フキダシが最大サイズで居座り、履歴を見るには全画面にするしかない。用途ごとに
// 見た目だけを差し替えられるようにして、キャラクター描画・履歴・通知・音声・許可は
// すべて共通のまま使う。
//
// フロントは `<main id="petApp" data-layout="…">` を見て CSS で配置を変えるだけ。
// バックエンド側の仕事は「どれが選ばれているか」と「ウィンドウの寸法」の2つに絞る。

use std::sync::atomic::{AtomicBool, Ordering};

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
pub const LAYOUTS: [(&str, &str, f64, f64); 7] = [
    ("mascot", "ミニ（キャラ中心）", 360.0, 560.0),
    ("pet", "ペット（キャラ＋会話）", 900.0, 600.0),
    ("chat", "チャット（LINE風）", 520.0, 760.0),
    ("bar", "バー（すばやく質問）", 900.0, 320.0),
    ("workspace", "作業（情報を並べる）", 1400.0, 850.0),
    ("stream", "配信（キャラを大きく）", 1920.0, 1080.0),
    ("widget", "ウィジェット（予定と通知）", 440.0, 600.0),
];

/// 2026-08-07以前の既定寸法。保存値がこの組み合わせと完全一致する環境だけ新しい
/// 読みやすい既定へ移し、ユーザーが手で調整した寸法はそのまま残す。
const LEGACY_DEFAULT_SIZES: [(&str, f64, f64); 8] = [
    ("mascot", 240.0, 380.0),
    ("pet", 680.0, 430.0),
    ("chat", 380.0, 640.0),
    ("talk", 380.0, 640.0),
    ("bar", 640.0, 300.0),
    ("workspace", 1100.0, 700.0),
    ("stream", 1560.0, 920.0),
    ("widget", 320.0, 440.0),
];

/// ウィンドウの最小寸法。`tauri.conf.json` の `minWidth` / `minHeight` と揃える必要がある
/// （小さい形式ほどここで頭打ちになるため、設定値より小さい既定は持たせない）。
pub const MIN_WIDTH: f64 = 200.0;
pub const MIN_HEIGHT: f64 = 240.0;
/// 起動準備中だけ使う共通VTuber画面の寸法。保存中の表示形式には影響させない。
pub const BOOT_WIDTH: f64 = 680.0;
pub const BOOT_HEIGHT: f64 = 430.0;

/// 起動準備中の共通寸法を当てている間だけ立つ。
/// この間のウィンドウ実寸はユーザーが選んだものではないので、`remember_current_size()`
/// が覚えてしまうと、モデル取得の待ち時間中に表示形式を切り替えたり設定を保存した
/// だけで、chat / widget など小さい形式の記憶が 680×430 に置き換わる。
static BOOT_SIZE_ACTIVE: AtomicBool = AtomicBool::new(false);

fn begin_boot_size() {
    BOOT_SIZE_ACTIVE.store(true, Ordering::Relaxed);
}

fn end_boot_size() {
    BOOT_SIZE_ACTIVE.store(false, Ordering::Relaxed);
}

/// 起動準備中の共通寸法を当てている最中か。
/// この間は実寸を形式の記憶へ残さず、形式の切り替えも当てずに保留する。
fn boot_size_active() -> bool {
    BOOT_SIZE_ACTIVE.load(Ordering::Relaxed)
}

/// 形式ごとのウィンドウ寸法を覚えておく設定キー。
/// `{"chat":{"w":520,"h":760},…}` の形。位置は形式をまたいで共有する
/// （置き場所はユーザーの習慣で、形式ごとに変えたいものではないため）。
pub(crate) const SIZES_KEY: &str = "PICOAGENT_WINDOW_LAYOUTS";

/// 旧 `talk` は見た目も用途も近かった `chat` へ統合する。
fn normalize_id(id: &str) -> &str {
    if id == "talk" {
        "chat"
    } else {
        id
    }
}

/// 指定 ID が定義済みかどうか。
pub fn is_known(id: &str) -> bool {
    LAYOUTS.iter().any(|(layout, _, _, _)| *layout == id)
}

/// 選択中の表示形式。未知の値は既定へ丸める。
pub fn current(config: &crate::config::Config) -> String {
    let id = config.get("PICOAGENT_LAYOUT", DEFAULT);
    let normalized = normalize_id(&id);
    if is_known(normalized) {
        normalized.to_string()
    } else {
        DEFAULT.to_string()
    }
}

/// 表示形式の既定寸法。
fn default_size(id: &str) -> (f64, f64) {
    let id = normalize_id(id);
    LAYOUTS
        .iter()
        .find(|(layout, _, _, _)| *layout == id)
        .or_else(|| LAYOUTS.iter().find(|(layout, _, _, _)| *layout == DEFAULT))
        .map(|(_, _, width, height)| (*width, *height))
        .expect("既定の表示形式はLAYOUTSに定義済み")
}

/// 形式ごとに覚えている寸法。無ければ既定寸法。
pub fn size_for(config: &crate::config::Config, id: &str) -> (f64, f64) {
    let id = normalize_id(id);
    let (default_width, default_height) = default_size(id);
    let saved = config.get_all().get(SIZES_KEY).and_then(|value| {
        value
            .get(id)
            .cloned()
            .or_else(|| (id == "chat").then(|| value.get("talk").cloned()).flatten())
    });
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
    let legacy_default =
        LEGACY_DEFAULT_SIZES
            .iter()
            .any(|(legacy_id, legacy_width, legacy_height)| {
                *legacy_id == id && *legacy_width == width && *legacy_height == height
            });
    // ペット形式にはさらに古い暫定既定がある。完全一致だけを移行対象にする。
    let legacy_pet = id == "pet" && height == 430.0 && matches!(width as u32, 560 | 600);
    if legacy_default || legacy_pet {
        return (default_width, default_height);
    }
    (width.max(MIN_WIDTH), height.max(MIN_HEIGHT))
}

/// 形式ごとの寸法を覚える（ユーザーがウィンドウを手で変えたとき）。
pub fn remember_size(app: &AppHandle, id: &str, width: f64, height: f64) -> Result<(), String> {
    let id = normalize_id(id);
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
    // 起動準備中の共通寸法は利用者の選択ではないため、形式の記憶を上書きしない。
    if boot_size_active() {
        return Ok(());
    }
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
    let normalized = normalize_id(id);
    let id = if is_known(normalized) {
        normalized
    } else {
        DEFAULT
    };
    let (width, height) = {
        let state = app.state::<AppState>();
        size_for(&state.config, id)
    };

    // 保存中の形式の寸法へ戻した時点で、起動準備中の共通寸法は終わる。
    end_boot_size();
    apply_window_size(app, width, height, Some(id))
}

/// 保存中の表示形式にかかわらず、起動準備中は共通のVTuber画面へ広げる。
pub fn apply_boot(app: &AppHandle) -> Result<(), String> {
    begin_boot_size();
    apply_window_size(app, BOOT_WIDTH, BOOT_HEIGHT, None)
}

/// 形式の変更を当てる。ただし起動準備中は当てずに共通の起動画面を保つ。
/// 準備完了時の `pet_finish_boot` が保存中の形式（＝ここで保存した新しい形式）を当てる。
pub fn apply_unless_booting(app: &AppHandle, id: &str) -> Result<(), String> {
    if boot_size_active() {
        return Ok(());
    }
    apply(app, id)
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
            if let Err(error) = window.emit("pet:layout", id) {
                crate::log_paths::append_app_log(
                    app,
                    &format!("表示形式を画面へ反映できません: {error}"),
                );
            }
        }
    }
    Ok(())
}

/// 表示形式を切り替えて保存する（右クリックメニューの入口）。
pub fn select(app: &AppHandle, id: &str) -> Result<(), String> {
    let id = normalize_id(id);
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
    apply_unless_booting(app, id)
}

#[cfg(test)]
mod tests {
    use super::{
        begin_boot_size, boot_size_active, default_size, end_boot_size, is_known, BOOT_HEIGHT,
        BOOT_WIDTH, DEFAULT, LAYOUTS, MIN_HEIGHT, MIN_WIDTH,
    };

    /// 既定の表示形式が定義一覧に含まれていること（未知の値のフォールバック先が実在する）。
    #[test]
    fn 既定の表示形式は定義済み() {
        assert!(is_known(DEFAULT));
    }

    #[test]
    fn 起動画面は選択形式に依存しない共通寸法を使う() {
        assert_eq!((BOOT_WIDTH, BOOT_HEIGHT), (680.0, 430.0));
        assert_ne!((BOOT_WIDTH, BOOT_HEIGHT), default_size("chat"));
        assert_ne!((BOOT_WIDTH, BOOT_HEIGHT), default_size("bar"));
    }

    /// 起動準備中の共通寸法を、その表示形式の寸法として覚えないこと
    /// （覚えると、待ち時間中の形式切替・設定保存だけで小型形式の記憶が 680×430 になる）。
    #[test]
    fn 起動準備中の寸法は表示形式の記憶へ残さない() {
        end_boot_size();
        assert!(!boot_size_active());
        begin_boot_size();
        assert!(boot_size_active());
        end_boot_size();
        assert!(!boot_size_active());
    }

    /// 最小寸法が `tauri.conf.json` と一致すること。
    /// ずれるとウィンドウ側で丸められ、復元時の下限判定とも食い違う。
    #[test]
    fn 最小寸法はtauri設定と一致する() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
            .expect("tauri.conf.json を読めません");
        let conf: serde_json::Value =
            serde_json::from_str(&text).expect("tauri.conf.json を解析できません");
        let window = &conf["app"]["windows"][0];
        assert_eq!(window["minWidth"].as_f64(), Some(MIN_WIDTH));
        assert_eq!(window["minHeight"].as_f64(), Some(MIN_HEIGHT));
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

    #[test]
    fn 旧トーク形式はチャットへ統合する() {
        assert!(!is_known("talk"));
        assert_eq!(default_size("talk"), default_size("chat"));
    }

    fn read_repo_file(relative: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("リポジトリルートを取得できない");
        std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("{relative} を読めません: {error}"))
    }

    /// 表示形式の一覧は、ここの `LAYOUTS` のほかに設定画面のカード定義
    /// （`ui/settings.js` の `LAYOUT_CHOICES`。既定寸法を文字列で持つ）と、
    /// プレビュー画像をコピーする `xtask` の一覧にも複製されている。
    /// 片方だけ直すと「カードに古い寸法が出る」「新形式のカードが出ない」が黙って起きるので、
    /// ID と既定寸法の一致をここで検出する。
    #[test]
    fn 表示形式の一覧はフロントとxtaskに揃っている() {
        let settings_js = read_repo_file("ui/settings.js");
        let choices = settings_js
            .split("const LAYOUT_CHOICES = [")
            .nth(1)
            .and_then(|rest| rest.split("];").next())
            .expect("ui/settings.js に LAYOUT_CHOICES が見つかりません");
        let assets_rs = read_repo_file("xtask/src/assets.rs");

        let mut listed: Vec<&str> = choices
            .split("id: \"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        let mut expected: Vec<&str> = LAYOUTS.iter().map(|(id, _, _, _)| *id).collect();
        listed.sort_unstable();
        expected.sort_unstable();
        assert_eq!(
            listed, expected,
            "ui/settings.js の LAYOUT_CHOICES と表示形式の定義がずれています"
        );

        for (id, _, width, height) in LAYOUTS {
            let entry = choices
                .split(&format!("id: \"{id}\""))
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("{id} のカード定義を読み取れません"));
            let size = format!("size: \"{}×{}\"", width as u32, height as u32);
            assert!(
                entry.contains(&size),
                "{id} のカードに出す既定寸法が定義とずれています（期待: {size}）"
            );
            assert!(
                assets_rs.contains(&format!("\"{id}\"")),
                "{id} のプレビュー画像が xtask のコピー対象に入っていません"
            );
        }
    }
}
