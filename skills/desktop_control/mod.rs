// skills/desktop_control/mod.rs - デスクトップのマウス・キーボードを操作するスキル。
//
// **shell を経由しない**のがこのスキルの要点。以前から使える `code_runner` は
// モデルに PowerShell / Python を書かせて実行する形で、危険な操作かどうかを
// 「生成された文字列を検査して」判定していた。文字列検査は原理的に後手で、
// クオートやエンコードで回避できる。ここでは**操作を固定の型付き action に限定**し、
// 「その操作がそもそも存在しない」状態を作る（検査ではなく設計で防ぐ）。
//
// 入力の実体は enigo（Windows / macOS）で、OS ごとの分岐はクレート側に閉じる。
// ウィンドウ一覧・前面化は `window.rs`（現在は Windows のみ。macOS は Accessibility の
// 許可と AX API が必要で未実装）。
//
// 既定は無効。有効化しても、操作回数の上限・入力文字数の上限・座標の画面内チェックを
// 常に通す。実行した操作は結果文字列として会話とトレース（`pet_traces`）に残る。

pub mod window;

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use enigo::{
    Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings,
};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

/// 1回に入力できる文字数の上限（誤爆時の被害と、貼り付け作業の実用性の折り合い）。
const MAX_TEXT_CHARS: usize = 2_000;
/// 操作回数を数える窓。
const RATE_WINDOW: Duration = Duration::from_secs(60);

pub struct DesktopControlTool;

fn enabled(cfg: &Config) -> bool {
    cfg.get("PICOAGENT_DESKTOP_ENABLED", "false")
        .trim()
        .eq_ignore_ascii_case("true")
}

fn max_ops_per_min(cfg: &Config) -> usize {
    cfg.get("PICOAGENT_DESKTOP_MAX_OPS_PER_MIN", "60")
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=600).contains(value))
        .unwrap_or(60)
}

/// 直近1分の操作時刻。暴走したときに際限なく入力し続けないための歯止め。
fn recent_ops() -> &'static Mutex<Vec<Instant>> {
    static OPS: OnceLock<Mutex<Vec<Instant>>> = OnceLock::new();
    OPS.get_or_init(|| Mutex::new(Vec::new()))
}

fn check_rate(limit: usize) -> Result<(), String> {
    let mut ops = recent_ops()
        .lock()
        .map_err(|_| "エラー: 操作回数を数えられません。".to_string())?;
    let now = Instant::now();
    ops.retain(|at| now.duration_since(*at) < RATE_WINDOW);
    if ops.len() >= limit {
        return Err(format!(
            "エラー: 直近1分の操作が上限（{limit}回）に達しました。時間を置くか、PICOAGENT_DESKTOP_MAX_OPS_PER_MIN を見直してください。"
        ));
    }
    ops.push(now);
    Ok(())
}

fn make_enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|error| {
        format!("エラー: デスクトップ操作を初期化できません: {error}")
    })
}

/// キー名を enigo のキーへ写す。生のキーコードはモデルに書かせない。
fn parse_key(name: &str) -> Option<Key> {
    let key = match name.trim().to_ascii_lowercase().as_str() {
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "ctrl" | "control" => Key::Control,
        "alt" => Key::Alt,
        "shift" => Key::Shift,
        "meta" | "win" | "cmd" | "super" => Key::Meta,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        other => {
            let mut chars = other.chars();
            let first = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Key::Unicode(first)
        }
    };
    Some(key)
}

/// `Ctrl+C` のような組み合わせを、修飾キーと本体へ分ける。
fn parse_combo(spec: &str) -> Result<(Vec<Key>, Key), String> {
    let parts: Vec<&str> = spec
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let Some((last, modifiers)) = parts.split_last() else {
        return Err("エラー: keys が空です。".to_string());
    };
    let mut mods = Vec::new();
    for name in modifiers {
        let key = parse_key(name).ok_or_else(|| format!("エラー: 不明な修飾キーです: {name}"))?;
        if !matches!(key, Key::Control | Key::Alt | Key::Shift | Key::Meta) {
            return Err(format!("エラー: {name} は修飾キーとして使えません。"));
        }
        mods.push(key);
    }
    let main = parse_key(last).ok_or_else(|| format!("エラー: 不明なキーです: {last}"))?;
    Ok((mods, main))
}

