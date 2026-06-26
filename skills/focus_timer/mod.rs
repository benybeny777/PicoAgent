// focus_timer/mod.rs - 集中タイマーと作業見守り通知を設定するスキル。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::daily::{self, DailyEvent};

pub struct FocusTimerTool;

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
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
        out.push_str(&format!("\n- {} {} {}（id:{}）", event.kind, event.due_at, event.text, event.id));
    }
    out
}

#[async_trait]
impl Tool for FocusTimerTool {
    fn name(&self) -> &'static str {
        "focus_timer"
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
        let mut events = daily::load_events(cfg);
        match action.as_str() {
            "list" => list_text(&events),
            "clear" => {
                events.retain(|event| !matches!(event.kind.as_str(), "focus" | "watch"));
                match daily::save_events(cfg, &events) {
                    Ok(()) => "集中タイマーと見守りを全部止めたよ。".to_string(),
                    Err(error) => format!("エラー: タイマーの保存に失敗しました: {error}"),
                }
            }
            "start" => {
                let minutes = minutes_arg(&args, "minutes", 25, 1, 240);
                let label = str_arg(&args, "label");
                let text = if label.is_empty() {
                    format!("{minutes}分の集中タイマーが終わったよ。少し休もう。")
                } else {
                    format!("「{label}」の{minutes}分集中が終わったよ。少し休もう。")
                };
                let event = DailyEvent {
                    id: daily::new_id("fc"),
                    kind: "focus".to_string(),
                    due_at: daily::after_minutes(minutes),
                    text,
                    label,
                    enabled: true,
                    repeat_minutes: None,
                };
                let summary = format!("{} に終了", event.due_at);
                events.push(event);
                match daily::save_events(cfg, &events) {
                    Ok(()) => format!("集中タイマーを始めたよ: {summary}"),
                    Err(error) => format!("エラー: タイマーの保存に失敗しました: {error}"),
                }
            }
            "watch" => {
                let minutes = minutes_arg(&args, "minutes", 60, 5, 240);
                let label = str_arg(&args, "label");
                let text = if label.is_empty() {
                    "作業の様子を見にきたよ。無理してない？".to_string()
                } else {
                    format!("「{label}」の様子を見にきたよ。無理してない？")
                };
                let event = DailyEvent {
                    id: daily::new_id("wt"),
                    kind: "watch".to_string(),
                    due_at: daily::after_minutes(minutes),
                    text,
                    label,
                    enabled: true,
                    repeat_minutes: Some(minutes as u32),
                };
                events.push(event);
                match daily::save_events(cfg, &events) {
                    Ok(()) => format!("{minutes}分おきの作業見守りを始めたよ。"),
                    Err(error) => format!("エラー: 見守りの保存に失敗しました: {error}"),
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

