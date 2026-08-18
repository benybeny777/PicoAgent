// skills/jira/mod.rs - Jira Cloud の課題検索・作成スキル。

use std::time::Duration;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct JiraTool;

fn auth_header(email: &str, token: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{}:{}", email, token)))
}

#[async_trait]
impl Tool for JiraTool {
    fn name(&self) -> &'static str {
        "jira"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["jira"]
    }

    fn label(&self) -> &'static str {
        "Jira"
    }

    fn description(&self) -> String {
        "Jira Cloud REST API で課題検索・作成を行う。設定に base URL、email、API token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["search", "create_issue"], "description": "search/create_issue。" },
                "jql": { "type": "string", "description": "search の JQL。" },
                "project_key": { "type": "string", "description": "create_issue の project key。省略時は設定値。" },
                "issue_type": { "type": "string", "description": "Issue type。省略時 Task。" },
                "summary": { "type": "string", "description": "課題件名。" },
                "description": { "type": "string", "description": "課題説明。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_JIRA_BASE_URL", "Jira のベース URL", "text", "https://example.atlassian.net"),
            Field::new("PICOAGENT_JIRA_EMAIL", "Jira のアカウントメールアドレス", "text", ""),
            Field::new("PICOAGENT_JIRA_API_TOKEN", "Jira の API トークン", "password", ""),
            Field::new("PICOAGENT_JIRA_PROJECT_KEY", "Jira 既定 project key", "text", ""),
            Field::new("PICOAGENT_JIRA_TIMEOUT_SECS", "Jira API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base = cfg.get("PICOAGENT_JIRA_BASE_URL", "").trim_end_matches('/').to_string();
        let email = cfg.get("PICOAGENT_JIRA_EMAIL", "");
        let token = cfg.get("PICOAGENT_JIRA_API_TOKEN", "");
        if base.is_empty() || email.is_empty() || token.is_empty() {
            return super::config_guide("Jira", &["PICOAGENT_JIRA_BASE_URL", "PICOAGENT_JIRA_EMAIL", "PICOAGENT_JIRA_API_TOKEN"], "Jira Cloud のサイトURL、メールアドレス、API token を入れてください。");
        }
        let timeout = cfg
            .get("PICOAGENT_JIRA_TIMEOUT_SECS", "20")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Jira tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Jira API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "search" => {
                let jql = args.get("jql").and_then(Value::as_str).unwrap_or("ORDER BY updated DESC");
                handle_text_response(
                    client
                        .get(format!("{}/rest/api/3/search", base))
                        .header("Authorization", auth_header(&email, &token))
                        .query(&[("jql", jql), ("maxResults", "10")])
                        .send()
                        .await,
                    "Jira API",
                )
                .await
            }
            "create_issue" => {
                let project_key = args
                    .get("project_key")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_JIRA_PROJECT_KEY", ""));
                let Some(summary) = args.get("summary").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
                    return "エラー: summary が指定されていません。".to_string();
                };
                if project_key.is_empty() {
                    return super::config_guide("Jira", &["PICOAGENT_JIRA_PROJECT_KEY"], "依頼で project_key を渡すか、既定のプロジェクトキーを設定してください。");
                }
                let issue_type = args.get("issue_type").and_then(Value::as_str).unwrap_or("Task");
                let description = args.get("description").and_then(Value::as_str).unwrap_or("");
                let payload = json!({
                    "fields": {
                        "project": { "key": project_key },
                        "summary": summary,
                        "issuetype": { "name": issue_type },
                        "description": {
                            "type": "doc",
                            "version": 1,
                            "content": [{
                                "type": "paragraph",
                                "content": [{ "type": "text", "text": description }]
                            }]
                        }
                    }
                });
                handle_text_response(
                    client
                        .post(format!("{}/rest/api/3/issue", base))
                        .header("Authorization", auth_header(&email, &token))
                        .json(&payload)
                        .send()
                        .await,
                    "Jira API",
                )
                .await
            }
            _ => "エラー: action は search または create_issue を指定してください。".to_string(),
        }
    }
}

async fn handle_text_response(response: Result<reqwest::Response, reqwest::Error>, label: &str) -> String {
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

inventory::submit! { ToolReg { make: || Box::new(JiraTool) } }
