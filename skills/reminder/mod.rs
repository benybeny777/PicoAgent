// reminder/mod.rs - 日常リマインダーを追加・一覧・削除するスキル。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::daily::{self, DailyEvent};

pub struct ReminderTool;

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn due_arg(args: &Value) -> Option<String> {
    let minutes = args.get("minutes_from_now").and_then(Value::as_i64);
    if let Some(minutes) = minutes.filter(|m| *m > 0) {
        return Some(daily::after_minutes(minutes));
    }
    let due_at = str_arg(args, "due_at");
    daily::parse_local_minute(&due_at).map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
}

fn list_text(events: &[DailyEvent]) -> String {
    let reminders: Vec<&DailyEvent> = events
        .iter()
        .filter(|event| event.kind == "reminder" && event.enabled)
        .collect();
    if reminders.is_empty() {
        return "登録中のリマインダーはないよ。".to_string();
    }
    let mut out = String::from("登録中のリマインダー:");
    for event in reminders {
        out.push_str(&format!("\n- {} {}（id:{}）", event.due_at, event.text, event.id));
    }
    out
}

#[async_trait]
impl Tool for ReminderTool {
    fn name(&self) -> &'static str {
        "reminder"
    }

    fn label(&self) -> &'static str {
        "リマインダー"
    }

    fn description(&self) -> String {
        "指定時刻や数分後に声で知らせる日常リマインダー。追加・一覧・削除・全削除ができる。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["add", "list", "remove", "clear"] },
                "text": { "type": "string", "description": "知らせる内容" },
                "due_at": { "type": "string", "description": "YYYY-MM-DDTHH:MM または YYYY-MM-DD HH:MM" },
                "minutes_from_now": { "type": "integer", "minimum": 1, "description": "現在から何分後に知らせるか" },
                "id": { "type": "string", "description": "削除対象ID" }
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
                events.retain(|event| event.kind != "reminder");
                match daily::save_events(cfg, &events) {
                    Ok(()) => "リマインダーを全部消したよ。".to_string(),
                    Err(error) => format!("エラー: リマインダーの保存に失敗しました: {error}"),
                }
            }
            "add" => {
                let text = str_arg(&args, "text");
                if text.is_empty() {
                    return "エラー: 何を知らせるか指定してね。".to_string();
                }
                let Some(due_at) = due_arg(&args) else {
                    return "エラー: due_at（YYYY-MM-DDTHH:MM）か minutes_from_now を指定してね。".to_string();
                };
                let event = DailyEvent {
                    id: daily::new_id("rm"),
                    kind: "reminder".to_string(),
                    due_at,
                    text,
                    label: String::new(),
                    enabled: true,
                    repeat_minutes: None,
                };
                let summary = format!("{} {}", event.due_at, event.text);
                events.push(event);
                match daily::save_events(cfg, &events) {
                    Ok(()) => format!("リマインダーを設定したよ: {summary}"),
                    Err(error) => format!("エラー: リマインダーの保存に失敗しました: {error}"),
                }
            }
            "remove" => {
                let id = str_arg(&args, "id");
                let before = events.len();
                events.retain(|event| !(event.kind == "reminder" && event.id == id));
                if events.len() == before {
                    return format!("その id のリマインダーは見つからなかったよ。\n{}", list_text(&events));
                }
                match daily::save_events(cfg, &events) {
                    Ok(()) => "リマインダーを削除したよ。".to_string(),
                    Err(error) => format!("エラー: リマインダーの保存に失敗しました: {error}"),
                }
            }
            other => format!("エラー: 未知の action \"{other}\" です。add/list/remove/clear を指定してね。"),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ReminderTool) } }
