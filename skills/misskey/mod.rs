// skills/misskey/mod.rs - Misskey API のノート投稿・タイムライン取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_TEXT_CHARS: usize = 3000;

pub struct MisskeyTool;

#[async_trait]
impl Tool for MisskeyTool {
    fn name(&self) -> &'static str {
        "misskey"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["misskey", "ミスキー", "ノート"]
    }

    fn label(&self) -> &'static str {
        "Misskey"
    }

    fn description(&self) -> String {
        "Misskey API でノート投稿、タイムライン取得、ユーザー検索を行う。インスタンスURLとAPI tokenが必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["create_note", "timeline", "search_users"], "description": "create_note/timeline/search_users。" },
                "text": { "type": "string", "description": "create_note の本文。" },
                "visibility": { "type": "string", "enum": ["public", "home", "followers", "specified"], "description": "公開範囲。省略時 public。" },
                "query": { "type": "string", "description": "search_users の検索語。" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "取得件数。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_MISSKEY_BASE_URL",
                "Misskey のインスタンス URL",
                "text",
                "https://misskey.io",
            ),
            Field::new(
                "PICOAGENT_MISSKEY_TOKEN",
                "Misskey の API トークン",
                "password",
                "",
            ),
            Field::int(&crate::config::ints::MISSKEY_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_MISSKEY_TOKEN", "");
        if token.is_empty() {
            return super::config_guide(
                "Misskey",
                &["PICOAGENT_MISSKEY_TOKEN"],
                "Misskey API token が必要です。",
            );
        }
        let base = cfg
            .get("PICOAGENT_MISSKEY_BASE_URL", "https://misskey.io")
            .trim_end_matches('/')
            .to_string();
        let timeout = crate::tool_try!(cfg.int(&crate::config::ints::MISSKEY_TIMEOUT_SECS)) as u64;
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Misskey tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Misskey API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "create_note" => {
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                let visibility = args
                    .get("visibility")
                    .and_then(Value::as_str)
                    .unwrap_or("public");
                crate::tools::http::service_reply(
                    cfg,
                    client
                        .post(format!("{}/api/notes/create", base))
                        .json(&json!({ "i": token, "text": text, "visibility": visibility }))
                        .send()
                        .await,
                    "Misskey API",
                )
                .await
            }
            "timeline" => {
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .filter(|value| (1..=100).contains(value))
                    .unwrap_or(20);
                crate::tools::http::service_reply(
                    cfg,
                    client
                        .post(format!("{}/api/notes/timeline", base))
                        .json(&json!({ "i": token, "limit": limit }))
                        .send()
                        .await,
                    "Misskey API",
                )
                .await
            }
            "search_users" => {
                let query = match text_arg(&args, "query", 200) {
                    Ok(query) => query,
                    Err(error) => return format!("エラー: {}", error),
                };
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .filter(|value| (1..=100).contains(value))
                    .unwrap_or(20);
                crate::tools::http::service_reply(
                    cfg,
                    client
                        .post(format!("{}/api/users/search", base))
                        .json(&json!({ "i": token, "query": query, "limit": limit }))
                        .send()
                        .await,
                    "Misskey API",
                )
                .await
            }
            _ => {
                "エラー: action は create_note/timeline/search_users のいずれかを指定してください。"
                    .to_string()
            }
        }
    }
}

fn text_arg<'a>(args: &'a Value, key: &str, max_chars: usize) -> Result<&'a str, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} が指定されていません。", key))?;
    if text.chars().count() > max_chars {
        return Err(format!(
            "{} は {} 文字以内で指定してください。",
            key, max_chars
        ));
    }
    Ok(text)
}

inventory::submit! { ToolReg { make: || Box::new(MisskeyTool) } }
