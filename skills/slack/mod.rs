// skills/slack/mod.rs - Slack へメッセージを送るスキル。
//
// Bot token + channel または Incoming Webhook URL を設定から都度読み、秘密値は出力しない。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const SLACK_POST_MESSAGE_URL: &str = "https://slack.com/api/chat.postMessage";
const SLACK_AUTH_TEST_URL: &str = "https://slack.com/api/auth.test";
const SLACK_HISTORY_URL: &str = "https://slack.com/api/conversations.history";
const MAX_TEXT_CHARS: usize = 4000;

pub struct SlackTool;

fn parse_timeout(cfg: &Config) -> u64 {
    // フォールバック許可: Slack の補助 timeout 設定が壊れていても既定値で続行する。
    cfg.get("PICOAGENT_SLACK_TIMEOUT_SECS", "15")
        .parse::<u64>()
        .ok()
        .filter(|value| (3..=60).contains(value))
        .unwrap_or(15)
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

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["slack", "スラック", "slack履歴", "slack history"]
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
                "action": {
                    "type": "string",
                    "enum": ["send", "history", "test"],
                    "description": "send は送信、history はチャンネル履歴取得、test は Bot token の疎通確認。省略時 send。"
                },
                "text": {
                    "type": "string",
                    "description": "Slack に送る本文。"
                },
                "channel": {
                    "type": "string",
                    "description": "Bot token 利用時の送信先チャンネルIDまたは名前。省略時は設定値。Webhook 利用時は通常不要。"
                },
                "limit": { "type": "integer", "minimum": 1, "maximum": 20, "description": "history の最大件数。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_SLACK_BOT_TOKEN", "Slack の Bot トークン", "password", "xoxb-..."),
            Field::new("PICOAGENT_SLACK_CHANNEL", "Slack 既定チャンネル", "text", "#general"),
            Field::new("PICOAGENT_SLACK_WEBHOOK_URL", "Slack の Incoming Webhook URL", "password", "https://hooks.slack.com/services/..."),
            Field::new("PICOAGENT_SLACK_TIMEOUT_SECS", "Slack API 待ち時間（秒、3〜60）", "text", "15"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("send").trim();
        let timeout_secs = parse_timeout(cfg);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("PicoAgent/1.0 (Slack tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Slack送信の準備に失敗しました: {}", error),
        };

        let webhook_url = cfg.get("PICOAGENT_SLACK_WEBHOOK_URL", "");
        let token = cfg.get("PICOAGENT_SLACK_BOT_TOKEN", "");
        if action == "test" {
            if !token.is_empty() {
                let response = match client.get(SLACK_AUTH_TEST_URL).bearer_auth(&token).send().await {
                    Ok(response) => response,
                    Err(error) => return format!("エラー: Slack API への接続に失敗しました: {}", error),
                };
                let status = response.status();
                let body = crate::tool_response_text!(response);
                if !status.is_success() {
                    return format!("エラー: Slack API が HTTP {} を返しました。", status);
                }
                let parsed: Value = crate::tool_response_json!(&body);
                if parsed.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    return format!(
                        "OK: Slack Bot token は有効です。team={} user={}",
                        parsed.get("team").and_then(Value::as_str).unwrap_or("unknown"),
                        parsed.get("user").and_then(Value::as_str).unwrap_or("unknown")
                    );
                }
                return format!(
                    "エラー: Slack API が失敗を返しました: {}",
                    parsed.get("error").and_then(Value::as_str).unwrap_or("unknown_error")
                );
            }
            if !webhook_url.is_empty() {
                return "OK: Slack Webhook URL は設定されています。Webhook は非送信の疎通確認 API がないため、実送信で確認してください。".to_string();
            }
            return super::config_guide("Slack", &["PICOAGENT_SLACK_WEBHOOK_URL", "PICOAGENT_SLACK_BOT_TOKEN"], "接続テストには Incoming Webhook URL、または Bot token が必要です。");
        }

        let channel = args
            .get("channel")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|channel| !channel.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_SLACK_CHANNEL", ""));
        if action == "history" {
            if token.is_empty() {
                return super::config_guide("Slack履歴", &["PICOAGENT_SLACK_BOT_TOKEN"], "履歴取得には conversations.history 権限を持つ Bot token が必要です。Webhook では読み取りできません。");
            }
            if channel.is_empty() {
                return super::config_guide("Slack履歴", &["PICOAGENT_SLACK_CHANNEL"], "履歴取得には channel を渡すか既定チャンネルを設定してください。");
            }
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .filter(|value| (1..=20).contains(value))
                .unwrap_or(10)
                .to_string();
            let response = match client
                .get(SLACK_HISTORY_URL)
                .bearer_auth(&token)
                .query(&[("channel", channel.as_str()), ("limit", limit.as_str())])
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => return format!("エラー: Slack API への接続に失敗しました: {}", error),
            };
            let status = response.status();
            let body = crate::tool_response_text!(response);
            if !status.is_success() {
                return format!("エラー: Slack API が HTTP {} を返しました。", status);
            }
            let parsed: Value = crate::tool_response_json!(&body);
            if !parsed.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                return format!(
                    "エラー: Slack API が失敗を返しました: {}",
                    parsed.get("error").and_then(Value::as_str).unwrap_or("unknown_error")
                );
            }
            let mut out = format!("Slack履歴（{}）:", channel);
            for message in parsed
                .get("messages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                out.push_str(&format!(
                    "\n- {} / {}: {}",
                    message.get("ts").and_then(Value::as_str).unwrap_or(""),
                    message.get("user").and_then(Value::as_str).unwrap_or("unknown"),
                    message.get("text").and_then(Value::as_str).unwrap_or("")
                ));
            }
            return out;
        }

        if action != "send" && !action.is_empty() {
            return "エラー: action は send / history / test を指定してください。".to_string();
        }
        let text = match text_arg(&args) {
            Ok(text) => text,
            Err(error) => return format!("エラー: {}", error),
        };
        if !webhook_url.is_empty() {
            let response = client.post(&webhook_url).json(&json!({ "text": text })).send().await;
            return match response {
                Ok(response) if response.status().is_success() => "OK: Slack Webhook へ送信しました。".to_string(),
                Ok(response) => format!("エラー: Slack Webhook が HTTP {} を返しました。", response.status()),
                Err(error) => format!("エラー: Slack Webhook への接続に失敗しました: {}", error),
            };
        }

        if token.is_empty() {
            return super::config_guide("Slack送信", &["PICOAGENT_SLACK_WEBHOOK_URL", "PICOAGENT_SLACK_BOT_TOKEN"], "Incoming Webhook URL、または Bot token を設定してください。Bot token 利用時は送信先チャンネルも必要です。");
        }
        if channel.is_empty() {
            return super::config_guide("Slack送信", &["PICOAGENT_SLACK_CHANNEL"], "Bot token 利用時は依頼で channel を渡すか、既定の送信先チャンネルを設定してください。");
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
        let body = crate::tool_response_text!(response);
        if !status.is_success() {
            return format!("エラー: Slack API が HTTP {} を返しました。", status);
        }
        let parsed: Value = crate::tool_response_json!(&body);
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