fn button_of(name: &str) -> Result<Button, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "left" | "" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        "middle" => Ok(Button::Middle),
        other => Err(format!("エラー: 不明なマウスボタンです: {other}")),
    }
}

/// 座標が画面内かを確かめる。桁を間違えた指示で画面外へ飛ばさないための最低限の検査。
fn check_point(enigo: &Enigo, x: i32, y: i32) -> Result<(), String> {
    let (width, height) = enigo
        .main_display()
        .map_err(|error| format!("エラー: 画面サイズを取得できません: {error}"))?;
    if x < 0 || y < 0 || x >= width || y >= height {
        return Err(format!(
            "エラー: 座標が画面の外です（指定 {x},{y} / 画面 {width}x{height}）。"
        ));
    }
    Ok(())
}

fn coord(args: &Value, key: &str) -> Result<Option<i32>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let integer = value
        .as_i64()
        .ok_or_else(|| format!("エラー: {key} は整数で指定してください。"))?;
    let converted = i32::try_from(integer)
        .map_err(|_| format!("エラー: {key} が扱える範囲を超えています: {integer}"))?;
    Ok(Some(converted))
}

fn required_coord(args: &Value, key: &str) -> Result<i32, String> {
    coord(args, key)?.ok_or_else(|| format!("エラー: {key} が指定されていません。"))
}

