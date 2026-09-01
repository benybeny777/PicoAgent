// layout_placements.rs - 配置編集（OBS風）で決めた位置・倍率を正規化する。
//
// 保存先は `PICOAGENT_LAYOUT_PLACEMENTS`（設定ファイル）1本。以前は webview の
// `localStorage` にしか無く、WebViewデータの初期化・再インストール・別PCへの引き継ぎで
// 消えていた。設定へ移して、他の設定と同じ寿命・同じバックアップ経路に載せる。
//
// 形は編集画面の持ち方に合わせる。共有対象（フキダシ・入力欄・履歴・画面内ディスプレイ）は
// 表示形式ごと、キャラクターの立ち絵だけはキャラごと・表示形式ごとに分ける。
//
// ```json
// {
//   "stream": { "bubble": { "translate": "12px -4px", "scale": "1 1" } },
//   "characters": { "mugi": { "stream": { "translate": "18px 12px", "scale": "0.9 0.9" } } }
// }
// ```

use serde_json::{json, Map, Value};

/// キャラ別の配置を入れる予約キー（表示形式名と衝突しない）。
pub const CHARACTERS_KEY: &str = "characters";

/// 配置を覚える対象。編集画面が扱う5つだけを受け付ける。
const SOURCES: [&str; 5] = ["character", "bubble", "reply", "history", "display"];

/// 倍率の下限・上限（`ui/layout-editor.js` の MIN_SCALE / MAX_SCALE と同じ境界）。
const MIN_SCALE: f64 = 0.25;
const MAX_SCALE: f64 = 4.0;
/// 位置の上限（画面外へ飛ばした値をそのまま覚え続けない）。
const MAX_TRANSLATE_PX: f64 = 20_000.0;
/// 覚えるキャラクターの上限（設定ファイルの肥大を防ぐ）。
const MAX_CHARACTERS: usize = 64;

fn is_known_layout(layout: &str) -> bool {
    crate::layout::LAYOUTS
        .iter()
        .any(|(known, _, _, _)| *known == layout)
}

/// 旧「トーク」は「チャット」へ統合済み。保存値も読み込み時に寄せる。
fn canonical_layout(layout: &str) -> &str {
    if layout == "talk" {
        "chat"
    } else {
        layout
    }
}

