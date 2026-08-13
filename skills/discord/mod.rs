// skills/discord/mod.rs - Discord Webhook へメッセージを送るスキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_CONTENT_CHARS: usize = 2000;

pub struct DiscordTool;

#[async_trait]
impl Tool for DiscordTool {
    fn name(&self) -> &'static str {
        "discord_send_message"
    }

    fn label(&self) -> &'static str {
        "Discord送信"
    }

    fn description(&self) -> String {
        "Discord Webhook URL へメッセージを送る。設定に Webhook URL が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Discord に送る本文。" },
                "username": { "type": "string", "description": "Webhook 表示名。省略可。" }
            },
            "required": ["content"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_DISCORD_WEBHOOK_URL", "Discord の Webhook URL", "password", "https://discord.com/api/webhooks/..."),
            Field::new("PICOAGENT_DISCORD_TIMEOUT_SECS", "Discord API 待ち時間（秒、3〜60）", "text", "15"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let webhook = cfg.get("PICOAGENT_DISCORD_WEBHOOK_URL", "");
        if webhook.is_empty() {
            return super::config_guide("Discord送信", &["PICOAGENT_DISCORD_WEBHOOK_URL"], "Discord の Incoming Webhook URL を入れてください。");
        }
        let content = match args.get("content").and_then(Value::as_str).map(str::trim) {
            Some(content) if !content.is_empty() => content,
            _ => return "エラー: content が指定されていません。".to_string(),
        };
        if content.chars().count() > MAX_CONTENT_CHARS {
            return format!("エラー: content は {} 文字以内で指定してください。", MAX_CONTENT_CHARS);
        }
        let timeout_secs = cfg
            .get("PICOAGENT_DISCORD_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let mut payload = json!({ "content": content });
        if let Some(username) = args.get("username").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) {
            payload["username"] = json!(username);
        }
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("PicoAgent/1.0 (Discord tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Discord送信の準備に失敗しました: {}", error),
        };
        match client.post(webhook).json(&payload).send().await {
            Ok(response) if response.status().is_success() => "OK: Discord へ送信しました。".to_string(),
            Ok(response) => format!("エラー: Discord Webhook が HTTP {} を返しました。", response.status()),
            Err(error) => format!("エラー: Discord Webhook への接続に失敗しました: {}", error),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(DiscordTool) } }
