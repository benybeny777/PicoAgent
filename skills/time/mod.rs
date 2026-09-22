// skills/time/mod.rs - エージェント機能コネクタ（例）: 現在時刻を返す。副作用なし。

use async_trait::async_trait;
use chrono::{Datelike, Local, Weekday};
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
/// それぞれのスキルへ任せるため、現在日時の質問全体に一致するときだけ先回りする。
fn asks_current_time(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let compact = compact.trim_matches(['？', '?', '。', '！', '!']);
    // 回答の長さだけを指定した後続句は、日時の対象を変えない。
    let question = ["短く答えて", "短く教えて", "一言で答えて"]
        .iter()
        .find_map(|suffix| compact.strip_suffix(suffix))
        .unwrap_or(compact)
        .trim_end_matches(['？', '?', '。', '、', ',', '！', '!']);
    let prefixes = ["", "今", "いま", "現在", "今日", "きょう"];
    let particles = ["", "は", "って", "の"];
    let subjects = [
        "何時",
        "なんじ",
        "時刻",
        "時間",
        "何日",
        "なんにち",
        "何曜日",
        "何曜",
        "なんようび",
        "なんよう",
        "日付",
    ];
    let endings = [
        "",
        "は",
        "ですか",
        "かな",
        "だっけ",
        "なの",
        "なのかな",
        "か教えて",
        "か教えてください",
        "教えて",
        "教えてください",
        "を教えて",
        "を教えてください",
        "知りたい",
        "を知りたい",
    ];
    // 質問全体で一致させ、時間割・期間・予定・地域指定・複合依頼を時計へ横取りしない。
    prefixes.iter().any(|prefix| {
        let Some(rest) = question.strip_prefix(prefix) else {
            return false;
        };
        particles.iter().any(|particle| {
            if prefix.is_empty() && !particle.is_empty() {
                return false;
            }
            let Some(rest) = rest.strip_prefix(particle) else {
                return false;
            };
            subjects.iter().any(|subject| {
                rest.strip_prefix(subject)
                    .is_some_and(|ending| endings.contains(&ending))
            })
        })
    })
}

fn weekday_label(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "月曜日",
        Weekday::Tue => "火曜日",
        Weekday::Wed => "水曜日",
        Weekday::Thu => "木曜日",
        Weekday::Fri => "金曜日",
        Weekday::Sat => "土曜日",
        Weekday::Sun => "日曜日",
    }
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
                        return format!(
                            "現在時刻: {} {}（{}）",
                            now.format(fmt),
                            weekday_label(now.weekday()),
                            tz_name
                        );
                    }
                    Err(_) => return format!("エラー: 不明なタイムゾーンです: {}", tz_name),
                }
            }
        }
        let now = Local::now();
        format!(
            "現在時刻: {} {}",
            now.format(fmt),
            weekday_label(now.weekday())
        )
    }
}

inventory::submit! { ToolReg { make: || Box::new(TimeTool) } }

#[cfg(test)]
mod tests {
    use super::{asks_current_time, weekday_label};
    use chrono::{Datelike, NaiveDate};

    #[test]
    fn 自然な時計質問は検索より先に時計へ渡す() {
        for text in ["今は何時？", "いまはなんじですか？", "今日の日付は？"] {
            let matched = crate::tools::intent_for_text(text).unwrap();
            assert_eq!(matched.tool_name, "get_time", "{text}");
        }
        for text in [
            "今の時間割を調べて",
            "今何時間働いた？",
            "ロンドンは今何時？",
        ] {
            let matched = crate::tools::intent_for_text(text);
            assert!(
                matched.is_none_or(|matched| matched.tool_name != "get_time"),
                "{text}"
            );
        }
    }

    #[test]
    fn 曜日は暦から日本語で返す() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 22).unwrap();
        assert_eq!(weekday_label(date.weekday()), "火曜日");
    }

    #[test]
    fn いまの時刻と日付を聞く発話だけに一致する() {
        for text in [
            "今何時？短く答えて",
            "いま何時",
            "今って何時？",
            "今は何時？",
            "いまはなんじですか？",
            "現在の時刻を教えてください",
            "今日の日付は？",
            "きょうはなんようび？",
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
            "今の時間割を調べて",
            "今何時間働いた？",
            "今日何日休める？",
            "今何時から会議？",
            "ロンドンは今何時？",
            "今ロンドンは何時？",
            "今何時？ロンドンで",
            "今日は何曜日が休みか調べて",
            "今何時？それと今日のニュースを調べて",
        ] {
            assert!(!asks_current_time(text), "{text}");
        }
    }
}
