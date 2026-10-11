// focus_timer/mod.rs - 集中タイマーと作業見守り通知を設定するスキル。

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::daily::{self, DailyEvent};
use crate::tools::datetime_words::number_before;

pub struct FocusTimerTool;

// 発話全体が開始依頼のときだけ先回りする。停止・質問・引用は通常の解釈へ渡す。
static START_REQUEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[0-9]+分(?:の)?(?:集中)?(?:タイマー|ポモドーロ)(?:を)?(?:始めて|はじめて|開始して|セットして|設定して)?|(?:集中タイマー|タイマー|ポモドーロ)(?:を)?(?:始めて|はじめて|開始して|セットして|設定して))(?:ください|下さい)?[。！!]*$")
        .expect("タイマー開始依頼の正規表現")
});
static WATCH_REQUEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[0-9]+(?:分|時間)(?:おきに|おき|ごとに|ごと))?(?:作業を)?(?:見守って|みまもって|見て)(?:ください|下さい)?[。！!]*$")
        .expect("見守り開始依頼の正規表現")
});

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn minutes_arg(args: &Value, key: &str, default: i64, min: i64, max: i64) -> i64 {
    args.get(key)
        .and_then(Value::as_i64)
        .unwrap_or(default)
        .clamp(min, max)
}

fn list_text(events: &[DailyEvent]) -> String {
    let items: Vec<&DailyEvent> = events
        .iter()
        .filter(|event| matches!(event.kind.as_str(), "focus" | "watch") && event.enabled)
        .collect();
    if items.is_empty() {
        return "動いている集中タイマーや見守りはないよ。".to_string();
    }
    let mut out = String::from("動いているタイマー:");
    for event in items {
        out.push_str(&format!(
            "\n- {} {} {}（id:{}）",
            event.kind, event.due_at, event.text, event.id
        ));
    }
    out
}

