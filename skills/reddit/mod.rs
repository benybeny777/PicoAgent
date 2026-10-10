// skills/reddit/mod.rs - Reddit OAuth API の投稿・取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::tools::social_post::text_arg;

const REDDIT_BASE: &str = "https://oauth.reddit.com";
const MAX_TITLE_CHARS: usize = 300;
const MAX_TEXT_CHARS: usize = 40_000;

pub struct RedditTool;

#[async_trait]
impl Tool for RedditTool {
    fn name(&self) -> &'static str {
        "reddit"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["reddit"]
    }

    fn label(&self) -> &'static str {
        "Reddit"
    }

    fn description(&self) -> String {
        "Reddit OAuth API でサブレディット投稿、hot一覧取得、コメント投稿を行う。OAuth bearer token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["submit_text", "subreddit_hot", "comment"], "description": "submit_text/subreddit_hot/comment。" },
                "subreddit": { "type": "string", "description": "サブレディット名。省略時は設定値。" },
                "title": { "type": "string", "description": "submit_text のタイトル。" },
                "text": { "type": "string", "description": "投稿本文またはコメント本文。" },
                "thing_id": { "type": "string", "description": "comment の対象 fullname。例: t3_xxx。" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "取得件数。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_REDDIT_BEARER_TOKEN",
                "Reddit の OAuth bearer トークン",
                "password",
                "",
            ),
            Field::new(
                "PICOAGENT_REDDIT_USER_AGENT",
                "Reddit の User-Agent",
                "text",
                "PicoAgent/1.0",
            ),
            Field::new(
                "PICOAGENT_REDDIT_SUBREDDIT",
                "Reddit 既定 subreddit",
                "text",
                "",
            ),
            Field::int(&crate::config::ints::REDDIT_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_REDDIT_BEARER_TOKEN", "");
        if token.is_empty() {
            return super::config_guide(
                "Reddit",
                &["PICOAGENT_REDDIT_BEARER_TOKEN"],
                "Reddit OAuth bearer token が必要です。",
            );
        }
        let timeout = crate::tool_try!(cfg.int(&crate::config::ints::REDDIT_TIMEOUT_SECS)) as u64;
        let request_timeout = Duration::from_secs(timeout);
        let user_agent = cfg.get("PICOAGENT_REDDIT_USER_AGENT", "PicoAgent/1.0");
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "submit_text" => {
                let subreddit = subreddit(cfg, &args);
                if subreddit.is_empty() {
                    return super::config_guide(
                        "Reddit",
                        &["PICOAGENT_REDDIT_SUBREDDIT"],
                        "依頼で subreddit を渡すか、既定の subreddit を設定してください。",
                    );
                }
                let title = match text_arg(&args, "title", MAX_TITLE_CHARS) {
                    Ok(title) => title,
                    Err(error) => return format!("エラー: {}", error),
                };
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                crate::tools::http::service_reply(
                    cfg,
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::POST,
                        format!("{}/api/submit", REDDIT_BASE),
                        request_timeout
                    ))
                    .header(reqwest::header::USER_AGENT, user_agent.as_str())
                    .bearer_auth(token)
                    .form(&[
                        ("api_type", "json"),
                        ("kind", "self"),
                        ("sr", subreddit.as_str()),
                        ("title", title),
                        ("text", text),
                    ])
                    .send()
                    .await,
                    "Reddit API",
                )
                .await
            }
            "subreddit_hot" => {
                let subreddit = subreddit(cfg, &args);
                if subreddit.is_empty() {
                    return super::config_guide(
                        "Reddit",
                        &["PICOAGENT_REDDIT_SUBREDDIT"],
                        "依頼で subreddit を渡すか、既定の subreddit を設定してください。",
                    );
                }
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .filter(|value| (1..=100).contains(value))
                    .unwrap_or(25)
                    .to_string();
                crate::tools::http::service_reply(
                    cfg,
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::GET,
                        format!("{}/r/{}/hot", REDDIT_BASE, subreddit),
                        request_timeout
                    ))
                    .header(reqwest::header::USER_AGENT, user_agent.as_str())
                    .bearer_auth(token)
                    .query(&[("limit", limit.as_str())])
                    .send()
                    .await,
                    "Reddit API",
                )
                .await
            }
            "comment" => {
                let thing_id = match text_arg(&args, "thing_id", 100) {
                    Ok(thing_id) => thing_id,
                    Err(error) => return format!("エラー: {}", error),
                };
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                crate::tools::http::service_reply(
                    cfg,
                    crate::tool_try!(crate::tools::http::service(
                        reqwest::Method::POST,
                        format!("{}/api/comment", REDDIT_BASE),
                        request_timeout
                    ))
                    .header(reqwest::header::USER_AGENT, user_agent.as_str())
                    .bearer_auth(token)
                    .form(&[("api_type", "json"), ("thing_id", thing_id), ("text", text)])
                    .send()
                    .await,
                    "Reddit API",
                )
                .await
            }
            _ => {
                "エラー: action は submit_text/subreddit_hot/comment のいずれかを指定してください。"
                    .to_string()
            }
        }
    }
}

fn subreddit(cfg: &Config, args: &Value) -> String {
    args.get("subreddit")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| cfg.get("PICOAGENT_REDDIT_SUBREDDIT", ""))
}

inventory::submit! { ToolReg { make: || Box::new(RedditTool) } }
