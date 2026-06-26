// tools/x_twitter.rs - X API v2 の投稿・検索スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const X_API_BASE: &str = "https://api.x.com/2";
const MAX_TEXT_CHARS: usize = 280;

pub struct XTwitterTool;

#[async_trait]
impl Tool for XTwitterTool {
    fn name(&self) -> &'static str {
        "x_twitter"
    }

    fn label(&self) -> &'static str {
        "X/Twitter"
    }

    fn description(&self) -> String {
        "X API v2 でポスト投稿、最近のポスト検索、自分のポスト取得を行う。OAuth2 user-context bearer token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["post", "search_recent", "user_tweets"], "description": "post/search_recent/user_tweets。" },
                "text": { "type": "string", "description": "post の本文。" },
                "query": { "type": "string", "description": "search_recent の検索クエリ。" },
                "user_id": { "type": "string", "description": "user_tweets のユーザーID。省略時は設定値。" },
                "max_results": { "type": "integer", "minimum": 10, "maximum": 100, "description": "取得件数。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_X_BEARER_TOKEN", "X OAuth2 bearer token", "password", ""),
            Field::new("PICOAGENT_X_USER_ID", "X 既定 user ID", "text", ""),
            Field::new("PICOAGENT_X_TIMEOUT_SECS", "X API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_X_BEARER_TOKEN", "");
        if token.is_empty() {
            return "エラー: PICOAGENT_X_BEARER_TOKEN が未設定です。".to_string();
        }
        let timeout = timeout_secs(cfg, "PICOAGENT_X_TIMEOUT_SECS", 20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (X tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: X API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "post" => {
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                handle_text_response(
                    client
                        .post(format!("{}/tweets", X_API_BASE))
                        .bearer_auth(token)
                        .json(&json!({ "text": text }))
                        .send()
                        .await,
                    "X API",
                )
                .await
            }
            "search_recent" => {
                let query = match text_arg(&args, "query", 512) {
                    Ok(query) => query,
                    Err(error) => return format!("エラー: {}", error),
                };
                let max_results = max_results(&args, 10, 100, 10).to_string();
                handle_text_response(
                    client
                        .get(format!("{}/tweets/search/recent", X_API_BASE))
                        .bearer_auth(token)
                        .query(&[("query", query), ("max_results", max_results.as_str())])
                        .send()
                        .await,
                    "X API",
                )
                .await
            }
            "user_tweets" => {
                let user_id = args
                    .get("user_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_X_USER_ID", ""));
                if user_id.is_empty() {
                    return "エラー: user_id または PICOAGENT_X_USER_ID が未設定です。".to_string();
                }
                let max_results = max_results(&args, 5, 100, 10).to_string();
                handle_text_response(
                    client
                        .get(format!("{}/users/{}/tweets", X_API_BASE, user_id))
                        .bearer_auth(token)
                        .query(&[("max_results", max_results.as_str())])
                        .send()
                        .await,
                    "X API",
                )
                .await
            }
            _ => "エラー: action は post/search_recent/user_tweets のいずれかを指定してください。".to_string(),
        }
    }
}

fn timeout_secs(cfg: &Config, key: &str, fallback: u64) -> u64 {
    cfg.get(key, &fallback.to_string())
        .parse::<u64>()
        .ok()
        .filter(|value| (3..=60).contains(value))
        .unwrap_or(fallback)
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

fn max_results(args: &Value, min: u64, max: u64, fallback: u64) -> u64 {
    args.get("max_results")
        .and_then(Value::as_u64)
        .filter(|value| (min..=max).contains(value))
        .unwrap_or(fallback)
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

inventory::submit! { ToolReg { make: || Box::new(XTwitterTool) } }
