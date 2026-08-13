// skills/bluesky/mod.rs - Bluesky AT Protocol の投稿・検索スキル。

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_TEXT_CHARS: usize = 300;

pub struct BlueskyTool;

#[async_trait]
impl Tool for BlueskyTool {
    fn name(&self) -> &'static str {
        "bluesky"
    }

    fn label(&self) -> &'static str {
        "Bluesky"
    }

    fn description(&self) -> String {
        "Bluesky AT Protocol で投稿、投稿検索、プロフィール取得を行う。identifier と app password が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["post", "search_posts", "get_profile"], "description": "post/search_posts/get_profile。" },
                "text": { "type": "string", "description": "post の本文。" },
                "query": { "type": "string", "description": "search_posts の検索語。" },
                "actor": { "type": "string", "description": "get_profile の handle または DID。省略時は設定の identifier。" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "取得件数。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_BLUESKY_BASE_URL", "Bluesky の PDS URL", "text", "https://bsky.social"),
            Field::new("PICOAGENT_BLUESKY_IDENTIFIER", "Bluesky の識別子（ハンドルまたはメール）", "text", "user.bsky.social"),
            Field::new("PICOAGENT_BLUESKY_APP_PASSWORD", "Bluesky のアプリパスワード", "password", ""),
            Field::new("PICOAGENT_BLUESKY_TIMEOUT_SECS", "Bluesky API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base = cfg
            .get("PICOAGENT_BLUESKY_BASE_URL", "https://bsky.social")
            .trim_end_matches('/')
            .to_string();
        let identifier = cfg.get("PICOAGENT_BLUESKY_IDENTIFIER", "");
        let password = cfg.get("PICOAGENT_BLUESKY_APP_PASSWORD", "");
        if identifier.is_empty() || password.is_empty() {
            return super::config_guide("Bluesky", &["PICOAGENT_BLUESKY_IDENTIFIER", "PICOAGENT_BLUESKY_APP_PASSWORD"], "Bluesky のハンドル/メールアドレスと app password を入れてください。");
        }
        let timeout = cfg
            .get("PICOAGENT_BLUESKY_TIMEOUT_SECS", "20")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Bluesky tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Bluesky API の準備に失敗しました: {}", error),
        };
        let session = match create_session(&client, &base, &identifier, &password).await {
            Ok(session) => session,
            Err(error) => return format!("エラー: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "post" => {
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                let payload = json!({
                    "repo": session.did,
                    "collection": "app.bsky.feed.post",
                    "record": {
                        "$type": "app.bsky.feed.post",
                        "text": text,
                        "createdAt": Utc::now().to_rfc3339()
                    }
                });
                handle_text_response(
                    client
                        .post(format!("{}/xrpc/com.atproto.repo.createRecord", base))
                        .bearer_auth(session.access_jwt)
                        .json(&payload)
                        .send()
                        .await,
                    "Bluesky API",
                )
                .await
            }
            "search_posts" => {
                let query = match text_arg(&args, "query", 512) {
                    Ok(query) => query,
                    Err(error) => return format!("エラー: {}", error),
                };
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .filter(|value| (1..=100).contains(value))
                    .unwrap_or(25)
                    .to_string();
                handle_text_response(
                    client
                        .get(format!("{}/xrpc/app.bsky.feed.searchPosts", base))
                        .bearer_auth(session.access_jwt)
                        .query(&[("q", query), ("limit", limit.as_str())])
                        .send()
                        .await,
                    "Bluesky API",
                )
                .await
            }
            "get_profile" => {
                let actor = args
                    .get("actor")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(&identifier);
                handle_text_response(
                    client
                        .get(format!("{}/xrpc/app.bsky.actor.getProfile", base))
                        .bearer_auth(session.access_jwt)
                        .query(&[("actor", actor)])
                        .send()
                        .await,
                    "Bluesky API",
                )
                .await
            }
            _ => "エラー: action は post/search_posts/get_profile のいずれかを指定してください。".to_string(),
        }
    }
}

struct BlueskySession {
    access_jwt: String,
    did: String,
}

async fn create_session(
    client: &reqwest::Client,
    base: &str,
    identifier: &str,
    password: &str,
) -> Result<BlueskySession, String> {
    let response = client
        .post(format!("{}/xrpc/com.atproto.server.createSession", base))
        .json(&json!({ "identifier": identifier, "password": password }))
        .send()
        .await
        .map_err(|error| format!("Bluesky セッション作成に失敗しました: {}", error))?;
    let status = response.status();
    let body = crate::tools::http::response_text(response).await?;
    if !status.is_success() {
        return Err(format!("Bluesky セッション作成が HTTP {} を返しました。", status));
    }
    let parsed: Value = serde_json::from_str(&body).map_err(|error| format!("Bluesky セッションを解析できません: {}", error))?;
    let access_jwt = parsed
        .get("accessJwt")
        .and_then(Value::as_str)
        .ok_or_else(|| "accessJwt を取得できません。".to_string())?
        .to_string();
    let did = parsed
        .get("did")
        .and_then(Value::as_str)
        .ok_or_else(|| "did を取得できません。".to_string())?
        .to_string();
    Ok(BlueskySession { access_jwt, did })
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
    let body = crate::tool_response_text!(response);
    if status.is_success() {
        body
    } else {
        format!("エラー: {} が HTTP {} を返しました。", label, status)
    }
}

inventory::submit! { ToolReg { make: || Box::new(BlueskyTool) } }
