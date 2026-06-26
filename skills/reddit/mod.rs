// tools/reddit.rs - Reddit OAuth API の投稿・取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const REDDIT_BASE: &str = "https://oauth.reddit.com";
const MAX_TITLE_CHARS: usize = 300;
const MAX_TEXT_CHARS: usize = 40_000;

pub struct RedditTool;

#[async_trait]
impl Tool for RedditTool {
    fn name(&self) -> &'static str {
        "reddit"
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
            Field::new("PICOAGENT_REDDIT_BEARER_TOKEN", "Reddit OAuth bearer token", "password", ""),
            Field::new("PICOAGENT_REDDIT_USER_AGENT", "Reddit User-Agent", "text", "PicoAgent/1.0"),
            Field::new("PICOAGENT_REDDIT_SUBREDDIT", "Reddit 既定 subreddit", "text", ""),
            Field::new("PICOAGENT_REDDIT_TIMEOUT_SECS", "Reddit API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_REDDIT_BEARER_TOKEN", "");
        if token.is_empty() {
            return "エラー: PICOAGENT_REDDIT_BEARER_TOKEN が未設定です。".to_string();
        }
        let timeout = cfg
            .get("PICOAGENT_REDDIT_TIMEOUT_SECS", "20")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent(cfg.get("PICOAGENT_REDDIT_USER_AGENT", "PicoAgent/1.0"))
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Reddit API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "submit_text" => {
                let subreddit = subreddit(cfg, &args);
                if subreddit.is_empty() {
                    return "エラー: subreddit または PICOAGENT_REDDIT_SUBREDDIT が未設定です。".to_string();
                }
                let title = match text_arg(&args, "title", MAX_TITLE_CHARS) {
                    Ok(title) => title,
                    Err(error) => return format!("エラー: {}", error),
                };
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                handle_text_response(
                    client
                        .post(format!("{}/api/submit", REDDIT_BASE))
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
                    return "エラー: subreddit または PICOAGENT_REDDIT_SUBREDDIT が未設定です。".to_string();
                }
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .filter(|value| (1..=100).contains(value))
                    .unwrap_or(25)
                    .to_string();
                handle_text_response(
                    client
                        .get(format!("{}/r/{}/hot", REDDIT_BASE, subreddit))
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
                handle_text_response(
                    client
                        .post(format!("{}/api/comment", REDDIT_BASE))
                        .bearer_auth(token)
                        .form(&[("api_type", "json"), ("thing_id", thing_id), ("text", text)])
                        .send()
                        .await,
                    "Reddit API",
                )
                .await
            }
            _ => "エラー: action は submit_text/subreddit_hot/comment のいずれかを指定してください。".to_string(),
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

fn text_arg<'a>(args: &'a Value, key: &str, max_chars: usize) -> Result<&'a str, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} が指定されていません。", key))?;
    if text.chars().count() > max_chars {
        return Err(format!("{} は {} 文字以内で指定してください。", key, max_chars));
    }
    Ok(text)
}

async fn handle_text_response(response: Result<reqwest::Response, reqwest::Error>, label: &str) -> String {
    let response = match response {
        Ok(response) => response,
        Err(error) => return format!("エラー: {} への接続に失敗しました: {}", label, error),
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_success() {
        body
    } else {
        format!("エラー: {} が HTTP {} を返しました。", label, status)
    }
}

inventory::submit! { ToolReg { make: || Box::new(RedditTool) } }
