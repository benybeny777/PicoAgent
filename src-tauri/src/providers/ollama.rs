// providers/ollama.rs - ローカル Ollama 用プロバイダコネクタ。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, ToolCall};

pub struct Ollama;

fn host(cfg: &Config) -> String {
    cfg.get("OLLAMA_HOST", "http://127.0.0.1:11434")
}

// 正規形 -> Ollama ワイヤ形式。
fn to_wire(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            if m.role == "assistant" {
                if let Some(calls) = &m.tool_calls {
                    if !calls.is_empty() {
                        let tool_calls: Vec<Value> = calls
                            .iter()
                            .map(|c| json!({ "function": { "name": c.name, "arguments": c.arguments } }))
                            .collect();
                        return json!({ "role": "assistant", "content": m.content, "tool_calls": tool_calls });
                    }
                }
            }
            if m.role == "tool" {
                // Ollama は tool 結果を位置・名前で扱う。
                return json!({ "role": "tool", "content": m.content });
            }
            json!({ "role": m.role, "content": m.content })
        })
        .collect()
}

// Ollama assistant メッセージ -> 正規形。
fn from_wire(message: &Value) -> Message {
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
            let arguments = c
                .get("function")
                .and_then(|f| f.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            tool_calls.push(ToolCall {
                id: format!("call_{}", i),
                name,
                arguments,
            });
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
impl Provider for Ollama {
    fn id(&self) -> &'static str {
        "ollama"
    }
    fn label(&self) -> &'static str {
        "Ollama"
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("OLLAMA_HOST", "Ollama 接続先", "text", "http://127.0.0.1:11434"),
            Field::new("OLLAMA_MODEL", "既定モデル", "text", "qwen3:8b"),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("OLLAMA_MODEL", "qwen3:8b")
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/api/tags", host(cfg));
        let res = reqwest::get(&url).await?;
        if !res.status().is_success() {
            anyhow::bail!("Ollama /api/tags failed: {}", res.status());
        }
        let data: Value = res.json().await?;
        Ok(data
            .get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("name").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn is_up(&self, cfg: &Config) -> bool {
        let url = format!("{}/api/tags", host(cfg));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build();
        match client {
            Ok(c) => c.get(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false),
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
            "stream": false,
            "options": { "temperature": 0.6 },
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        let url = format!("{}/api/chat", host(cfg));
        let res = reqwest::Client::new().post(&url).json(&body).send().await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("Ollama /api/chat failed: {} {}", status, text);
        }
        let data: Value = res.json().await?;
        let message = data.get("message").cloned().unwrap_or_else(|| json!({}));
        Ok(from_wire(&message))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Ollama) } }
