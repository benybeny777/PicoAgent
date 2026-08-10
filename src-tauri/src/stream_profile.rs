// stream_profile.rs - キャラクター別の配信表示上書きを検証して解決する。

use serde_json::{Map, Value};

use crate::config::Config;

const BACKGROUNDS: [&str; 4] = ["pastel", "gaming", "simple", "custom"];
const DISPLAYS: [&str; 5] = ["dashboard", "game", "clock", "image", "none"];

fn valid_character_id(value: &str) -> bool {
    crate::character_pack::is_valid_id(value)
}

fn allowed(value: Option<&Value>, choices: &[&str], fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|value| choices.contains(value))
        .unwrap_or(fallback)
        .to_string()
}

fn path(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn color(value: Option<&Value>) -> String {
    let value = value.and_then(Value::as_str).unwrap_or("#5a3b4b");
    let digits = value.strip_prefix('#').unwrap_or_default();
    if digits.len() == 6
        && digits
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        value.to_ascii_lowercase()
    } else {
        "#5a3b4b".to_string()
    }
}

pub fn normalize(value: &Value) -> Value {
    let mut profiles = Map::new();
    let Some(source) = value.as_object() else {
        return Value::Object(profiles);
    };
    for (character_id, profile) in source {
        if !valid_character_id(character_id) {
            continue;
        }
        let Some(profile) = profile.as_object() else {
            continue;
        };
        profiles.insert(
            character_id.clone(),
            serde_json::json!({
                "background": allowed(profile.get("background"), &BACKGROUNDS, "pastel"),
                "backgroundImage": path(profile.get("backgroundImage")),
                "display": allowed(profile.get("display"), &DISPLAYS, "dashboard"),
                "displayImage": path(profile.get("displayImage")),
                "captionColor": color(profile.get("captionColor")),
            }),
        );
    }
    Value::Object(profiles)
}

/// 保存経路では、壊れた既存値を空オブジェクトへ置き換えないよう最上位の型を検証する。
pub fn normalize_checked(value: &Value) -> Result<Value, String> {
    if !value.is_object() {
        return Err("キャラクター別の配信設定がオブジェクトではありません。元の値を保持して保存を中止しました。".to_string());
    }
    Ok(normalize(value))
}

#[derive(Debug, PartialEq)]
pub struct Appearance {
    pub background: String,
    pub background_image: String,
    pub display: String,
    pub display_image: String,
    pub caption_color: String,
}

pub fn resolve(config: &Config, character_id: &str) -> Appearance {
    let all = config.get_all();
    let profiles = normalize(all.get("PICOAGENT_STREAM_PROFILES").unwrap_or(&Value::Null));
    let profile = profiles.get(character_id).and_then(Value::as_object);
    Appearance {
        background: profile
            .and_then(|value| value.get("background"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| config.get("PICOAGENT_STREAM_BACKGROUND", "pastel")),
        background_image: profile
            .and_then(|value| value.get("backgroundImage"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| config.get("PICOAGENT_STREAM_BACKGROUND_IMAGE", "")),
        display: profile
            .and_then(|value| value.get("display"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| config.get("PICOAGENT_STREAM_DISPLAY", "dashboard")),
        display_image: profile
            .and_then(|value| value.get("displayImage"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| config.get("PICOAGENT_STREAM_DISPLAY_IMAGE", "")),
        caption_color: profile
            .and_then(|value| value.get("captionColor"))
            .and_then(Value::as_str)
            .unwrap_or("#5a3b4b")
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 不正な配信上書きを安全な値へ戻す() {
        let value = serde_json::json!({
            "mugi": { "background": "bad", "display": "image", "captionColor": "red" },
            "../bad": { "background": "gaming" }
        });
        assert_eq!(
            normalize(&value),
            serde_json::json!({
                "mugi": { "background": "pastel", "backgroundImage": "", "display": "image", "displayImage": "", "captionColor": "#5a3b4b" }
            })
        );
    }

    #[test]
    fn 壊れた配信設定を空で上書きしない() {
        assert!(normalize_checked(&serde_json::json!("broken")).is_err());
    }
}
