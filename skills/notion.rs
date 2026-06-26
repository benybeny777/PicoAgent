// tools/notion.rs - Notion の検索・データベースページ作成スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const NOTION_BASE: &str = "https://api.notion.com/v1";
const NOTION_VERSION: &str = "2022-06-28";
const MAX_TITLE_CHARS: usize = 300;
const MAX_CONTENT_CHARS: usize = 2000;

pub struct NotionTool;

fn timeout_secs(cfg: &Config) -> u64 {
    cfg.get("PICOAGENT_NOTION_TIMEOUT_SECS", "20")
        .parse::<u64>()
        .ok()
        .filter(|value| (3..=60).contains(value))
        .unwrap_or(20)
}

fn notion_post(client: &reqwest::Client, token: String, url: String) -> reqwest::RequestBuilder {
    client
        .post(url)
        .bearer_auth(token)
        .header("Notion-Version", NOTION_VERSION)
}

#[async_trait]
impl Tool for NotionTool {
    fn name(&self) -> &'static str {
        "notion"
    }

    fn label(&self) -> &'static str {
        "Notion"
    }

    fn description(&self) -> String {
        "Notion API でページ検索またはデータベースへのページ作成を行う。設定に integration token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["search", "create_page"], "description": "search はページ検索、create_page はDBへページ作成。" },
                "query": { "type": "string", "description": "search の検索語。" },
                "database_id": { "type": "string", "description": "create_page のDB ID。省略時は設定値。" },
                "title": { "type": "string", "description": "create_page のタイトル。" },
                "content": { "type": "string", "description": "create_page の本文（段落）。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_NOTION_TOKEN", "Notion integration token", "password", "secret_..."),
            Field::new("PICOAGENT_NOTION_DATABASE_ID", "Notion 既定 database ID", "text", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
            Field::new("PICOAGENT_NOTION_TIMEOUT_SECS", "Notion API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_NOTION_TOKEN", "");
        if token.is_empty() {
            return "エラー: PICOAGENT_NOTION_TOKEN が未設定です。".to_string();
        }
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs(cfg)))
            .user_agent("PicoAgent/1.0 (Notion tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Notion API の準備に失敗しました: {}", error),
        };
        match action {
            "search" => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or("").trim();
                let response = match notion_post(&client, token, format!("{}/search", NOTION_BASE))
                    .json(&json!({ "query": query, "page_size": 10 }))
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => return format!("エラー: Notion API への接続に失敗しました: {}", error),
                };
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    return format!("エラー: Notion API が HTTP {} を返しました。", status);
                }
                let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
                let mut out = String::from("Notion検索結果:\n");
                for item in parsed.get("results").and_then(Value::as_array).into_iter().flatten() {
                    let title = item
                        .get("properties")
                        .and_then(Value::as_object)
                        .and_then(|props| props.values().find_map(|prop| prop.get("title")))
                        .and_then(Value::as_array)
                        .and_then(|parts| parts.first())
                        .and_then(|part| part.get("plain_text"))
                        .and_then(Value::as_str)
                        .unwrap_or("（無題）");
                    out.push_str(&format!(
                        "- {} ({})\n",
                        title,
                        item.get("url").and_then(Value::as_str).unwrap_or("")
                    ));
                }
                out
            }
            "create_page" => {
                let database_id = args
                    .get("database_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_NOTION_DATABASE_ID", ""));
                if database_id.is_empty() {
                    return "エラー: database_id または PICOAGENT_NOTION_DATABASE_ID が未設定です。".to_string();
                }
                let title = match args.get("title").and_then(Value::as_str).map(str::trim) {
                    Some(title) if !title.is_empty() => title,
                    _ => return "エラー: title が指定されていません。".to_string(),
                };
                if title.chars().count() > MAX_TITLE_CHARS {
                    return format!("エラー: title は {} 文字以内で指定してください。", MAX_TITLE_CHARS);
                }
                let content = args.get("content").and_then(Value::as_str).unwrap_or("");
                if content.chars().count() > MAX_CONTENT_CHARS {
                    return format!("エラー: content は {} 文字以内で指定してください。", MAX_CONTENT_CHARS);
                }
                let payload = json!({
                    "parent": { "database_id": database_id },
                    "properties": {
                        "Name": {
                            "title": [{ "text": { "content": title } }]
                        }
                    },
                    "children": [{
                        "object": "block",
                        "type": "paragraph",
                        "paragraph": {
                            "rich_text": [{ "type": "text", "text": { "content": content } }]
                        }
                    }]
                });
                let response = match notion_post(&client, token, format!("{}/pages", NOTION_BASE))
                    .json(&payload)
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => return format!("エラー: Notion API への接続に失敗しました: {}", error),
                };
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    return format!("エラー: Notion API が HTTP {} を返しました。", status);
                }
                let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
                format!(
                    "OK: Notionページを作成しました。{}",
                    parsed.get("url").and_then(Value::as_str).unwrap_or("")
                )
            }
            _ => "エラー: action は search または create_page を指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(NotionTool) } }
