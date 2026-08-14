// character_framing.rs - キャラクター別・表示形式別の構図設定を正規化する。

use serde_json::{json, Map, Value};

const PRESETS: [&str; 4] = ["bust", "upper", "full", "custom"];

fn number(value: Option<&Value>, fallback: f64) -> f64 {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.parse::<f64>().ok())
        })
        .filter(|value| value.is_finite())
        .unwrap_or(fallback)
}

fn defaults(preset: &str) -> (f64, f64, f64) {
    match preset {
        "bust" => (2.05, 0.0, 38.0),
        "full" => (1.0, 0.0, 0.0),
        _ => (1.55, 0.0, 18.0),
    }
}

fn valid_character_id(value: &str) -> bool {
    crate::character_pack::is_valid_id(value)
}

/// 保存前と読み込み後に同じ規則を使えるよう、未知キーと不正値を落として返す。
pub fn normalize(value: &Value) -> Value {
    let mut characters = Map::new();
    let Some(source_characters) = value.as_object() else {
        return Value::Object(characters);
    };
    for (character_id, layouts) in source_characters {
        if !valid_character_id(character_id) {
            continue;
        }
        let Some(layouts) = layouts.as_object() else {
            continue;
        };
        let mut normalized_layouts = Map::new();
        for (layout, framing) in layouts {
            // 旧トークの個別構図はチャットへ移す。両方あれば現行chatを正本にする。
            if layout == "talk" && layouts.contains_key("chat") {
                continue;
            }
            let layout = if layout == "talk" {
                "chat"
            } else {
                layout.as_str()
            };
            if !crate::layout::LAYOUTS
                .iter()
                .any(|(known, _, _, _)| *known == layout)
            {
                continue;
            }
            let Some(framing) = framing.as_object() else {
                continue;
            };
            let preset = framing
                .get("preset")
                .and_then(Value::as_str)
                .filter(|preset| PRESETS.contains(preset))
                .unwrap_or("custom");
            let (default_zoom, default_x, default_y) = defaults(preset);
            normalized_layouts.insert(
                layout.to_string(),
                json!({
                    "preset": preset,
                    "zoom": number(framing.get("zoom"), default_zoom).clamp(1.0, 4.0),
                    "x": number(framing.get("x"), default_x).clamp(-4000.0, 4000.0).round(),
                    "y": number(framing.get("y"), default_y).clamp(-4000.0, 4000.0).round(),
                }),
            );
        }
        if !normalized_layouts.is_empty() {
            characters.insert(character_id.clone(), Value::Object(normalized_layouts));
        }
    }
    Value::Object(characters)
}

/// 保存経路では、壊れた既存値を空オブジェクトへ置き換えないよう最上位の型を検証する。
pub fn normalize_checked(value: &Value) -> Result<Value, String> {
    if !value.is_object() {
        return Err("キャラクター構図設定がオブジェクトではありません。元の値を保持して保存を中止しました。".to_string());
    }
    Ok(normalize(value))
}

/// 指定したキャラと表示形式の構図だけを削除し、空になった親要素も掃除する。
pub fn remove(value: &Value, character_id: &str, layout: &str) -> Value {
    let mut normalized = normalize(value);
    let Some(characters) = normalized.as_object_mut() else {
        return Value::Object(Map::new());
    };
    if let Some(layouts) = characters
        .get_mut(character_id)
        .and_then(Value::as_object_mut)
    {
        layouts.remove(layout);
        if layouts.is_empty() {
            characters.remove(character_id);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 構図設定を許可範囲へ正規化する() {
        let value = json!({
            "mugi": {
                "pet": { "preset": "broken", "zoom": 99, "x": -9999, "y": "24.6", "extra": true },
                "unknown": { "preset": "full" }
            },
            "../bad": { "pet": { "preset": "upper" } }
        });
        assert_eq!(
            normalize(&value),
            json!({ "mugi": { "pet": { "preset": "custom", "zoom": 4.0, "x": -4000.0, "y": 25.0 } } })
        );
    }

    #[test]
    fn 現在の構図だけを削除する() {
        let value = json!({
            "mugi": {
                "pet": { "preset": "upper", "zoom": 1.55, "x": 0, "y": 18 },
                "stream": { "preset": "bust", "zoom": 2.05, "x": 0, "y": 38 }
            }
        });
        assert_eq!(
            remove(&value, "mugi", "pet"),
            json!({ "mugi": { "stream": { "preset": "bust", "zoom": 2.05, "x": 0.0, "y": 38.0 } } })
        );
    }

    #[test]
    fn 壊れた構図設定を空で上書きしない() {
        assert!(normalize_checked(&json!(["broken"])).is_err());
    }

    #[test]
    fn 旧トーク構図をチャットへ移す() {
        let value = json!({
            "mugi": { "talk": { "preset": "upper", "zoom": 1.55, "x": 3, "y": 18 } }
        });
        assert_eq!(
            normalize(&value),
            json!({ "mugi": { "chat": { "preset": "upper", "zoom": 1.55, "x": 3.0, "y": 18.0 } } })
        );
    }
}
