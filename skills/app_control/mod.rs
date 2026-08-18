// app_control - PicoAgent 自身のアプリ操作スキル。

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;

use super::{Tool, ToolReg};

pub struct AppControlTool;

#[async_trait]
impl Tool for AppControlTool {
    fn name(&self) -> &'static str {
        "app_control"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["アプリ終了", "アプリを終了", "アプリ閉じ", "アプリを閉じ", "このアプリ", "このソフト", "picoagent", "ピコエージェント", "pico agent"]
    }

    fn label(&self) -> &'static str {
        "アプリ操作"
    }

    fn description(&self) -> String {
        "PicoAgentアプリ自身を操作する。アプリ終了・PicoAgentを閉じる依頼に使う。タイマーやタスクの終了には使わない。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["quit"],
                    "description": "quit は PicoAgent アプリを終了する。"
                }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "アプリ終了して",
            "PicoAgent を閉じて",
            "このアプリを終了して",
        ]
    }

    async fn run(&self, _cfg: &Config, args: Value) -> String {
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "quit" => "PICOAGENT_APP_QUIT_REQUESTED\nPicoAgentを終了します。".to_string(),
            _ => "エラー: action は quit を指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(AppControlTool) } }
