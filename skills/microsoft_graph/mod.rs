// skills/microsoft_graph/mod.rs - Microsoft Graph のメール・予定・Teams送信スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

pub struct MicrosoftGraphTool;

fn recipients(args: &Value, key: &str) -> Result<Vec<Value>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{} は文字列配列で指定してください。", key))?;
    let mut out = Vec::new();
    for item in array {
        let address = item
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{} は空でない文字列配列で指定してください。", key))?;
        out.push(json!({ "emailAddress": { "address": address } }));
    }
    Ok(out)
}

#[async_trait]
impl Tool for MicrosoftGraphTool {
    fn name(&self) -> &'static str {
        "microsoft_graph"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["outlook", "teams", "microsoft", "office365"]
    }

    fn label(&self) -> &'static str {
        "Microsoft Graph"
    }

    fn description(&self) -> String {
        "Microsoft Graph API で Outlook メール送信、予定一覧/作成、Teams チャンネル送信を行う。設定に OAuth access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["send_mail", "list_events", "create_event", "teams_send"], "description": "実行する操作。" },
                "to": { "type": "array", "items": { "type": "string" }, "description": "send_mail の宛先。" },
                "subject": { "type": "string", "description": "メール件名または予定名。" },
                "body": { "type": "string", "description": "本文。" },
                "start": { "type": "string", "description": "create_event の開始時刻 RFC3339。" },
                "end": { "type": "string", "description": "create_event の終了時刻 RFC3339。" },
                "timezone": { "type": "string", "description": "create_event のタイムゾーン。省略時 Asia/Tokyo。" },
                "team_id": { "type": "string", "description": "teams_send の team ID。省略時は設定値。" },
                "channel_id": { "type": "string", "description": "teams_send の channel ID。省略時は設定値。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_MS_GRAPH_ACCESS_TOKEN",
                "Microsoft Graph の OAuth アクセストークン",
                "password",
                "",
            ),
            Field::new(
                "PICOAGENT_MS_GRAPH_TEAM_ID",
                "Microsoft Teams 既定 team ID",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_MS_GRAPH_CHANNEL_ID",
                "Microsoft Teams 既定 channel ID",
                "text",
                "",
            ),
            Field::int(&crate::config::ints::MS_GRAPH_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_MS_GRAPH_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide(
                "Microsoft Graph",
                &["PICOAGENT_MS_GRAPH_ACCESS_TOKEN"],
                "Outlook/Teams/予定表にアクセスできる OAuth access token が必要です。",
            );
        }
        let timeout = crate::tool_try!(cfg.int(&crate::config::ints::MS_GRAPH_TIMEOUT_SECS)) as u64;
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Microsoft Graph tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                return format!(
                    "エラー: Microsoft Graph API の準備に失敗しました: {}",
                    error
                )
            }
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "send_mail" => {
                let subject = args.get("subject").and_then(Value::as_str).unwrap_or("");
                let body = args.get("body").and_then(Value::as_str).unwrap_or("");
                let to = match recipients(&args, "to") {
                    Ok(to) if !to.is_empty() => to,
                    Ok(_) => return "エラー: to が指定されていません。".to_string(),
                    Err(error) => return format!("エラー: {}", error),
                };
                let payload = json!({
                    "message": {
                        "subject": subject,
                        "body": { "contentType": "Text", "content": body },
                        "toRecipients": to
                    },
                    "saveToSentItems": true
                });
                crate::tools::http::service_reply(cfg,
                    client
                        .post(format!("{}/me/sendMail", GRAPH_BASE))
                        .bearer_auth(token)
                        .json(&payload)
                        .send()
                        .await,
                    "Microsoft Graph API",
                )
                .await
            }
            "list_events" => {
                crate::tools::http::service_reply(cfg,
                    client
                        .get(format!("{}/me/events", GRAPH_BASE))
                        .bearer_auth(token)
                        .query(&[("$top", "10"), ("$orderby", "start/dateTime")])
                        .send()
                        .await,
                    "Microsoft Graph API",
                )
                .await
            }
            "create_event" => {
                let Some(subject) = args.get("subject").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
                    return "エラー: subject が指定されていません。".to_string();
                };
                let Some(start) = args.get("start").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
                    return "エラー: start が指定されていません。".to_string();
                };
                let Some(end) = args.get("end").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
                    return "エラー: end が指定されていません。".to_string();
                };
                let timezone = args.get("timezone").and_then(Value::as_str).unwrap_or("Asia/Tokyo");
                let payload = json!({
                    "subject": subject,
                    "body": { "contentType": "Text", "content": args.get("body").and_then(Value::as_str).unwrap_or("") },
                    "start": { "dateTime": start, "timeZone": timezone },
                    "end": { "dateTime": end, "timeZone": timezone }
                });
                crate::tools::http::service_reply(cfg,
                    client
                        .post(format!("{}/me/events", GRAPH_BASE))
                        .bearer_auth(token)
                        .json(&payload)
                        .send()
                        .await,
                    "Microsoft Graph API",
                )
                .await
            }
            "teams_send" => {
                let team_id = args
                    .get("team_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_MS_GRAPH_TEAM_ID", ""));
                let channel_id = args
                    .get("channel_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_MS_GRAPH_CHANNEL_ID", ""));
                if team_id.is_empty() || channel_id.is_empty() {
                    return super::config_guide("Microsoft Graph", &["PICOAGENT_MS_GRAPH_TEAM_ID", "PICOAGENT_MS_GRAPH_CHANNEL_ID"], "Teams送信では依頼で team_id/channel_id を渡すか、既定値を設定してください。");
                }
                let body = args.get("body").and_then(Value::as_str).unwrap_or("");
                crate::tools::http::service_reply(cfg,
                    client
                        .post(format!("{}/teams/{}/channels/{}/messages", GRAPH_BASE, team_id, channel_id))
                        .bearer_auth(token)
                        .json(&json!({ "body": { "contentType": "text", "content": body } }))
                        .send()
                        .await,
                    "Microsoft Graph API",
                )
                .await
            }
            _ => "エラー: action は send_mail/list_events/create_event/teams_send のいずれかを指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(MicrosoftGraphTool) } }
