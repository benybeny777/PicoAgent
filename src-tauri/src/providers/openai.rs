// providers/openai.rs - OpenAI 互換プロバイダコネクタ。
// OPENAI_BASE_URL（既定 https://api.openai.com/v1）/ OPENAI_API_KEY / OPENAI_MODEL を都度読む。
// ストリーミング対応: `chat_stream` で SSE を受信し、テキストチャンクを on_chunk へ逐次渡す。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, TokenUsage, ToolCall};

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
            // マルチモーダル: user メッセージに画像がある場合はコンテンツ配列形式にする。
            if m.role == "user" {
                if let Some(images) = &m.images {
                    if !images.is_empty() {
                        let mut parts: Vec<Value> = vec![json!({"type": "text", "text": m.content})];
                        for img in images {
                            parts.push(json!({
                                "type": "image_url",
                                "image_url": { "url": img }
                            }));
                        }
                        return json!({ "role": "user", "content": parts });
                    }
                }
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
            let arguments = serde_json::from_str::<Value>(raw)
                .unwrap_or_else(|_| json!({ "_raw": raw }));
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

/// レスポンス JSON の `usage` フィールドから TokenUsage を抽出する。
fn extract_usage(data: &Value) -> Option<TokenUsage> {
    let u = data.get("usage")?;
    Some(TokenUsage {
        prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
        total_tokens: u["total_tokens"].as_u64().unwrap_or(0) as u32,
    })
}

/// ストリーミング SSE を受信してチャンクを on_chunk に渡し、完全な Message を返す。
/// OpenAI 互換 API 共通の実装（同梱ローカルエンジン local からも利用する）。
pub(crate) async fn stream_chat(
    url: &str,
    api_key: &str,
    body: Value,
    on_chunk: Arc<dyn Fn(String) + Send + Sync>,
) -> anyhow::Result<Message> {
    use futures::StreamExt;

    // API キー不要のローカルエンジン（llama-server）からも呼ばれるため、
    // 空キーのときは Authorization ヘッダ自体を付けない（空 Bearer で弾かれるのを防ぐ）。
    let request = crate::tools::http::client().post(url).json(&body);
    let request = if api_key.is_empty() {
        request
    } else {
        request.bearer_auth(api_key)
    };
    let res = request.send().await?;

    if !res.status().is_success() {
        let status = res.status();
        let text = crate::tools::http::response_text(res)
            .await
            .map_err(anyhow::Error::msg)?;
        anyhow::bail!("{}", crate::tools::http::service_error("OpenAI互換API", "/chat/completions（ストリーミング）", status, &text));
    }

    let mut full_content = String::new();
    let mut partial_calls: Vec<PartialToolCall> = Vec::new();
    let mut buf = String::new();
    // チャンク境界で日本語が分断されても化けないよう、逐次デコーダを通す。
    let mut decoder = crate::text::Utf8Stream::default();

    let mut stream = res.bytes_stream();
    'outer: while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        buf.push_str(&decoder.push(&chunk));

        while let Some(line_end) = buf.find('\n') {
            let line = buf[..line_end].trim_end_matches('\r').to_string();
            buf.drain(..=line_end);

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                break 'outer;
            }
            let Ok(json) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            let delta = &json["choices"][0]["delta"];

            // テキストチャンク。
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    full_content.push_str(content);
                    on_chunk(content.to_string());
                }
            }

            // tool_calls チャンク（インデックスベースで結合）。
            if let Some(tc_array) = delta.get("tool_calls").and_then(Value::as_array) {
                for tc_delta in tc_array {
                    let idx = tc_delta["index"].as_u64().unwrap_or(0) as usize;
                    while partial_calls.len() <= idx {
                        partial_calls.push(PartialToolCall::default());
                    }
                    let pt = &mut partial_calls[idx];
                    if let Some(id) = tc_delta.get("id").and_then(Value::as_str) {
                        pt.id = id.to_string();
                    }
                    if let Some(fn_delta) = tc_delta.get("function") {
                        if let Some(name) = fn_delta.get("name").and_then(Value::as_str) {
                            pt.name = name.to_string();
                        }
                        if let Some(args) = fn_delta.get("arguments").and_then(Value::as_str) {
                            pt.args_buf.push_str(args);
                        }
                    }
                }
            }
        }
    }

    let tool_calls: Vec<ToolCall> = partial_calls
        .into_iter()
        .enumerate()
        .filter(|(_, pt)| !pt.name.is_empty())
        .map(|(i, pt)| {
            let arguments = serde_json::from_str::<Value>(&pt.args_buf)
                .unwrap_or_else(|_| json!({ "_raw": pt.args_buf }));
            ToolCall {
                id: if pt.id.is_empty() {
                    format!("call_{}", i)
                } else {
                    pt.id
                },
                name: pt.name,
                arguments,
            }
        })
        .collect();

    Ok(Message {
        role: "assistant".to_string(),
        content: full_content,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: None,
        name: None,
        images: None,
        usage: None, // ストリーミング時は usage フィールドが最終チャンクに含まれる場合があるが省略。
    })
}

/// ストリーミングの部分ツールコール（SSE チャンクを結合するためのバッファ）。
#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args_buf: String,
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
            Field::new(
                "OPENAI_BASE_URL",
                "エンドポイント",
                "text",
                "https://api.openai.com/v1",
            ),
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
        let res = crate::tools::http::client()
            .get(&url)
            .bearer_auth(&key)
            .timeout(crate::tools::http::metadata_timeout(cfg))
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
            body["tools"] = json!(tools);
        }
        let url = format!("{}/chat/completions", base_url(cfg));
        let res = crate::tools::http::client()
            .post(&url)
            .bearer_auth(api_key(cfg))
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = crate::tools::http::response_text(res)
                .await
                .map_err(anyhow::Error::msg)?;
            anyhow::bail!("{}", crate::tools::http::service_error("OpenAI互換API", "/chat/completions", status, &text));
        }
        let data: Value = res.json().await?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("OpenAI 応答に choices[0].message がありません")
            })?;
        let mut msg = from_wire(&message);
        msg.usage = extract_usage(&data);
        Ok(msg)
    }

    async fn chat_stream(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        on_chunk: Arc<dyn Fn(String) + Send + Sync>,
    ) -> anyhow::Result<Message> {
        let mut body = json!({
            "model": model,
            "messages": to_wire(messages),
            "temperature": 0.6,
            "stream": true,
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        let url = format!("{}/chat/completions", base_url(cfg));
        stream_chat(&url, &api_key(cfg), body, on_chunk).await
    }

    async fn chat_with_options(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        json_mode: bool,
    ) -> anyhow::Result<Message> {
        let mut body = json!({
            "model": model,
            "messages": to_wire(messages),
            "temperature": 0.3,
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        let url = format!("{}/chat/completions", base_url(cfg));
        let res = crate::tools::http::client()
            .post(&url)
            .bearer_auth(api_key(cfg))
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = crate::tools::http::response_text(res)
                .await
                .map_err(anyhow::Error::msg)?;
            anyhow::bail!("{}", crate::tools::http::service_error("OpenAI互換API", "/chat/completions", status, &text));
        }
        let data: Value = res.json().await?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("OpenAI 応答に choices[0].message がありません")
            })?;
        let mut msg = from_wire(&message);
        msg.usage = extract_usage(&data);
        Ok(msg)
    }
}

inventory::submit! { ProviderReg { make: || Box::new(OpenAI) } }
