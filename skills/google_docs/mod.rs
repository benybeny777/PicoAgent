// skills/google_docs/mod.rs - Google Docs API の文書取得・作成・置換スキル。
//
// Google Docs は Drive の .gdoc ファイルではなく Docs API を正ルートにする。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const DOCS_BASE: &str = "https://docs.googleapis.com/v1/documents";
const MAX_TEXT_CHARS: usize = 100_000;

pub struct GoogleDocsTool;

fn timeout_secs(cfg: &Config) -> u64 {
    cfg.get("PICOAGENT_GOOGLE_DOCS_TIMEOUT_SECS", "20")
        .parse::<u64>()
        .ok()
        .filter(|value| (3..=60).contains(value))
        .unwrap_or(20)
}

fn collect_text(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            if let Some(content) = map
                .get("textRun")
                .and_then(|text_run| text_run.get("content"))
                .and_then(Value::as_str)
            {
                out.push_str(content);
            }
            for child in map.values() {
                collect_text(child, out);
                if out.chars().count() > MAX_TEXT_CHARS {
                    return;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_text(item, out);
                if out.chars().count() > MAX_TEXT_CHARS {
                    return;
                }
            }
        }
        _ => {}
    }
}

#[async_trait]
impl Tool for GoogleDocsTool {
    fn name(&self) -> &'static str {
        "google_docs"
    }

    fn label(&self) -> &'static str {
        "Google Docs"
    }

    fn description(&self) -> String {
        "Google Docs API でドキュメントの作成、本文抽出、全文末尾追記、文字列置換を行う。設定に OAuth access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["create", "get_text", "append_text", "replace_text"], "description": "create/get_text/append_text/replace_text。" },
                "document_id": { "type": "string", "description": "対象ドキュメントID。create では不要。" },
                "title": { "type": "string", "description": "create のタイトル。" },
                "text": { "type": "string", "description": "append_text で末尾に追加するテキスト。" },
                "needle": { "type": "string", "description": "replace_text の置換前文字列。" },
                "replacement": { "type": "string", "description": "replace_text の置換後文字列。" },
                "match_case": { "type": "boolean", "description": "replace_text の大文字小文字区別。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_GOOGLE_DOCS_ACCESS_TOKEN", "Google Docs OAuth access token", "password", "ya29..."),
            Field::new("PICOAGENT_GOOGLE_DOCS_TIMEOUT_SECS", "Google Docs API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_GOOGLE_DOCS_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide("Google Docs", &["PICOAGENT_GOOGLE_DOCS_ACCESS_TOKEN"], "Google Docs API を使える OAuth access token が必要です。");
        }
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs(cfg)))
            .user_agent("PicoAgent/1.0 (Google Docs tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Google Docs API の準備に失敗しました: {}", error),
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "create" => {
                let title = args.get("title").and_then(Value::as_str).unwrap_or("Untitled");
                let response = client
                    .post(DOCS_BASE)
                    .bearer_auth(token)
                    .json(&json!({ "title": title }))
                    .send()
                    .await;
                handle_text_response(response, "Google Docs API").await
            }
            "get_text" => {
                let document_id = match document_id(&args) {
                    Ok(document_id) => document_id,
                    Err(error) => return format!("エラー: {}", error),
                };
                let response = match client
                    .get(format!("{}/{}", DOCS_BASE, document_id))
                    .bearer_auth(token)
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => return format!("エラー: Google Docs API への接続に失敗しました: {}", error),
                };
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    return format!("エラー: Google Docs API が HTTP {} を返しました。", status);
                }
                let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
                let mut text = String::new();
                collect_text(&parsed, &mut text);
                if text.chars().count() > MAX_TEXT_CHARS {
                    text = text.chars().take(MAX_TEXT_CHARS).collect::<String>();
                    text.push_str("\n…（抽出サイズ上限で打ち切り）");
                }
                text
            }
            "append_text" => {
                let document_id = match document_id(&args) {
                    Ok(document_id) => document_id,
                    Err(error) => return format!("エラー: {}", error),
                };
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                let end_index = match fetch_end_index(&client, &token, document_id).await {
                    Ok(index) => index,
                    Err(error) => return format!("エラー: {}", error),
                };
                let payload = json!({
                    "requests": [{
                        "insertText": {
                            "location": { "index": end_index.saturating_sub(1) },
                            "text": text
                        }
                    }]
                });
                handle_text_response(
                    client
                        .post(format!("{}/{}:batchUpdate", DOCS_BASE, document_id))
                        .bearer_auth(token)
                        .json(&payload)
                        .send()
                        .await,
                    "Google Docs API",
                )
                .await
            }
            "replace_text" => {
                let document_id = match document_id(&args) {
                    Ok(document_id) => document_id,
                    Err(error) => return format!("エラー: {}", error),
                };
                let needle = match args.get("needle").and_then(Value::as_str) {
                    Some(needle) if !needle.is_empty() => needle,
                    _ => return "エラー: needle が指定されていません。".to_string(),
                };
                let replacement = args.get("replacement").and_then(Value::as_str).unwrap_or("");
                let payload = json!({
                    "requests": [{
                        "replaceAllText": {
                            "containsText": {
                                "text": needle,
                                "matchCase": args.get("match_case").and_then(Value::as_bool).unwrap_or(true)
                            },
                            "replaceText": replacement
                        }
                    }]
                });
                handle_text_response(
                    client
                        .post(format!("{}/{}:batchUpdate", DOCS_BASE, document_id))
                        .bearer_auth(token)
                        .json(&payload)
                        .send()
                        .await,
                    "Google Docs API",
                )
                .await
            }
            _ => "エラー: action は create/get_text/append_text/replace_text のいずれかを指定してください。".to_string(),
        }
    }
}

fn document_id(args: &Value) -> Result<&str, String> {
    args.get("document_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "document_id が指定されていません。".to_string())
}

async fn fetch_end_index(client: &reqwest::Client, token: &str, document_id: &str) -> Result<u64, String> {
    let response = client
        .get(format!("{}/{}", DOCS_BASE, document_id))
        .bearer_auth(token)
        .query(&[("fields", "body/content/endIndex")])
        .send()
        .await
        .map_err(|error| format!("Google Docs API への接続に失敗しました: {}", error))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Google Docs API が HTTP {} を返しました。", status));
    }
    let parsed: Value = serde_json::from_str(&body).map_err(|error| format!("Docs構造の解析に失敗しました: {}", error))?;
    parsed
        .get("body")
        .and_then(|body| body.get("content"))
        .and_then(Value::as_array)
        .and_then(|items| items.last())
        .and_then(|item| item.get("endIndex"))
        .and_then(Value::as_u64)
        .ok_or_else(|| "文書末尾位置を取得できませんでした。".to_string())
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

inventory::submit! { ToolReg { make: || Box::new(GoogleDocsTool) } }

#[cfg(test)]
mod tests {
    use super::collect_text;
    use serde_json::json;

    #[test]
    fn docs構造からtextrunを集める() {
        let value = json!({
            "body": { "content": [{ "paragraph": { "elements": [
                { "textRun": { "content": "A" } },
                { "textRun": { "content": "B" } }
            ] } }] }
        });
        let mut out = String::new();
        collect_text(&value, &mut out);
        assert_eq!(out, "AB");
    }
}