#[async_trait]
impl Tool for FocusTimerTool {
    fn name(&self) -> &'static str {
        "focus_timer"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "集中",
            "ポモドーロ",
            "タイマー",
            "見守",
            "みまも",
            "作業を見て",
            "focus",
            "pomodoro",
        ]
    }

    fn intent_priority(&self) -> i32 {
        890
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        if !self.matches_intent(text) {
            return None;
        }
        let text = text.trim();
        let watch = WATCH_REQUEST.is_match(text);
        if !watch && !START_REQUEST.is_match(text) {
            return None;
        }
        let parsed = match number_before(text, "分").and_then(|minutes| {
            if minutes.is_some() {
                return Ok(minutes);
            }
            number_before(text, "時間")?
                .map(|hours| {
                    hours
                        .checked_mul(60)
                        .ok_or_else(|| "時間を分へ換算できません".to_string())
                })
                .transpose()
        }) {
            Ok(parsed) => parsed,
            Err(reason) => return Some(crate::tools::IntentInvocation::invalid(reason)),
        };
        let (action, minutes) = if watch {
            ("watch", parsed.unwrap_or(60).clamp(5, 240))
        } else {
            ("start", parsed.unwrap_or(25).clamp(1, 240))
        };
        Some(crate::tools::IntentInvocation::direct(json!({
            "action": action,
            "minutes": minutes,
            "label": text
        })))
    }

    fn label(&self) -> &'static str {
        "集中タイマー"
    }

    fn description(&self) -> String {
        "ポモドーロ・集中タイマー・作業見守りを開始/一覧/停止する。終了時や見守り間隔ごとに声で知らせる。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["start", "watch", "list", "stop", "clear"] },
                "minutes": { "type": "integer", "minimum": 1, "maximum": 240, "description": "集中時間または見守り間隔。start 既定25、watch 既定60" },
                "label": { "type": "string", "description": "作業名" },
                "id": { "type": "string", "description": "停止対象ID" }
            },
            "required": ["action"]
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = str_arg(&args, "action").to_lowercase();
        // 読む→直す→書き戻すの間、同じ一覧を触る並列ツール実行と競合しないようにする。
        let _guard = crate::store::lock_state(daily::EVENTS_KEY);
        let mut events = match daily::load_events(cfg) {
            Ok(events) => events,
            Err(error) => return format!("エラー: {error}"),
        };
        match action.as_str() {
            "list" => list_text(&events),
            "clear" => {
                events.retain(|event| !matches!(event.kind.as_str(), "focus" | "watch"));
                match daily::save_events(cfg, &events) {
                    Ok(()) => "集中タイマーと見守りを全部止めたよ。".to_string(),
                    Err(error) => format!("エラー: タイマーの保存に失敗しました: {error}"),
                }
            }
            "start" | "watch" => {
                let watch = action == "watch";
                let (default, minimum, prefix, kind) = if watch {
                    (60, 5, "wt", "watch")
                } else {
                    (25, 1, "fc", "focus")
                };
                let minutes = minutes_arg(&args, "minutes", default, minimum, 240);
                let due_at = match daily::after_minutes(minutes) {
                    Ok(due_at) => due_at,
                    Err(reason) => return format!("エラー: {reason}"),
                };
                let label = str_arg(&args, "label");
                let text = match (watch, label.is_empty()) {
                    (true, true) => "作業の様子を見にきたよ。無理してない？".to_string(),
                    (true, false) => format!("「{label}」の様子を見にきたよ。無理してない？"),
                    (false, true) => format!("{minutes}分の集中タイマーが終わったよ。少し休もう。"),
                    (false, false) => format!("「{label}」の{minutes}分集中が終わったよ。少し休もう。"),
                };
                let summary = if watch {
                    format!("{minutes}分おきの作業見守りを始めたよ。")
                } else {
                    format!("集中タイマーを始めたよ: {due_at} に終了")
                };
                events.push(DailyEvent {
                    id: daily::new_id(prefix),
                    kind: kind.to_string(),
                    due_at,
                    text,
                    label,
                    enabled: true,
                    repeat_minutes: watch.then_some(minutes as u32),
                });
                match daily::save_events(cfg, &events) {
                    Ok(()) => summary,
                    Err(error) => format!("エラー: タイマーの保存に失敗しました: {error}"),
                }
            }
            "stop" => {
                let id = str_arg(&args, "id");
                let mut hit = false;
                for event in events.iter_mut() {
                    if matches!(event.kind.as_str(), "focus" | "watch") && event.id == id {
                        event.enabled = false;
                        hit = true;
                    }
                }
                if !hit {
                    return format!("その id のタイマーは見つからなかったよ。\n{}", list_text(&events));
                }
                match daily::save_events(cfg, &events) {
                    Ok(()) => "タイマーを止めたよ。".to_string(),
                    Err(error) => format!("エラー: タイマーの保存に失敗しました: {error}"),
                }
            }
            other => format!("エラー: 未知の action \"{other}\" です。start/watch/list/stop/clear を指定してね。"),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(FocusTimerTool) } }

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn 集中終了と繰り返す見守りを区別して保存する() {
        let directory = tempfile::tempdir().unwrap();
        let cfg = Config::new();
        cfg.load(directory.path().join("config.json")).unwrap();
        for (action, minutes, kind, repeat) in [
            ("start", 15, "focus", None),
            ("watch", 30, "watch", Some(30)),
        ] {
            let reply = FocusTimerTool
                .run(
                    &cfg,
                    json!({"action": action, "minutes": minutes, "label": "資料"}),
                )
                .await;
            assert!(!reply.starts_with("エラー:"), "{reply}");
            let events = daily::load_events(&cfg).unwrap();
            let event = events.last().unwrap();
            assert_eq!(event.kind, kind);
            assert_eq!(event.repeat_minutes, repeat);
            assert!(daily::parse_local_minute(&event.due_at).is_some());
            assert_eq!(event.label, "資料");
        }
        assert_eq!(daily::load_events(&cfg).unwrap().len(), 2);
    }

    #[test]
    fn タイマーの停止や相談を新規開始へ読み替えない() {
        for text in [
            "タイマーを止めて",
            "25分タイマーを削除して",
            "見守りをやめて",
            "集中できない",
            "タイマー一覧を見せて",
            "ポモドーロとは何ですか",
            "25分集中タイマーを始めないで",
            "「25分集中タイマー」と言った",
        ] {
            assert!(FocusTimerTool.intent_invocation(text).is_none(), "{text}");
        }
    }

    #[test]
    fn 明示した開始依頼だけ時間を保って先回りする() {
        for (text, action, minutes) in [
            ("25分集中タイマー", "start", 25),
            ("10分のタイマーを始めてください", "start", 10),
            ("タイマーをはじめて", "start", 25),
            ("30分おきに見守って", "watch", 30),
            ("1時間おきに見守って", "watch", 60),
            ("作業をみまもってください。", "watch", 60),
        ] {
            let invocation = FocusTimerTool.intent_invocation(text).expect(text);
            assert_eq!(invocation.args["action"], action, "{text}");
            assert_eq!(invocation.args["minutes"], minutes, "{text}");
        }
    }
}