#[async_trait]
impl Tool for DesktopControlTool {
    fn name(&self) -> &'static str {
        "desktop_control"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["クリックして", "クリックし", "マウス", "キーボード", "キーを押", "入力して", "画面を操作", "pc操作", "デスクトップ操作", "ドラッグ"]
    }

    fn label(&self) -> &'static str {
        "デスクトップ操作"
    }

    fn description(&self) -> String {
        "デスクトップのマウスとキーボードを直接操作する。クリック・ドラッグ・文字入力・キー送出・スクロールを、決められた操作の組み合わせだけで行う（シェルやスクリプトは介さない）。画面の内容はスクリーンショットで確認してから座標を決めること。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["screen_size", "cursor", "window_list", "window_focus", "move", "click", "double_click", "drag", "scroll", "type_text", "key"],
                    "description": "screen_size: 画面サイズ / cursor: 現在のカーソル位置 / window_list: ウィンドウ一覧（識別子つき）/ window_focus: 識別子のウィンドウを前面へ / move: 移動 / click: クリック / double_click: ダブルクリック / drag: ドラッグ / scroll: スクロール / type_text: 文字入力 / key: キー送出。"
                },
                "x": { "type": "integer", "description": "対象の X 座標（画面左上が 0,0）。" },
                "y": { "type": "integer", "description": "対象の Y 座標。" },
                "to_x": { "type": "integer", "description": "drag の移動先 X。" },
                "to_y": { "type": "integer", "description": "drag の移動先 Y。" },
                "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "クリックするボタン（既定 left）。" },
                "text": { "type": "string", "description": "type_text で入力する文字列。" },
                "keys": { "type": "string", "description": "key で送るキー。単体（Enter / Tab / F5 / a）か組み合わせ（Ctrl+C / Alt+Tab / Ctrl+Shift+T）。" },
                "amount": { "type": "integer", "description": "scroll の量。正が下/右、負が上/左。" },
                "window_id": { "type": "string", "description": "window_focus で前面にするウィンドウの識別子（window_list が返した id）。" },
                "axis": { "type": "string", "enum": ["vertical", "horizontal"], "description": "scroll の方向（既定 vertical）。" }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "開いているウィンドウを一覧して",
            "メモ帳を前面にして",
            "画面を見て、右上の閉じるボタンを押して",
            "この文章を今開いているウィンドウへ入力して",
            "Ctrl+S で保存して",
        ]
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_DESKTOP_ENABLED", "デスクトップ操作を有効にする（既定は無効）", "checkbox", "false"),
            Field::new("PICOAGENT_DESKTOP_MAX_OPS_PER_MIN", "1分あたりの操作回数の上限（1〜600）", "text", "60"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        if !enabled(cfg) {
            return crate::tools::config_guide(
                "デスクトップ操作",
                &["PICOAGENT_DESKTOP_ENABLED"],
                "既定では無効です。有効にすると、マウス・キーボードを直接操作できるようになります。取り消せない操作を行うため、必要なときだけ有効にしてください。",
            );
        }
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        let mut enigo = match make_enigo() {
            Ok(enigo) => enigo,
            Err(error) => return error,
        };

        // 読み取りだけの action は回数制限の対象外にする（画面サイズの確認で枠を消費させない）。
        match action.as_str() {
            "screen_size" => {
                return match enigo.main_display() {
                    Ok((width, height)) => format!("画面サイズ: {width}x{height}"),
                    Err(error) => format!("エラー: 画面サイズを取得できません: {error}"),
                }
            }
            "cursor" => {
                return match enigo.location() {
                    Ok((x, y)) => format!("カーソル位置: {x},{y}"),
                    Err(error) => format!("エラー: カーソル位置を取得できません: {error}"),
                }
            }
            "window_list" => {
                return match window::list() {
                    Ok(windows) => window::format(&windows),
                    Err(error) => error,
                }
            }
            _ => {}
        }

        if let Err(error) = check_rate(max_ops_per_min(cfg)) {
            return error;
        }

        match action.as_str() {
            "window_focus" => {
                let Some(window_id) = args.get("window_id").and_then(Value::as_str) else {
                    return "エラー: window_focus には window_id（window_list の識別子）が必要です。".to_string();
                };
                let windows = match window::list() {
                    Ok(windows) => windows,
                    Err(error) => return error,
                };
                let Some(target) = windows
                    .iter()
                    .find(|window| window::identifier(window) == window_id)
                else {
                    return format!(
                        "エラー: ウィンドウ識別子 {window_id} は現在の一覧にありません。window_list で取り直してください。"
                    );
                };
                match window::focus(target.handle) {
                    Ok(()) => format!("ウィンドウを前面にしました: {}", target.title),
                    Err(error) => error,
                }
            }
            "move" | "click" | "double_click" => {
                // 座標があれば移動してから押す。省略時は現在位置で押す。
                let x = match coord(&args, "x") {
                    Ok(value) => value,
                    Err(error) => return error,
                };
                let y = match coord(&args, "y") {
                    Ok(value) => value,
                    Err(error) => return error,
                };
                if x.is_some() != y.is_some() {
                    return "エラー: 座標を指定する場合は x と y の両方が必要です。".to_string();
                }
                if let (Some(x), Some(y)) = (x, y) {
                    if let Err(error) = check_point(&enigo, x, y) {
                        return error;
                    }
                    if let Err(error) = enigo.move_mouse(x, y, Coordinate::Abs) {
                        return format!("エラー: マウスを移動できません: {error}");
                    }
                } else if action == "move" {
                    return "エラー: move には x と y が必要です。".to_string();
                }
                if action == "move" {
                    return "マウスを移動しました。".to_string();
                }
                let button = match button_of(
                    args.get("button").and_then(Value::as_str).unwrap_or("left"),
                ) {
                    Ok(button) => button,
                    Err(error) => return error,
                };
                let times = if action == "double_click" { 2 } else { 1 };
                for _ in 0..times {
                    if let Err(error) = enigo.button(button, Direction::Click) {
                        return format!("エラー: クリックできません: {error}");
                    }
                }
                let where_ = match (x, y) {
                    (Some(x), Some(y)) => format!("{x},{y}"),
                    _ => "現在位置".to_string(),
                };
                format!(
                    "{}しました（{where_}）。",
                    if action == "double_click" { "ダブルクリック" } else { "クリック" }
                )
            }
            "drag" => {
                let coords = (
                    required_coord(&args, "x"),
                    required_coord(&args, "y"),
                    required_coord(&args, "to_x"),
                    required_coord(&args, "to_y"),
                );
                let (x, y, tx, ty) = match coords {
                    (Ok(x), Ok(y), Ok(tx), Ok(ty)) => (x, y, tx, ty),
                    (Err(error), _, _, _)
                    | (_, Err(error), _, _)
                    | (_, _, Err(error), _)
                    | (_, _, _, Err(error)) => return error,
                };
                for (px, py) in [(x, y), (tx, ty)] {
                    if let Err(error) = check_point(&enigo, px, py) {
                        return error;
                    }
                }
                let button = match button_of(
                    args.get("button").and_then(Value::as_str).unwrap_or("left"),
                ) {
                    Ok(button) => button,
                    Err(error) => return error,
                };
                // 1操作ずつ成功を確認してから次へ進む。配列へ式を並べると、Rust は
                // 配列生成時に全操作を実行するため、先頭が失敗しても後続が動いてしまう。
                if let Err(error) = enigo.move_mouse(x, y, Coordinate::Abs) {
                    return format!("エラー: ドラッグ開始位置へ移動できません: {error}");
                }
                if let Err(error) = enigo.button(button, Direction::Press) {
                    return format!("エラー: ドラッグ開始時にボタンを押せません: {error}");
                }
                if let Err(error) = enigo.move_mouse(tx, ty, Coordinate::Abs) {
                    let _ = enigo.button(button, Direction::Release);
                    return format!("エラー: ドラッグ中に移動できません: {error}");
                }
                if let Err(error) = enigo.button(button, Direction::Release) {
                    let _ = enigo.button(button, Direction::Release);
                    return format!("エラー: ドラッグ終了時にボタンを離せません: {error}");
                }
                format!("ドラッグしました（{x},{y} → {tx},{ty}）。")
            }
            "scroll" => {
                let amount = match coord(&args, "amount") {
                    Ok(Some(value)) => value,
                    Ok(None) => 3,
                    Err(error) => return error,
                };
                let axis = match args.get("axis").and_then(Value::as_str).unwrap_or("vertical") {
                    "horizontal" => Axis::Horizontal,
                    "vertical" => Axis::Vertical,
                    other => return format!("エラー: 不明なスクロール方向です: {other}"),
                };
                match enigo.scroll(amount, axis) {
                    Ok(()) => format!("スクロールしました（{amount}）。"),
                    Err(error) => format!("エラー: スクロールできません: {error}"),
                }
            }
            "type_text" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    return "エラー: text が空です。".to_string();
                }
                if text.chars().count() > MAX_TEXT_CHARS {
                    return format!(
                        "エラー: 一度に入力できるのは {MAX_TEXT_CHARS} 文字までです（指定 {} 文字）。",
                        text.chars().count()
                    );
                }
                match enigo.text(text) {
                    Ok(()) => format!("{} 文字を入力しました。", text.chars().count()),
                    Err(error) => format!("エラー: 文字を入力できません: {error}"),
                }
            }
            "key" => {
                let spec = args.get("keys").and_then(Value::as_str).unwrap_or("");
                if spec.trim().is_empty() {
                    return "エラー: keys が指定されていません。".to_string();
                }
                let (modifiers, main) = match parse_combo(spec) {
                    Ok(parsed) => parsed,
                    Err(error) => return error,
                };
                let mut pressed = Vec::new();
                for key in &modifiers {
                    if let Err(error) = enigo.key(*key, Direction::Press) {
                        for pressed_key in pressed.iter().rev() {
                            let _ = enigo.key(*pressed_key, Direction::Release);
                        }
                        return format!("エラー: 修飾キーを押せません: {error}");
                    }
                    pressed.push(*key);
                }
                let result = enigo.key(main, Direction::Click);
                // 押した修飾キーは、本体が失敗しても必ず離す。
                for key in pressed.iter().rev() {
                    let _ = enigo.key(*key, Direction::Release);
                }
                match result {
                    Ok(()) => format!("キーを送りました（{spec}）。"),
                    Err(error) => format!("エラー: キーを送れません: {error}"),
                }
            }
            other => format!(
                "エラー: 未知の action \"{other}\" です。screen_size / cursor / window_list / window_focus / move / click / double_click / drag / scroll / type_text / key のいずれかを指定してください。"
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(DesktopControlTool) } }

