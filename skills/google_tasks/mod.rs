// skills/google_tasks/mod.rs - Google Tasks のタスク一覧・作成・完了スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const TASKS_BASE: &str = "https://tasks.googleapis.com/tasks/v1";

pub struct GoogleTasksTool;

#[async_trait]
impl Tool for GoogleTasksTool {
    fn name(&self) -> &'static str {
        "google_tasks"
    }

    fn label(&self) -> &'static str {
        "Google Tasks"
    }

    fn description(&self) -> String {
        "Google Tasks API でタスクリスト取得、タスク一覧、作成、完了を行う。設定に OAuth access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list_tasklists", "list", "create", "complete"], "description": "list_tasklists/list/create/complete。" },
                "tasklist_id": { "type": "string", "description": "対象タスクリストID。省略時は設定値または @default。" },
                "task_id": { "type": "string", "description": "complete のタスクID。" },
                "title": { "type": "string", "description": "create のタスク名。" },
                "notes": { "type": "string", "description": "create のメモ。" },
                "due": { "type": "string", "description": "create の期限 RFC3339。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_GOOGLE_TASKS_ACCESS_TOKEN", "Google Tasks OAuth access token", "password", "ya29..."),
            Field::new("PICOAGENT_GOOGLE_TASKS_LIST_ID", "Google Tasks 既定 tasklist ID", "text", "@default"),
            Field::new("PICOAGENT_GOOGLE_TASKS_TIMEOUT_SECS", "Google Tasks API 待ち時間（秒、3〜60）", "text", "15"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_GOOGLE_TASKS_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide("Google Tasks", &["PICOAGENT_GOOGLE_TASKS_ACCESS_TOKEN"], "Google Tasks API を使える OAuth access token が必要です。");
        }
        let timeout = cfg
            .get("PICOAGENT_GOOGLE_TASKS_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Google Tasks tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Google Tasks API の準備に失敗しました: {}", error),
        };
        let tasklist_id = args
            .get("tasklist_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_GOOGLE_TASKS_LIST_ID", "@default"));
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "list_tasklists" => {
                handle_text_response(
                    client
                        .get(format!("{}/users/@me/lists", TASKS_BASE))
                        .bearer_auth(token)
                        .send()
                        .await,
                    "Google Tasks API",
                )
                .await
            }
            "list" => {
                handle_text_response(
                    client
                        .get(format!("{}/lists/{}/tasks", TASKS_BASE, tasklist_id))
                        .bearer_auth(token)
                        .send()
                        .await,
                    "Google Tasks API",
                )
                .await
            }
            "create" => {
                let title = match args.get("title").and_then(Value::as_str).map(str::trim) {
                    Some(title) if !title.is_empty() => title,
                    _ => return "エラー: title が指定されていません。".to_string(),
                };
                let mut payload = json!({ "title": title });
                if let Some(notes) = args.get("notes").and_then(Value::as_str) {
                    payload["notes"] = json!(notes);
                }
                if let Some(due) = args.get("due").and_then(Value::as_str) {
                    payload["due"] = json!(due);
                }
                handle_text_response(
                    client
                        .post(format!("{}/lists/{}/tasks", TASKS_BASE, tasklist_id))
                        .bearer_auth(token)
                        .json(&payload)
                        .send()
                        .await,
                    "Google Tasks API",
                )
                .await
            }
            "complete" => {
                let task_id = match args.get("task_id").and_then(Value::as_str).map(str::trim) {
                    Some(task_id) if !task_id.is_empty() => task_id,
                    _ => return "エラー: task_id が指定されていません。".to_string(),
                };
                handle_text_response(
                    client
                        .patch(format!("{}/lists/{}/tasks/{}", TASKS_BASE, tasklist_id, task_id))
                        .bearer_auth(token)
                        .json(&json!({ "status": "completed" }))
                        .send()
                        .await,
                    "Google Tasks API",
                )
                .await
            }
            _ => "エラー: action は list_tasklists/list/create/complete のいずれかを指定してください。".to_string(),
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

inventory::submit! { ToolReg { make: || Box::new(GoogleTasksTool) } }
