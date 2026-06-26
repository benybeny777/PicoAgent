// skills/mastodon/mod.rs - Mastodon API の投稿・タイムライン・検索スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_STATUS_CHARS: usize = 5000;

pub struct MastodonTool;

#[async_trait]
impl Tool for MastodonTool {
    fn name(&self) -> &'static str {
        "mastodon"
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
                "visibility": { "type": "string", "enum": ["public", "unlisted", "private", "direct"], "description": "公開範囲。省略時 public。" },
                "query": { "type": "string", "description": "search の検索語。" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 40, "description": "取得件数。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_MASTODON_BASE_URL", "Mastodon instance URL", "text", "https://mastodon.social"),
            Field::new("PICOAGENT_MASTODON_ACCESS_TOKEN", "Mastodon access token", "password", ""),
            Field::new("PICOAGENT_MASTODON_TIMEOUT_SECS", "Mastodon API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_MASTODON_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide("Mastodon", &["PICOAGENT_MASTODON_ACCESS_TOKEN"], "投稿やホームタイムライン取得に使う Mastodon access token が必要です。");
        }
        let base = cfg
            .get("PICOAGENT_MASTODON_BASE_URL", "https://mastodon.social")
            .trim_end_matches('/')
            .to_string();
        let timeout = cfg
            .get("PICOAGENT_MASTODON_TIMEOUT_SECS", "20")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Mastodon tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Mastodon API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "post" => {
                let status = match text_arg(&args, "status", MAX_STATUS_CHARS) {
                    Ok(status) => status,
                    Err(error) => return format!("エラー: {}", error),
                };
                let visibility = args.get("visibility").and_then(Value::as_str).unwrap_or("public");
                handle_text_response(
                    client
                        .post(format!("{}/api/v1/statuses", base))
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
                handle_text_response(
                    client
                        .get(format!("{}/api/v1/timelines/home", base))
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
                handle_text_response(
                    client
                        .get(format!("{}/api/v2/search", base))
                        .bearer_auth(token)
                        .query(&[("q", query), ("limit", limit.as_str())])
                        .send()
                        .await,
                    "Mastodon API",
                )
                .await
            }
            _ => "エラー: action は post/home_timeline/search のいずれかを指定してください。".to_string(),
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
        return Err(format!("{} は {} 文字以内で指定してください。", key, max_chars));
    }
    Ok(text)
}

fn limit_arg(args: &Value, fallback: u64, max: u64) -> u64 {
    args.get("limit")
        .and_then(Value::as_u64)
        .filter(|value| (1..=max).contains(value))
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

inventory::submit! { ToolReg { make: || Box::new(MastodonTool) } }
