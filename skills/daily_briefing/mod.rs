// daily_briefing/mod.rs - おでかけ前チェックをまとめるスキル。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::daily;

pub struct DailyBriefingTool;

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

#[async_trait]
impl Tool for DailyBriefingTool {
    fn name(&self) -> &'static str {
        "daily_briefing"
    }

    fn label(&self) -> &'static str {
        "おでかけ前チェック"
    }

    fn description(&self) -> String {
        "出かける前に、天気・気温・雨・登録中の通知をまとめて確認する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "location": { "type": "string", "description": "天気を見る地名。空なら現在地推定または既定地点" }
            },
            "required": []
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let location = str_arg(&args, "location");
        let weather_args = if location.is_empty() {
            json!({})
        } else {
            json!({ "location": location })
        };
        let weather = crate::tools::dispatch(cfg, "weather", weather_args).await;
        // 読み取り専用の要約なので、読めなければ通知欄だけ空にして続ける（保存はしない）。
        let events: Vec<String> = daily::load_events(cfg)
            .unwrap_or_default()
            .into_iter()
            .filter(|event| event.enabled)
            .take(5)
            .map(|event| format!("- {} {}", event.due_at, event.text))
            .collect();
        let mut out = String::from("おでかけ前チェック:\n");
        out.push_str(&weather);
        if events.is_empty() {
            out.push_str("\n今日見る通知は今のところ少なめだよ。");
        } else {
            out.push_str("\n控えている通知:\n");
            out.push_str(&events.join("\n"));
        }
        out.push_str("\n雨・気温・風を見て、必要なら傘や上着を持っていこう。");
        out
    }
}

inventory::submit! { ToolReg { make: || Box::new(DailyBriefingTool) } }

