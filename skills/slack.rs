// tools/slack.rs - Slack へメッセージを送るスキル。
//
// Bot token + channel または Incoming Webhook URL を設定から都度読み、秘密値は出力しない。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const SLACK_POST_MESSAGE_URL: &str = "https://slack.com/api/chat.postMessage";
const MAX_TEXT_CHARS: usize = 4000;

pub struct SlackTool;

fn parse_timeout(cfg: &Config) -> Result<u64, String> {
    let timeout = cfg
        .get("PICOAGENT_SLACK_TIMEOUT_SECS", "15")
        .parse::<u64>()
        .map_err(|_| "PICOAGENT_SLACK_TIMEOUT_SECS は 3〜60 の整数で指定してください。".to_string())?;
    if (3..=60).contains(&timeout) {
        Ok(timeout)
    } else {
        Err("PICOAGENT_SLACK_TIMEOUT_SECS は 3〜60 の範囲で指定してください。".to_string())
    }
}

fn text_arg(args: &Value) -> Result<&str, String> {
    let text = args
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| "text が指定されていません。".to_string())?;
    if text.is_empty() {
        return Err("text が空です。".to_string());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("text は {} 文字以内で指定してください。", MAX_TEXT_CHARS));
    }
    Ok(text)
}

#[async_trait]
impl Tool for SlackTool {
    fn name(&self) -> &'static str {
        "slack_send_message"
    }

    fn label(&self) -> &'static str {
        "Slack送信"
    }

    fn description(&self) -> String {
        "Slack の指定チャンネルへメッセージを送る。設定に Bot token + channel、または Incoming Webhook URL が必要。送信前に宛先と本文がユーザー意図に合っている場合だけ使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Slack に送る本文。"
                },
                "channel": {
                    "type": "string",
                    "description": "Bot token 利用時の送信先チャンネルIDまたは名前。省略時は設定値。Webhook 利用時は通常不要。"
                }
            },
            "required": ["text"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_SLACK_BOT_TOKEN", "Slack Bot token", "password", "xoxb-..."),
            Field::new("PICOAGENT_SLACK_CHANNEL", "Slack 既定チャンネル", "text", "#general"),
            Field::new("PICOAGENT_SLACK_WEBHOOK_URL", "Slack Incoming Webhook URL", "password", "https://hooks.slack.com/services/..."),
            Field::new("PICOAGENT_SLACK_TIMEOUT_SECS", "Slack API 待ち時間（秒、3〜60）", "text", "15"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let text = match text_arg(&args) {
            Ok(text) => text,
            Err(error) => return format!("エラー: {}", error),
        };
        let timeout_secs = match parse_timeout(cfg) {
            Ok(timeout) => timeout,
            Err(error) => return format!("エラー: {}", error),
        };
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("PicoAgent/1.0 (Slack tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Slack送信の準備に失敗しました: {}", error),
        };

        let webhook_url = cfg.get("PICOAGENT_SLACK_WEBHOOK_URL", "");
        if !webhook_url.is_empty() {
            let response = client.post(&webhook_url).json(&json!({ "text": text })).send().await;
            return match response {
                Ok(response) if response.status().is_success() => "OK: Slack Webhook へ送信しました。".to_string(),
                Ok(response) => format!("エラー: Slack Webhook が HTTP {} を返しました。", response.status()),
                Err(error) => format!("エラー: Slack Webhook への接続に失敗しました: {}", error),
            };
        }

        let token = cfg.get("PICOAGENT_SLACK_BOT_TOKEN", "");
        if token.is_empty() {
            return "エラー: PICOAGENT_SLACK_BOT_TOKEN または PICOAGENT_SLACK_WEBHOOK_URL が未設定です。".to_string();
        }
        let channel = args
            .get("channel")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|channel| !channel.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_SLACK_CHANNEL", ""));
        if channel.is_empty() {
            return "エラー: channel または PICOAGENT_SLACK_CHANNEL が未設定です。".to_string();
        }

        let response = client
            .post(SLACK_POST_MESSAGE_URL)
            .bearer_auth(token)
            .json(&json!({ "channel": channel, "text": text }))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => return format!("エラー: Slack API への接続に失敗しました: {}", error),
        };
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return format!("エラー: Slack API が HTTP {} を返しました。", status);
        }
        let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        if parsed.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            "OK: Slack へ送信しました。".to_string()
        } else {
            let reason = parsed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown_error");
            format!("エラー: Slack API が失敗を返しました: {}", reason)
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(SlackTool) } }
