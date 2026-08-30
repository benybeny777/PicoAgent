// skills/trello/mod.rs - Trello のカード一覧・作成スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const TRELLO_BASE: &str = "https://api.trello.com/1";

pub struct TrelloTool;

#[async_trait]
impl Tool for TrelloTool {
    fn name(&self) -> &'static str {
        "trello"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["trello"]
    }

    fn label(&self) -> &'static str {
        "Trello"
    }

    fn description(&self) -> String {
        "Trello API でカード一覧取得・カード作成を行う。設定に API key/token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list_cards", "create_card"], "description": "list_cards/create_card。" },
                "list_id": { "type": "string", "description": "Trello list ID。省略時は設定値。" },
                "name": { "type": "string", "description": "create_card のカード名。" },
                "desc": { "type": "string", "description": "create_card の説明。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_TRELLO_KEY", "Trello の API キー", "password", ""),
            Field::new(
                "PICOAGENT_TRELLO_TOKEN",
                "Trello のトークン",
                "password",
                "",
            ),
            Field::new(
                "PICOAGENT_TRELLO_LIST_ID",
                "Trello 既定 list ID",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_TRELLO_TIMEOUT_SECS",
                "Trello API 待ち時間（秒、3〜60）",
                "text",
                "15",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let key = cfg.get("PICOAGENT_TRELLO_KEY", "");
        let token = cfg.get("PICOAGENT_TRELLO_TOKEN", "");
        if key.is_empty() || token.is_empty() {
            return super::config_guide(
                "Trello",
                &["PICOAGENT_TRELLO_KEY", "PICOAGENT_TRELLO_TOKEN"],
                "Trello API key と token が必要です。",
            );
        }
        let list_id = args
            .get("list_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_TRELLO_LIST_ID", ""));
        if list_id.is_empty() {
            return super::config_guide(
                "Trello",
                &["PICOAGENT_TRELLO_LIST_ID"],
                "依頼で list_id を渡すか、既定のリストIDを設定してください。",
            );
        }
        let timeout = cfg
            .get("PICOAGENT_TRELLO_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Trello tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Trello API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "list_cards" => {
                handle_text_response(
                    client
                        .get(format!("{}/lists/{}/cards", TRELLO_BASE, list_id))
                        .query(&[("key", key), ("token", token)])
                        .send()
                        .await,
                    "Trello API",
                )
                .await
            }
            "create_card" => {
                let Some(name) = args
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                else {
                    return "エラー: name が指定されていません。".to_string();
                };
                let desc = args.get("desc").and_then(Value::as_str).unwrap_or("");
                handle_text_response(
                    client
                        .post(format!("{}/cards", TRELLO_BASE))
                        .query(&[
                            ("key", key.as_str()),
                            ("token", token.as_str()),
                            ("idList", list_id.as_str()),
                            ("name", name),
                            ("desc", desc),
                        ])
                        .send()
                        .await,
                    "Trello API",
                )
                .await
            }
            _ => "エラー: action は list_cards または create_card を指定してください。".to_string(),
        }
    }
}

async fn handle_text_response(
    response: Result<reqwest::Response, reqwest::Error>,
    label: &str,
) -> String {
    let response = match response {
        Ok(response) => response,
        Err(error) => return format!("エラー: {} への接続に失敗しました: {}", label, error),
    };
    let status = response.status();
    let body = crate::tool_response_text!(response);
    if status.is_success() {
        body
    } else {
        format!("エラー: {} が HTTP {} を返しました。", label, status)
    }
}

inventory::submit! { ToolReg { make: || Box::new(TrelloTool) } }