#[cfg(test)]
mod tests {
    use super::{button_of, check_rate, coord, enabled, max_ops_per_min, parse_combo, parse_key, recent_ops};
    use crate::config::Config;
    use enigo::{Button, Key};

    #[test]
    fn 既定では無効() {
        assert!(!enabled(&Config::new()));
        let cfg = Config::new();
        cfg.set("PICOAGENT_DESKTOP_ENABLED", "true");
        assert!(enabled(&cfg));
    }

    #[test]
    fn キー名を写像する() {
        assert!(matches!(parse_key("Enter"), Some(Key::Return)));
        assert!(matches!(parse_key("esc"), Some(Key::Escape)));
        assert!(matches!(parse_key("F5"), Some(Key::F5)));
        assert!(matches!(parse_key("a"), Some(Key::Unicode('a'))));
        assert!(parse_key("そんなキーはない").is_none(), "複数文字の未知名は拒否する");
    }

    #[test]
    fn 組み合わせキーを分解する() {
        let (mods, main) = parse_combo("Ctrl+C").unwrap();
        assert_eq!(mods.len(), 1);
        assert!(matches!(mods[0], Key::Control));
        assert!(matches!(main, Key::Unicode('c')));

        let (mods, main) = parse_combo("Ctrl+Shift+T").unwrap();
        assert_eq!(mods.len(), 2);
        assert!(matches!(main, Key::Unicode('t')));
    }