fn number(value: Option<&str>) -> Option<f64> {
    value?.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// CSS の `translate` を `Xpx Ypx` へ正規化する。2値目を省いた指定はY=0とみなす。
fn normalize_translate(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?;
    let mut parts = raw
        .split_whitespace()
        .map(|part| part.trim_end_matches("px"));
    let x = number(parts.next())?;
    let y = number(parts.next()).unwrap_or(0.0);
    if parts.next().is_some() {
        return None;
    }
    let clamp = |v: f64| v.clamp(-MAX_TRANSLATE_PX, MAX_TRANSLATE_PX).round();
    Some(format!("{}px {}px", clamp(x), clamp(y)))
}

/// CSS の `scale` を `sx sy` へ正規化する。2値目を省いた指定はXと同じ倍率にする。
fn normalize_scale(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?;
    let mut parts = raw.split_whitespace();
    let x = number(parts.next())?;
    let y = number(parts.next()).unwrap_or(x);
    if parts.next().is_some() {
        return None;
    }
    let clamp = |v: f64| v.clamp(MIN_SCALE, MAX_SCALE);
    Some(format!("{} {}", clamp(x), clamp(y)))
}

/// 1つの対象（translate / scale）を正規化する。どちらも読めなければ覚えない。
fn normalize_placement(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let translate = normalize_translate(object.get("translate"));
    let scale = normalize_scale(object.get("scale"));
    if translate.is_none() && scale.is_none() {
        return None;
    }
    Some(json!({
        "translate": translate.unwrap_or_default(),
        "scale": scale.unwrap_or_default(),
    }))
}

/// 1つの表示形式にぶら下がる対象群を正規化する。
fn normalize_sources(value: &Value, allowed: &[&str]) -> Map<String, Value> {
    let mut sources = Map::new();
    let Some(object) = value.as_object() else {
        return sources;
    };
    for (key, placement) in object {
        if !allowed.contains(&key.as_str()) {
            continue;
        }
        if let Some(normalized) = normalize_placement(placement) {
            sources.insert(key.clone(), normalized);
        }
    }
    sources
}

/// 保存前と読み込み後に同じ規則を使えるよう、未知キーと不正値を落として返す。
pub fn normalize(value: &Value) -> Value {
    let mut result = Map::new();
    let Some(root) = value.as_object() else {
        return Value::Object(result);
    };

    // 共有対象（キャラ以外）は表示形式ごとに持つ。
    let shared_sources: Vec<&str> = SOURCES
        .iter()
        .copied()
        .filter(|source| *source != "character")
        .collect();
    for (key, layouts) in root {
        if key == CHARACTERS_KEY {
            continue;
        }
        let layout = canonical_layout(key);
        if !is_known_layout(layout) {
            continue;
        }
        // 旧トークと現行チャットが両方あるときは現行を正本にする。
        if key == "talk" && root.contains_key("chat") {
            continue;
        }
        let sources = normalize_sources(layouts, &shared_sources);
        if !sources.is_empty() {
            result.insert(layout.to_string(), Value::Object(sources));
        }
    }

    // キャラクターの立ち絵はキャラごと・表示形式ごとに持つ。
    let mut characters = Map::new();
    if let Some(source_characters) = root.get(CHARACTERS_KEY).and_then(Value::as_object) {
        for (character_id, layouts) in source_characters {
            if characters.len() >= MAX_CHARACTERS {
                break;
            }
            if !crate::character_pack::is_valid_id(character_id) {
                continue;
            }
            let Some(layouts) = layouts.as_object() else {
                continue;
            };
            let mut normalized_layouts = Map::new();
            for (key, placement) in layouts {
                let layout = canonical_layout(key);
                if !is_known_layout(layout) {
                    continue;
                }
                if key == "talk" && layouts.contains_key("chat") {
                    continue;
                }
                if let Some(normalized) = normalize_placement(placement) {
                    normalized_layouts.insert(layout.to_string(), normalized);
                }
            }
            if !normalized_layouts.is_empty() {
                characters.insert(character_id.clone(), Value::Object(normalized_layouts));
            }
        }
    }
    if !characters.is_empty() {
        result.insert(CHARACTERS_KEY.to_string(), Value::Object(characters));
    }
    Value::Object(result)
}

/// 保存経路では、壊れた既存値を空オブジェクトへ置き換えないよう最上位の型を検証する。
pub fn normalize_checked(value: &Value) -> Result<Value, String> {
    if !value.is_object() {
        return Err(
            "配置設定がオブジェクトではありません。元の値を保持して保存を中止しました。"
                .to_string(),
        );
    }
    Ok(normalize(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 配置設定を許可範囲へ正規化する() {
        let value = json!({
            "stream": {
                "bubble": { "translate": "12.4px -4px", "scale": "9 9" },
                "unknown": { "translate": "1px 1px" }
            },
            "nolayout": { "bubble": { "translate": "1px 1px" } },
            "characters": {
                "mugi": { "stream": { "translate": "18px", "scale": "0.01" } },
                "../bad": { "stream": { "translate": "1px 1px" } }
            }
        });
        assert_eq!(
            normalize(&value),
            json!({
                "stream": { "bubble": { "translate": "12px -4px", "scale": "4 4" } },
                "characters": {
                    // 2値目を省いた translate はY=0、scale はXと同じ倍率として読む。
                    "mugi": { "stream": { "translate": "18px 0px", "scale": "0.25 0.25" } }
                }
            })
        );
    }

    #[test]
    fn キャラの配置は共有対象へ混ざらない() {
        // 表示形式直下の `character` は受け付けない（キャラ別の枠へ入れるため）。
        let value = json!({ "pet": { "character": { "translate": "5px 5px", "scale": "1 1" } } });
        assert_eq!(normalize(&value), json!({}));
    }

    #[test]
    fn 旧トークの配置はチャットへ寄せる() {
        let value = json!({
            "talk": { "bubble": { "translate": "3px 3px", "scale": "1 1" } },
            "characters": { "mugi": { "talk": { "translate": "4px 4px", "scale": "1 1" } } }
        });
        assert_eq!(
            normalize(&value),
            json!({
                "chat": { "bubble": { "translate": "3px 3px", "scale": "1 1" } },
                "characters": { "mugi": { "chat": { "translate": "4px 4px", "scale": "1 1" } } }
            })
        );

        // 現行チャットがあるときはそちらを正本にする。
        let both = json!({
            "talk": { "bubble": { "translate": "3px 3px", "scale": "1 1" } },
            "chat": { "bubble": { "translate": "9px 9px", "scale": "1 1" } }
        });
        assert_eq!(
            normalize(&both),
            json!({ "chat": { "bubble": { "translate": "9px 9px", "scale": "1 1" } } })
        );
    }

    #[test]
    fn 読めない値は覚えずに落とす() {
        let value = json!({
            "pet": {
                "bubble": { "translate": "とても遠く", "scale": "大きく" },
                "reply": { "translate": "1px 2px 3px" },
                "history": { "scale": "1.5" }
            }
        });
        assert_eq!(
            normalize(&value),
            json!({ "pet": { "history": { "translate": "", "scale": "1.5 1.5" } } })
        );
    }

    #[test]
    fn 壊れた保存値は空で上書きしない() {
        assert!(normalize_checked(&json!("こわれている")).is_err());
        assert!(normalize_checked(&json!({})).is_ok());
    }
}
