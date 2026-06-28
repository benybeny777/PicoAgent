// providers/openai.rs - OpenAI 互換プロバイダコネクタ。
// OPENAI_BASE_URL（既定 https://api.openai.com/v1）/ OPENAI_API_KEY / OPENAI_MODEL を都度読む。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, ToolCall};

pub struct OpenAI;

const DEFAULT_MODEL: &str = "gpt-5.4-mini";
const STATIC_MODELS: &[&str] = &["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.4-nano"];

fn base_url(cfg: &Config) -> String {
    cfg.get("OPENAI_BASE_URL", "https://api.openai.com/v1")
        .trim_end_matches('/')
        .to_string()
}
fn api_key(cfg: &Config) -> String {
    cfg.get("OPENAI_API_KEY", "")
}

// 正規形 -> OpenAI ワイヤ形式。local プロバイダからも再利用する。
pub(crate) fn to_wire(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            if m.role == "assistant" {
                if let Some(calls) = &m.tool_calls {
                    if !calls.is_empty() {
                        let tool_calls: Vec<Value> = calls
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": { "name": c.name, "arguments": c.arguments.to_string() }
                                })
                            })
                            .collect();
                        return json!({ "role": "assistant", "content": m.content, "tool_calls": tool_calls });
                    }
                }
            }
            if m.role == "tool" {
                return json!({
                    "role": "tool",
                    "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.content
                });
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect()
}

// OpenAI assistant メッセージ -> 正規形。local プロバイダからも再利用する。
pub(crate) fn from_wire(message: &Value) -> Message {
    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for (i, c) in calls.iter().enumerate() {
            let name = c
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let raw = c
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            // OpenAI は arguments を JSON 文字列で返す。壊れていれば _raw として残す。
            let arguments = serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({ "_raw": raw }));
            let id = c
                .get("id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("call_{}", i));
            tool_calls.push(ToolCall { id, name, arguments });
        }
    }
    Message {
        role: "assistant".to_string(),
        content,
        tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
        tool_call_id: None,
        name: None,
    }
}

#[async_trait]
impl Provider for OpenAI {
    fn id(&self) -> &'static str {
        "openai"
    }
    fn label(&self) -> &'static str {
        "OpenAI互換"
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("OPENAI_BASE_URL", "エンドポイント", "text", "https://api.openai.com/v1"),
            Field::new("OPENAI_API_KEY", "API キー", "password", "sk-..."),
            Field::new("OPENAI_MODEL", "既定モデル", "text", DEFAULT_MODEL),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("OPENAI_MODEL", DEFAULT_MODEL)
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let key = api_key(cfg);
        if key.is_empty() {
            return Ok(STATIC_MODELS.iter().map(|model| model.to_string()).collect());
        }
        let url = format!("{}/models", base_url(cfg));
        let res = reqwest::Client::new().get(&url).bearer_auth(&key).send().await?;
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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build();
        match client {
            Ok(c) => c
                .get(&url)
                .bearer_auth(&key)
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
        let mut body = json!({
            "model": model,
            "messages": to_wire(messages),
            "temperature": 0.6,
        });
        if !tools.is_empty() {
            // tools は既に { type:"function", function:{...} } 形式。
            body["tools"] = json!(tools);
        }
        let url = format!("{}/chat/completions", base_url(cfg));
        let res = reqwest::Client::new()
            .post(&url)
            .bearer_auth(api_key(cfg))
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI /chat/completions failed: {} {}", status, text);
        }
        let data: Value = res.json().await?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("OpenAI 応答に choices[0].message がありません"))?;
        Ok(from_wire(&message))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(OpenAI) } }
