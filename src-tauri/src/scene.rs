// scene.rs - 画面構成（シーン）の保存値を正規化する。
//
// 保存先は `PICOAGENT_SCENES` 1キー。表示形式ごとに、利用者が動かしたソースの矩形だけを
// 差分として持つ（既定配置は `ui/scenes/<形式ID>.json` が持つ）。キャラは形式ごとに
// キャラIDで分けて、矩形と構図を一緒に持つ。倍率という値は持たない。
//
// ```json
// {
//   "version": 1,
//   "pet": {
//     "sources": { "chat": { "anchor": "bottom-left", "x": 12, "y": -12, "w": 420, "h": 300 } },
//     "characters": { "mugi": { "anchor": "bottom-right", "x": -8, "y": 0, "w": 300, "h": 480, "framing": "upper" } }
//   }
// }
// ```

use serde_json::{json, Map, Value};

/// 設定キー。
pub const CONFIG_KEY: &str = "PICOAGENT_SCENES";
/// 保存形式の版。形式を変えるときは上げ、`migrate` に移行を1つ足す。
pub const VERSION: u64 = 1;

/// キャラ以外に配置を覚えるソース。キャラは `characters` の下へ分ける。
const SOURCES: [&str; 5] = ["chat", "subtitle", "display", "info", "background"];
/// アンカー（シーンのどの点を基準に置くか）。
const ANCHORS: [&str; 9] = [
    "top-left",
    "top",
    "top-right",
    "left",
    "center",
    "right",
    "bottom-left",
    "bottom",
    "bottom-right",
];
/// 構図（描画範囲の切り出し）。倍率ではない。
const FRAMINGS: [&str; 3] = ["full", "upper", "bust"];
/// 大きさの下限・上限（シーンpx）。
const MIN_SIZE: f64 = 48.0;
const MAX_SIZE: f64 = 20_000.0;
/// 位置の上限（画面外へ飛ばした値をそのまま覚え続けない）。
const MAX_OFFSET: f64 = 20_000.0;
/// 形式ごとに覚えるキャラクターの上限（設定ファイルの肥大を防ぐ）。
const MAX_CHARACTERS: usize = 64;
/// 設定ファイルへ保存するシーンJSONの上限。
const MAX_SERIALIZED_BYTES: usize = 64 * 1024;

fn finite(value: Option<&Value>) -> Option<f64> {
    value?.as_f64().filter(|number| number.is_finite())
}

/// 矩形（アンカー・位置・大きさ）を正規化する。どれか1つでも読めなければ矩形ごと捨てる
/// （一部だけ残すと、残った値と既定値の組み合わせで意図しない場所へ出るため）。
fn normalize_rect(object: &Map<String, Value>) -> Option<Map<String, Value>> {
    let anchor = object.get("anchor")?.as_str()?;
    if !ANCHORS.contains(&anchor) {
        return None;
    }
    let x = finite(object.get("x"))?
        .clamp(-MAX_OFFSET, MAX_OFFSET)
        .round();
    let y = finite(object.get("y"))?
        .clamp(-MAX_OFFSET, MAX_OFFSET)
        .round();
    let w = finite(object.get("w"))?.clamp(MIN_SIZE, MAX_SIZE).round();
    let h = finite(object.get("h"))?.clamp(MIN_SIZE, MAX_SIZE).round();
    let mut rect = Map::new();
    rect.insert("anchor".into(), json!(anchor));
    rect.insert("x".into(), json!(x));
    rect.insert("y".into(), json!(y));
    rect.insert("w".into(), json!(w));
    rect.insert("h".into(), json!(h));
    Some(rect)
}

fn normalize_source(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let mut source = normalize_rect(object).unwrap_or_default();
    if let Some(visible) = object.get("visible").and_then(Value::as_bool) {
        source.insert("visible".into(), json!(visible));
    }
    (!source.is_empty()).then_some(Value::Object(source))
}

/// キャラは矩形と構図のどちらか片方だけでも覚える（構図だけ切り替えた場合など）。
fn normalize_character(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let mut character = normalize_rect(object).unwrap_or_default();
    if let Some(framing) = object
        .get("framing")
        .and_then(Value::as_str)
        .filter(|framing| FRAMINGS.contains(framing))
    {
        character.insert("framing".into(), json!(framing));
    }
    (!character.is_empty()).then_some(Value::Object(character))
}

fn normalize_layout(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let mut layout = Map::new();
    if let Some(sources) = object.get("sources").and_then(Value::as_object) {
        let normalized: Map<String, Value> = sources
            .iter()
            .filter(|(name, _)| SOURCES.contains(&name.as_str()))
            .filter_map(|(name, source)| Some((name.clone(), normalize_source(source)?)))
            .collect();
        if !normalized.is_empty() {
            layout.insert("sources".into(), Value::Object(normalized));
        }
    }
    if let Some(characters) = object.get("characters").and_then(Value::as_object) {
        let normalized: Map<String, Value> = characters
            .iter()
            .filter(|(id, _)| crate::character_pack::is_valid_id(id))
            .filter_map(|(id, character)| Some((id.clone(), normalize_character(character)?)))
            .take(MAX_CHARACTERS)
            .collect();
        if !normalized.is_empty() {
            layout.insert("characters".into(), Value::Object(normalized));
        }
    }
    (!layout.is_empty()).then_some(Value::Object(layout))
}

