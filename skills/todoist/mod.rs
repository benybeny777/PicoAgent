// skills/todoist/mod.rs - Todoist のタスク一覧・作成・完了スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const TODOIST_BASE: &str = "https://api.todoist.com/rest/v2";
const MAX_CONTENT_CHARS: usize = 1000;

pub struct TodoistTool;

#[async_trait]
impl Tool for TodoistTool {
    fn name(&self) -> &'static str {
        "todoist"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["todoist"]
    }

    fn label(&self) -> &'static str {
        "Todoist"
    }

    fn description(&self) -> String {
        "Todoist API でタスク一覧取得、作成、完了を行う。設定に API token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "create", "close"], "description": "list/create/close。" },
                "content": { "type": "string", "description": "create のタスク名。" },
                "description": { "type": "string", "description": "create の説明。" },
                "project_id": { "type": "string", "description": "対象プロジェクトID。" },
                "due_string": { "type": "string", "description": "自然言語の期限。例: tomorrow 10:00" },
                "task_id": { "type": "string", "description": "close のタスクID。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_TODOIST_TOKEN",
                "Todoist の API トークン",
                "password",
                "",
            ),
            Field::new(
                "PICOAGENT_TODOIST_PROJECT_ID",
                "Todoist 既定 project ID",
                "text",
                "",
            ),
            Field::int(&crate::config::ints::TODOIST_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_TODOIST_TOKEN", "");
        if token.is_empty() {
            return super::config_guide(
                "Todoist",
                &["PICOAGENT_TODOIST_TOKEN"],
                "Todoist API token が必要です。",
            );
        }
        let timeout = crate::tool_try!(cfg.int(&crate::config::ints::TODOIST_TIMEOUT_SECS)) as u64;
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Todoist tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Todoist API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "list" => {
                let mut request = client
                    .get(format!("{}/tasks", TODOIST_BASE))
                    .bearer_auth(token);
                let project_id = args
                    .get("project_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_TODOIST_PROJECT_ID", ""));
                if !project_id.is_empty() {
                    request = request.query(&[("project_id", project_id)]);
                }
                crate::tools::http::service_reply(cfg, request.send().await, "Todoist API").await
            }
            "create" => {
                let Some(content) = args
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                else {
                    return "エラー: content が指定されていません。".to_string();
                };
                if content.chars().count() > MAX_CONTENT_CHARS {
                    return format!(
                        "エラー: content は {} 文字以内で指定してください。",
                        MAX_CONTENT_CHARS
                    );
                }
                let mut payload = json!({ "content": content });
                for key in ["description", "due_string", "project_id"] {
                    if let Some(value) = args
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                    {
                        payload[key] = json!(value);
                    }
                }
                if payload.get("project_id").is_none() {
                    let project_id = cfg.get("PICOAGENT_TODOIST_PROJECT_ID", "");
                    if !project_id.is_empty() {
                        payload["project_id"] = json!(project_id);
                    }
                }
                crate::tools::http::service_reply(
                    cfg,
                    client
                        .post(format!("{}/tasks", TODOIST_BASE))
                        .bearer_auth(token)
                        .json(&payload)
                        .send()
                        .await,
                    "Todoist API",
                )
                .await
            }
            "close" => {
                let Some(task_id) = args
                    .get("task_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                else {
                    return "エラー: task_id が指定されていません。".to_string();
                };
                crate::tools::http::service_reply(
                    cfg,
                    client
                        .post(format!("{}/tasks/{}/close", TODOIST_BASE, task_id))
                        .bearer_auth(token)
                        .send()
                        .await,
                    "Todoist API",
                )
                .await
            }
            _ => "エラー: action は list/create/close のいずれかを指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(TodoistTool) } }
