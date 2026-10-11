// skills/misskey/mod.rs - Misskey API のノート投稿・タイムライン取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::tools::social_post::text_arg;

const MAX_TEXT_CHARS: usize = 3000;

const VISIBILITIES: &[&str] = &["public", "home", "followers", "specified"];

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
                "visibility": { "type": "string", "enum": VISIBILITIES, "description": "公開範囲。省略時 public。" },
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
        let request_timeout = Duration::from_secs(timeout);
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "create_note" => {
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                let visibility = crate::tool_try!(crate::tools::social_post::visibility(
                    &args,
                    VISIBILITIES,
                    "public"
                ));
                crate::tools::http::service_reply(
                    cfg,
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::POST,
                        format!("{}/api/notes/create", base),
                        request_timeout
                    ))
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
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::POST,
                        format!("{}/api/notes/timeline", base),
                        request_timeout
                    ))
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
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::POST,
                        format!("{}/api/users/search", base),
                        request_timeout
                    ))
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

inventory::submit! { ToolReg { make: || Box::new(MisskeyTool) } }
