// skills/linear/mod.rs - Linear GraphQL API の Issue 検索・作成スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const LINEAR_URL: &str = "https://api.linear.app/graphql";

pub struct LinearTool;

#[async_trait]
impl Tool for LinearTool {
    fn name(&self) -> &'static str {
        "linear"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["linear"]
    }

    fn label(&self) -> &'static str {
        "Linear"
    }

    fn description(&self) -> String {
        "Linear GraphQL API で Issue 検索・作成を行う。設定に API key が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list_issues", "create_issue"], "description": "list_issues/create_issue。" },
                "team_id": { "type": "string", "description": "create_issue の team ID。省略時は設定値。" },
                "title": { "type": "string", "description": "Issue title。" },
                "description": { "type": "string", "description": "Issue description。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_LINEAR_API_KEY",
                "Linear の API キー",
                "password",
                "",
            ),
            Field::new(
                "PICOAGENT_LINEAR_TEAM_ID",
                "Linear 既定 team ID",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_LINEAR_TIMEOUT_SECS",
                "Linear API 待ち時間（秒、3〜60）",
                "text",
                "20",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_LINEAR_API_KEY", "");
        if token.is_empty() {
            return super::config_guide(
                "Linear",
                &["PICOAGENT_LINEAR_API_KEY"],
                "Linear の API key を入れてください。",
            );
        }
        let timeout = cfg
            .get("PICOAGENT_LINEAR_TIMEOUT_SECS", "20")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Linear tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Linear API の準備に失敗しました: {}", error),
        };
        let payload = match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "list_issues" => json!({
                "query": "query { issues(first: 10, orderBy: updatedAt) { nodes { identifier title url state { name } } } }"
            }),
            "create_issue" => {
                let team_id = args
                    .get("team_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_LINEAR_TEAM_ID", ""));
                let Some(title) = args
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                else {
                    return "エラー: title が指定されていません。".to_string();
                };
                if team_id.is_empty() {
                    return super::config_guide(
                        "Linear",
                        &["PICOAGENT_LINEAR_TEAM_ID"],
                        "依頼で team_id を渡すか、既定の team ID を設定してください。",
                    );
                }
                json!({
                    "query": "mutation IssueCreate($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { identifier title url } } }",
                    "variables": {
                        "input": {
                            "teamId": team_id,
                            "title": title,
                            "description": args.get("description").and_then(Value::as_str).unwrap_or("")
                        }
                    }
                })
            }
            _ => {
                return "エラー: action は list_issues または create_issue を指定してください。"
                    .to_string()
            }
        };
        crate::tools::http::service_reply(
            cfg,
            client
                .post(LINEAR_URL)
                .bearer_auth(token)
                .json(&payload)
                .send()
                .await,
            "Linear API",
        )
        .await
    }
}

inventory::submit! { ToolReg { make: || Box::new(LinearTool) } }
