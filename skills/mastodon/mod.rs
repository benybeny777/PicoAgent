// skills/mastodon/mod.rs - Mastodon API の投稿・タイムライン・検索スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::tools::social_post::text_arg;

const MAX_STATUS_CHARS: usize = 5000;

const VISIBILITIES: &[&str] = &["public", "unlisted", "private", "direct"];

pub struct MastodonTool;

#[async_trait]
impl Tool for MastodonTool {
    fn name(&self) -> &'static str {
        "mastodon"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["mastodon", "マストドン"]
    }

    fn label(&self) -> &'static str {
        "Mastodon"
    }

    fn description(&self) -> String {
        "Mastodon API で投稿、ホームタイムライン取得、検索を行う。インスタンスURLと access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["post", "home_timeline", "search"], "description": "post/home_timeline/search。" },
                "status": { "type": "string", "description": "post の本文。" },
                "visibility": { "type": "string", "enum": VISIBILITIES, "description": "公開範囲。省略時 public。" },
                "query": { "type": "string", "description": "search の検索語。" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 40, "description": "取得件数。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_MASTODON_BASE_URL",
                "Mastodon のインスタンス URL",
                "text",
                "https://mastodon.social",
            ),
            Field::new(
                "PICOAGENT_MASTODON_ACCESS_TOKEN",
                "Mastodon のアクセストークン",
                "password",
                "",
            ),
            Field::int(&crate::config::ints::MASTODON_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_MASTODON_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide(
                "Mastodon",
                &["PICOAGENT_MASTODON_ACCESS_TOKEN"],
                "投稿やホームタイムライン取得に使う Mastodon access token が必要です。",
            );
        }
        let base = cfg
            .get("PICOAGENT_MASTODON_BASE_URL", "https://mastodon.social")
            .trim_end_matches('/')
            .to_string();
        let timeout = crate::tool_try!(cfg.int(&crate::config::ints::MASTODON_TIMEOUT_SECS)) as u64;
        let request_timeout = Duration::from_secs(timeout);
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "post" => {
                let status = match text_arg(&args, "status", MAX_STATUS_CHARS) {
                    Ok(status) => status,
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
                        format!("{}/api/v1/statuses", base),
                        request_timeout
                    ))
                    .bearer_auth(token)
                    .form(&[("status", status), ("visibility", visibility)])
                    .send()
                    .await,
                    "Mastodon API",
                )
                .await
            }
            "home_timeline" => {
                let limit = limit_arg(&args, 20, 40).to_string();
                crate::tools::http::service_reply(
                    cfg,
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::GET,
                        format!("{}/api/v1/timelines/home", base),
                        request_timeout
                    ))
                    .bearer_auth(token)
                    .query(&[("limit", limit.as_str())])
                    .send()
                    .await,
                    "Mastodon API",
                )
                .await
            }
            "search" => {
                let query = match text_arg(&args, "query", 512) {
                    Ok(query) => query,
                    Err(error) => return format!("エラー: {}", error),
                };
                let limit = limit_arg(&args, 20, 40).to_string();
                crate::tools::http::service_reply(
                    cfg,
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::GET,
                        format!("{}/api/v2/search", base),
                        request_timeout
                    ))
                    .bearer_auth(token)
                    .query(&[("q", query), ("limit", limit.as_str())])
                    .send()
                    .await,
                    "Mastodon API",
                )
                .await
            }
            _ => "エラー: action は post/home_timeline/search のいずれかを指定してください。"
                .to_string(),
        }
    }
}

fn limit_arg(args: &Value, fallback: u64, max: u64) -> u64 {
    args.get("limit")
        .and_then(Value::as_u64)
        .filter(|value| (1..=max).contains(value))
        .unwrap_or(fallback)
}

inventory::submit! { ToolReg { make: || Box::new(MastodonTool) } }
