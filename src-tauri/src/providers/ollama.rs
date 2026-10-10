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
            // Ollama は画像を data URL ではなく素の base64 配列で受け取る（view_image 経由）。
            if let Some(images) = &m.images {
                let encoded: Vec<Value> = images
                    .iter()
                    .map(|image| {
                        let data = image
                            .split_once(";base64,")
                            .map(|(_, data)| data)
                            .unwrap_or(image.as_str());
                        Value::String(data.to_string())
                    })
                    .collect();
                if !encoded.is_empty() {
                    return json!({ "role": m.role, "content": m.content, "images": encoded });
                }
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
impl Provider for Ollama {
    fn id(&self) -> &'static str {
        "ollama"
    }
    fn label(&self) -> &'static str {
        "Ollama"
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "OLLAMA_HOST",
                "Ollama 接続先",
                "text",
                "http://127.0.0.1:11434",
            ),
            Field::new("OLLAMA_MODEL", "既定モデル", "text", "qwen3:8b"),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("OLLAMA_MODEL", "qwen3:8b")
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/api/tags", host(cfg));
        let res = crate::tools::http::client()
            .get(&url)
            .timeout(crate::tools::http::metadata_timeout(cfg).map_err(anyhow::Error::msg)?)
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res = super::http_error::provider_response(cfg, "Ollama", "/api/tags", Ok(res)).await?;
        let data: Value = res.json().await.map_err(super::http_error::send_error)?;
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

    async fn is_up(&self, cfg: &Config) -> Result<bool, String> {
        let url = format!("{}/api/tags", host(cfg));
        super::http_error::provider_probe(
            cfg,
            "Ollama",
            "/api/tags",
            crate::tools::http::client()
                .get(url)
                .timeout(std::time::Duration::from_secs(2)),
        )
        .await
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
        let res = crate::tools::http::client()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res = super::http_error::provider_response(cfg, "Ollama", "/api/chat", Ok(res)).await?;
        let data: Value = res.json().await.map_err(super::http_error::send_error)?;
        let message = data.get("message").cloned().unwrap_or_else(|| json!({}));
        Ok(from_wire(&message))
    }

    /// ストリーミング chat。Ollama /api/chat の NDJSON（1行1JSON）を逐次受信し、
    /// テキストチャンクを on_chunk へ渡す（UI の逐次表示・先行読み上げが Ollama でも効く）。
    async fn chat_stream(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        on_chunk: std::sync::Arc<dyn Fn(String) + Send + Sync>,
    ) -> anyhow::Result<Message> {
        use futures::StreamExt;

        let mut body = json!({
            "model": model,
            "messages": to_wire(messages),
            "stream": true,
            "options": { "temperature": 0.6 },
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        let url = format!("{}/api/chat", host(cfg));
        let res = crate::tools::http::client()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res = super::http_error::provider_response(cfg, "Ollama", "/api/chat", Ok(res)).await?;

        let mut full_content = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        let mut buf = String::new();
        // チャンク境界で日本語が分断されても化けないよう、逐次デコーダを通す。
        let mut decoder = crate::text::Utf8Stream::default();
        let mut stream = res.bytes_stream();
        'outer: while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(super::http_error::send_error)?;
            buf.push_str(&decoder.push(&chunk));
            while let Some(line_end) = buf.find('\n') {
                let line = buf[..line_end].trim().to_string();
                buf.drain(..=line_end);
                if line.is_empty() {
                    continue;
                }
                let Ok(json) = serde_json::from_str::<Value>(&line) else {
                    continue; // JSON でない行は読み飛ばす
                };
                if let Some(content) = json["message"]["content"].as_str() {
                    if !content.is_empty() {
                        full_content.push_str(content);
                        on_chunk(content.to_string());
                    }
                }
                // tool_calls は途中のメッセージにまとめて載る（OpenAI のような分割 delta ではない）。
                if let Some(calls) = json["message"]["tool_calls"].as_array() {
                    tool_calls.extend(calls.iter().cloned());
                }
                if json["done"].as_bool() == Some(true) {
                    break 'outer;
                }
            }
        }
        let mut message = json!({ "role": "assistant", "content": full_content });
        if !tool_calls.is_empty() {
            message["tool_calls"] = json!(tool_calls);
        }
        Ok(from_wire(&message))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Ollama) } }

#[cfg(test)]
mod tests {
    use super::{Ollama, Provider};
    use crate::message::Message;
    use std::sync::{Arc, Mutex};

    // NDJSON を返すスタブ Ollama サーバに対して、チャンクが逐次届き最終 Message が
    // 組み上がることを検証する（実 TCP 経由）。
    #[test]
    fn ollamaのndjsonストリーミングを逐次受信できる() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await.unwrap(); // リクエストは読み捨て
                let lines = [
                    r#"{"message":{"role":"assistant","content":"こん"},"done":false}"#,
                    r#"{"message":{"role":"assistant","content":"にちは。"},"done":false}"#,
                    r#"{"message":{"role":"assistant","content":""},"done":true}"#,
                ];
                let body = lines.join("\n") + "\n";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(body.as_bytes()).await.unwrap();
            });

            let cfg = crate::config::Config::new();
            cfg.set("OLLAMA_HOST", &format!("http://127.0.0.1:{}", addr.port()));
            let chunks = Arc::new(Mutex::new(Vec::<String>::new()));
            let sink = {
                let chunks = chunks.clone();
                Arc::new(move |chunk: String| chunks.lock().unwrap().push(chunk)) as Arc<dyn Fn(String) + Send + Sync>
            };
            let message = Ollama
                .chat_stream(&cfg, "test", &[Message::new("user", "やあ")], &[], sink)
                .await
                .unwrap();
            assert_eq!(message.content, "こんにちは。");
            assert_eq!(chunks.lock().unwrap().as_slice(), ["こん", "にちは。"]);
            server.await.unwrap();
        });
    }
}
