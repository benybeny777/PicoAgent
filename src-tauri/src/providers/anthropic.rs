// providers/anthropic.rs - Anthropic Claude プロバイダコネクタ。
// Messages API の content block へ正規形メッセージとツール呼び出しを相互変換する。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, ToolCall};

pub struct Anthropic;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const STATIC_MODELS: &[&str] = &[
    "claude-opus-4-8",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
];

fn base_url(cfg: &Config) -> String {
    cfg.get("PICOAGENT_ANTHROPIC_BASE_URL", DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

fn api_key(cfg: &Config) -> String {
    cfg.get("PICOAGENT_ANTHROPIC_API_KEY", "")
}

fn to_anthropic_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            Some(json!({
                "name": function.get("name").cloned().unwrap_or_else(|| json!("")),
                "description": function.get("description").cloned().unwrap_or_else(|| json!("")),
                "input_schema": function.get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object" })),
            }))
        })
        .collect()
}

/// data URL 形式の画像を Anthropic の image ブロックへ変換する。
///
/// `data:<media type>;base64,<データ>` 以外（外部URLなど）は base64 ソースに載せられない
/// ため、その画像だけを飛ばす。1枚も載せられなければ None を返し、本文だけを送る。
fn image_blocks(message: &Message) -> Option<Vec<Value>> {
    let images = message.images.as_ref()?;
    let mut blocks = Vec::new();
    for image in images {
        let Some(rest) = image.strip_prefix("data:") else {
            continue;
        };
        let Some((media_type, data)) = rest.split_once(";base64,") else {
            continue;
        };
        blocks.push(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data },
        }));
    }
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

fn to_anthropic_messages(messages: &[Message]) -> (Option<String>, Vec<Value>) {
    let mut system_parts = Vec::new();
    let mut out = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {
                if !message.content.is_empty() {
                    system_parts.push(message.content.clone());
                }
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if !message.content.is_empty() {
                    blocks.push(json!({ "type": "text", "text": message.content }));
                }
                if let Some(calls) = &message.tool_calls {
                    for call in calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": call.arguments,
                        }));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                out.push(json!({ "role": "assistant", "content": blocks }));
            }
            "tool" => {
                out.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                        "content": message.content,
                    }],
                }));
            }
            _ => {
                // 画像付き user メッセージ（view_image スキル経由）は content ブロックで渡す。
                match image_blocks(message) {
                    Some(mut blocks) => {
                        blocks.push(json!({ "type": "text", "text": message.content }));
                        out.push(json!({ "role": "user", "content": blocks }));
                    }
                    None => out.push(json!({ "role": "user", "content": message.content })),
                }
            }
        }
    }
    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };
    (system, out)
}

fn from_anthropic_message(data: &Value) -> Message {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    if let Some(blocks) = data.get("content").and_then(|v| v.as_array()) {
        for (index, block) in blocks.iter().enumerate() {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() {
                            text_parts.push(text.to_string());
                        }
                    }
                }
                Some("tool_use") => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| format!("toolu_{}", index));
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            }
        }
    }
    Message {
        role: "assistant".to_string(),
        content: text_parts.join("\n"),
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: None,
        name: None,
        images: None,
        usage: None,
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn label(&self) -> &'static str {
        "Claude"
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_ANTHROPIC_API_KEY", "Anthropic API キー", "password", "sk-ant-..."),
            Field::new("PICOAGENT_ANTHROPIC_MODEL", "既定モデル", "text", DEFAULT_MODEL),
            Field::new("PICOAGENT_ANTHROPIC_BASE_URL", "エンドポイント", "text", DEFAULT_BASE_URL),
        ]
    }

    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("PICOAGENT_ANTHROPIC_MODEL", DEFAULT_MODEL)
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let key = api_key(cfg);
        if key.is_empty() {
            return Ok(STATIC_MODELS.iter().map(|model| model.to_string()).collect());
        }
        let url = format!("{}/models", base_url(cfg));
        let res = reqwest::Client::new()
            .get(&url)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await?;
        if !res.status().is_success() {
            return Ok(STATIC_MODELS.iter().map(|model| model.to_string()).collect());
        }
        let data: Value = res.json().await?;
        let mut models: Vec<String> = data
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for model in STATIC_MODELS {
            if !models.iter().any(|item| item == model) {
                models.push((*model).to_string());
            }
        }
        Ok(models)
    }

    async fn is_up(&self, cfg: &Config) -> bool {
        let key = api_key(cfg);
        if key.is_empty() {
            return false;
        }
        let url = format!("{}/models", base_url(cfg));
        match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
        {
            Ok(c) => c
                .get(&url)
                .header("x-api-key", key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn chat(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
    ) -> anyhow::Result<Message> {
        let key = api_key(cfg);
        if key.is_empty() {
            anyhow::bail!("Anthropic API キーが未設定です（設定画面の Claude で入力してください）");
        }
        let (system, anthropic_messages) = to_anthropic_messages(messages);
        let mut body = json!({
            "model": model,
            "max_tokens": 4096,
            "temperature": 0.6,
            "messages": anthropic_messages,
        });
        if let Some(system) = system {
            body["system"] = json!(system);
        }
        let anthropic_tools = to_anthropic_tools(tools);
        if !anthropic_tools.is_empty() {
            body["tools"] = json!(anthropic_tools);
        }
        let url = format!("{}/messages", base_url(cfg));
        let res = reqwest::Client::new()
            .post(&url)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic /messages failed: {} {}", status, text);
        }
        let data: Value = res.json().await?;
        Ok(from_anthropic_message(&data))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Anthropic) } }
