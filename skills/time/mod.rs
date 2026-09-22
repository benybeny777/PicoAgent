// skills/time/mod.rs - エージェント機能コネクタ（例）: 現在時刻を返す。副作用なし。

use async_trait::async_trait;
use chrono::Local;
use chrono_tz::Tz;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;

pub struct TimeTool;

const RESULT_MATERIAL_PROMPT: &str = include_str!("prompts/result-material.md");

fn render_result_material(user_text: &str, tool_result: &str) -> String {
    crate::tools::render_intent_prompt(RESULT_MATERIAL_PROMPT, user_text, tool_result)
}

/// 「今何時？」「今日何日？」「何曜日？」のように、いまの時刻・日付そのものを聞く発話か。
/// 以前はこの判定が無く、「何時」を調べもの扱いするWeb検索が先に動いて、なぞなぞのページを
/// 参考に「〇時だよ」と答えていた。予定・目覚まし（「明日7時に起こして」「何時に会議？」）は
/// それぞれのスキルへ任せるため、現在を指す語と組になったときだけ一致させる。
fn asks_current_time(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let now_words = ["今", "いま", "現在"];
    let time_words = ["何時", "なんじ", "の時間", "の時刻", "時刻"];
    let date_words = ["何日", "なんにち", "何曜", "なんよう", "の日付", "日付"];
    let asks_time = now_words.iter().any(|now| {
        time_words.iter().any(|word| {
            compact.contains(&format!("{now}{word}"))
                || compact.contains(&format!("{now}って{word}"))
        })
    });
    let asks_date = ["今日", "きょう", "今", "現在"].iter().any(|now| {
        date_words.iter().any(|word| {
            compact.contains(&format!("{now}{word}"))
                || compact.contains(&format!("{now}は{word}"))
                || compact.contains(&format!("{now}って{word}"))
        })
    });
    let bare = [
        "何時?",
        "何時？",
        "なんじ?",
        "なんじ？",
        "何曜日?",
        "何曜日？",
    ]
    .iter()
    .any(|question| compact == *question);
    asks_time || asks_date || bare
}

#[async_trait]
impl Tool for TimeTool {
    fn name(&self) -> &'static str {
        "get_time"
    }
    fn intent_priority(&self) -> i32 {
        // Web検索（100）より先に判定する。
        700
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        asks_current_time(text).then(|| {
            crate::tools::IntentInvocation::material(json!({}), render_result_material, false)
        })
    }

    fn description(&self) -> String {
        "現在の日付と時刻を返す。ユーザーが今の時間・日付・曜日を尋ねたときに使う。".to_string()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "IANA タイムゾーン名（例: Asia/Tokyo）。省略時はローカル時刻。"
                }
            },
            "required": []
        })
    }

    async fn run(&self, _cfg: &Config, args: Value) -> String {
        let fmt = "%Y/%m/%d %H:%M:%S";
        if let Some(tz_name) = args.get("timezone").and_then(|v| v.as_str()) {
            if !tz_name.is_empty() {
                match tz_name.parse::<Tz>() {
                    Ok(tz) => {
                        let now = Local::now().with_timezone(&tz);
                        return format!("現在時刻: {}（{}）", now.format(fmt), tz_name);
                    }
                    Err(_) => return format!("エラー: 不明なタイムゾーンです: {}", tz_name),
                }
            }
        }
        let now = Local::now();
        format!("現在時刻: {}", now.format(fmt))
    }
}

inventory::submit! { ToolReg { make: || Box::new(TimeTool) } }

#[cfg(test)]
mod tests {
    use super::asks_current_time;

    #[test]
    fn いまの時刻と日付を聞く発話だけに一致する() {
        for text in [
            "今何時？短く答えて",
            "いま何時",
            "今って何時？",
            "今日何日？",
            "今日は何曜日",
            "現在時刻は？",
            "何時？",
        ] {
            assert!(asks_current_time(text), "{text}");
        }
        for text in [
            "明日7時に起こして",
            "会議は何時から？",
            "何時に寝るのがいい？",
            "東京とロンドンの時差は何時間？",
        ] {
            assert!(!asks_current_time(text), "{text}");
        }
    }
}
