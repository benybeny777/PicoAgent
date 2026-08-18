// skills/linkedin/mod.rs - LinkedIn API のプロフィール取得・UGC投稿スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const LINKEDIN_BASE: &str = "https://api.linkedin.com";
const MAX_TEXT_CHARS: usize = 3000;

pub struct LinkedInTool;

#[async_trait]
impl Tool for LinkedInTool {
    fn name(&self) -> &'static str {
        "linkedin"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["linkedin"]
    }

    fn label(&self) -> &'static str {
        "LinkedIn"
    }

    fn description(&self) -> String {
        "LinkedIn API でプロフィール取得とテキスト投稿を行う。投稿には author URN と access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["post", "userinfo"], "description": "post/userinfo。" },
                "text": { "type": "string", "description": "post の本文。" },
                "author_urn": { "type": "string", "description": "投稿者URN。例: urn:li:person:xxxx。省略時は設定値。" },
                "visibility": { "type": "string", "enum": ["PUBLIC", "CONNECTIONS"], "description": "公開範囲。省略時 PUBLIC。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_LINKEDIN_ACCESS_TOKEN", "LinkedIn のアクセストークン", "password", ""),
            Field::new("PICOAGENT_LINKEDIN_AUTHOR_URN", "LinkedIn の投稿者 URN", "text", "urn:li:person:..."),
            Field::new("PICOAGENT_LINKEDIN_TIMEOUT_SECS", "LinkedIn API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_LINKEDIN_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide("LinkedIn", &["PICOAGENT_LINKEDIN_ACCESS_TOKEN"], "LinkedIn API の access token が必要です。");
        }
        let timeout = cfg
            .get("PICOAGENT_LINKEDIN_TIMEOUT_SECS", "20")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (LinkedIn tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: LinkedIn API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "userinfo" => {
                handle_text_response(
                    client
                        .get(format!("{}/v2/userinfo", LINKEDIN_BASE))
                        .bearer_auth(token)
                        .send()
                        .await,
                    "LinkedIn API",
                )
                .await
            }
            "post" => {
                let text = match text_arg(&args, "text", MAX_TEXT_CHARS) {
                    Ok(text) => text,
                    Err(error) => return format!("エラー: {}", error),
                };
                let author = args
                    .get("author_urn")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| cfg.get("PICOAGENT_LINKEDIN_AUTHOR_URN", ""));
                if author.is_empty() {
                    return super::config_guide("LinkedIn", &["PICOAGENT_LINKEDIN_AUTHOR_URN"], "投稿時は依頼で author_urn を渡すか、既定の author URN を設定してください。");
                }
                let visibility = args.get("visibility").and_then(Value::as_str).unwrap_or("PUBLIC");
                let payload = json!({
                    "author": author,
                    "lifecycleState": "PUBLISHED",
                    "specificContent": {
                        "com.linkedin.ugc.ShareContent": {
                            "shareCommentary": { "text": text },
                            "shareMediaCategory": "NONE"
                        }
                    },
                    "visibility": {
                        "com.linkedin.ugc.MemberNetworkVisibility": visibility
                    }
                });
                handle_text_response(
                    client
                        .post(format!("{}/v2/ugcPosts", LINKEDIN_BASE))
                        .bearer_auth(token)
                        .header("X-Restli-Protocol-Version", "2.0.0")
                        .json(&payload)
                        .send()
                        .await,
                    "LinkedIn API",
                )
                .await
            }
            _ => "エラー: action は post/userinfo のいずれかを指定してください。".to_string(),
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

inventory::submit! { ToolReg { make: || Box::new(LinkedInTool) } }