/// 旧い版の保存値を現行の版へ寄せる。版1が最初の形式なので、いまは版の検査だけを行う。
/// 未来の版（新しいアプリで保存してから古いアプリへ戻した場合）は読めないので `None`。
fn migrate(value: &Value) -> Option<Value> {
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .unwrap_or(VERSION);
    (version <= VERSION).then(|| value.clone())
}

/// 保存前と読み込み後に同じ規則を使えるよう、未知の形式・ソースと不正値を落として返す。
pub fn normalize(value: &Value) -> Value {
    let mut result = Map::new();
    result.insert("version".into(), json!(VERSION));
    let Some(migrated) = migrate(value) else {
        return Value::Object(result);
    };
    let Some(root) = migrated.as_object() else {
        return Value::Object(result);
    };
    for (layout, _, _, _) in crate::layout::AVAILABLE_LAYOUTS {
        if let Some(normalized) = root.get(layout).and_then(normalize_layout) {
            result.insert(layout.to_string(), normalized);
        }
    }
    Value::Object(result)
}

/// 画面から届いた保存要求を検査する。オブジェクトでない・大きすぎる・未来の版の入力は
/// 保存せずエラーにする（空として保存すると、利用者が作った配置が全部消えるため）。
pub fn normalize_checked(value: &Value) -> Result<Value, String> {
    if !value.is_object() {
        return Err("画面配置の保存値がオブジェクトではありません".to_string());
    }
    if migrate(value).is_none() {
        return Err("このアプリより新しい版で保存された画面配置は上書きしません".to_string());
    }
    let normalized = normalize(value);
    let size = serde_json::to_vec(&normalized)
        .map_err(|error| format!("画面配置を保存形式にできません: {error}"))?
        .len();
    if size > MAX_SERIALIZED_BYTES {
        return Err(format!(
            "画面配置が大きすぎます（{}KB / 上限{}KB）",
            size / 1000,
            MAX_SERIALIZED_BYTES / 1000
        ));
    }
    Ok(normalized)
}

/// 保存値を読む。未保存なら既定（空の差分）を返す。
pub fn load(config: &crate::config::Config) -> Value {
    normalize(
        &config
            .get_all()
            .get(CONFIG_KEY)
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
}

/// 1つの表示形式の保存値だけを捨てる（位置とサイズのリセット用）。
pub fn remove_layout(value: &Value, layout: &str) -> Value {
    let mut normalized = normalize(value);
    if let Some(object) = normalized.as_object_mut() {
        object.remove(layout);
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::{normalize, normalize_checked, remove_layout, VERSION};
    use serde_json::json;

    #[test]
    fn 矩形と構図を許可範囲へ正規化する() {
        let normalized = normalize(&json!({
            "version": 1,
            "pet": {
                "sources": {
                    "chat": { "anchor": "bottom-left", "x": 12.4, "y": -99999, "w": 10, "h": 50000 },
                    "unknown": { "anchor": "top-left", "x": 0, "y": 0, "w": 100, "h": 100 }
                },
                "characters": {
                    "mugi": { "anchor": "bottom-right", "x": -8, "y": 0, "w": 300, "h": 480, "framing": "upper" },
                    "../evil": { "anchor": "bottom-right", "x": 0, "y": 0, "w": 300, "h": 480 },
                    "rin": { "framing": "bust" }
                }
            },
            "chat": { "sources": { "chat": { "anchor": "top", "x": 0, "y": 0, "w": 100, "h": 100 } } }
        }));
        assert_eq!(
            normalized,
            json!({
                "version": VERSION,
                "pet": {
                    "sources": { "chat": { "anchor": "bottom-left", "x": 12.0, "y": -20000.0, "w": 48.0, "h": 20000.0 } },
                    "characters": {
                        "mugi": { "anchor": "bottom-right", "x": -8.0, "y": 0.0, "w": 300.0, "h": 480.0, "framing": "upper" },
                        "rin": { "framing": "bust" }
                    }
                }
            })
        );
    }

    #[test]
    fn 読めない矩形は一部だけ残さず捨てる() {
        let normalized = normalize(&json!({
            "pet": { "sources": { "chat": { "anchor": "middle", "x": 1, "y": 2, "w": 300, "h": 200, "visible": false } } }
        }));
        assert_eq!(
            normalized["pet"]["sources"]["chat"],
            json!({ "visible": false })
        );
    }

    #[test]
    fn 壊れた入力と未来の版は保存しない() {
        assert!(normalize_checked(&json!("text")).is_err());
        assert!(normalize_checked(&json!({ "version": VERSION + 1 })).is_err());
        assert!(normalize_checked(&json!({ "version": VERSION })).is_ok());
    }

    #[test]
    fn 形式ごとにだけ保存値を捨てる() {
        let value = json!({
            "pet": { "sources": { "chat": { "anchor": "top", "x": 0, "y": 0, "w": 100, "h": 100 } } },
            "stream": { "sources": { "display": { "anchor": "left", "x": 0, "y": 0, "w": 100, "h": 100 } } }
        });
        let removed = remove_layout(&value, "pet");
        assert!(removed.get("pet").is_none());
        assert!(removed.get("stream").is_some());
    }
}