    #[test]
    fn 修飾キーでないものを修飾位置に置けない() {
        let error = parse_combo("Enter+C").unwrap_err();
        assert!(error.contains("修飾キー"), "error: {error}");
    }

    #[test]
    fn ボタン名を写像する() {
        assert!(matches!(button_of("right"), Ok(Button::Right)));
        assert!(matches!(button_of("middle"), Ok(Button::Middle)));
        assert!(button_of("なにか").is_err(), "不明な指定は拒否する");
    }

    #[test]
    fn 座標はi32の範囲外を拒否する() {
        let args = serde_json::json!({ "x": i64::MAX });
        assert!(coord(&args, "x").is_err());
    }

    #[test]
    fn 座標は未指定と不正値を区別する() {
        assert_eq!(coord(&serde_json::json!({}), "x").unwrap(), None);
        assert!(coord(&serde_json::json!({ "x": "10" }), "x").is_err());
    }

    #[test]
    fn 待ち時間の上限は範囲外なら既定へ戻す() {
        assert_eq!(max_ops_per_min(&Config::new()), 60);
        let cfg = Config::new();
        cfg.set("PICOAGENT_DESKTOP_MAX_OPS_PER_MIN", "9999");
        assert_eq!(max_ops_per_min(&cfg), 60, "範囲外は既定へ戻す");
    }

    #[test]
    fn 操作回数の上限を超えたら止める() {
        recent_ops().lock().unwrap().clear();
        assert!(check_rate(2).is_ok());
        assert!(check_rate(2).is_ok());
        let error = check_rate(2).unwrap_err();
        assert!(error.contains("上限"), "error: {error}");
        recent_ops().lock().unwrap().clear();
    }
}
