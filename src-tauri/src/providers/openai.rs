// providers/openai.rs - OpenAI 互換プロバイダコネクタ。
// OPENAI_BASE_URL（既定 https://api.openai.com/v1）/ OPENAI_API_KEY / OPENAI_MODEL を都度読む。
// ストリーミング対応: `chat_stream` で SSE を受信し、テキストチャンクを on_chunk へ逐次渡す。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, TokenUsage, ToolCall};

pub struct OpenAI;

const DEFAULT_MODEL: &str = "gpt-5.6-luna";
const STATIC_MODELS: &[&str] = &["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"];
const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;
const MAX_STREAM_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 60;

fn base_url(cfg: &Config) -> String {
    cfg.get("OPENAI_BASE_URL", "https://api.openai.com/v1")
        .trim_end_matches('/')
        .to_string()
}
fn api_key(cfg: &Config) -> String {
    if is_official_origin(&base_url(cfg)) {
        cfg.get("OPENAI_API_KEY", "")
    } else {
        cfg.get("OPENAI_COMPAT_API_KEY", "")
    }
}

fn is_official_origin(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.openai.com")
            && url.port_or_known_default() == Some(443)
    })
}

/// 公式エンドポイントを使うのにキーが空なら、生の 401 ではなく設定案内を返す。
///
/// 他の5プロバイダは未設定を明示しているのに、いちばん使われるここだけが
/// `HTTP 401 Unauthorized` とJSONの塊をフキダシへ出していた。
/// ただし `OPENAI_BASE_URL` を自前サーバーへ向けた OpenAI 互換構成では
/// キーが不要なこともあるので、公式ホストのときだけ要求する。
fn ensure_api_key(cfg: &Config) -> anyhow::Result<()> {
    let url = base_url(cfg);
    let official = is_official_origin(&url);
    if official && api_key(cfg).trim().is_empty() {
        anyhow::bail!(
            "OpenAI API キーが未設定です（設定画面の「AIモデル」→ OpenAI で入力してください）"
        );
    }
    Ok(())
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
            let arguments =
                serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({ "_raw": raw }));
            let id = c
                .get("id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("call_{}", i));
            tool_calls.push(ToolCall {
                id,
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
    cfg: &Config,
    url: &str,
    api_key: &str,
    body: Value,
    on_chunk: Arc<dyn Fn(String) + Send + Sync>,
) -> anyhow::Result<Message> {
    use futures::StreamExt;

    // API キー不要のローカルエンジン（llama-server）からも呼ばれるため、
    // 空キーのときは Authorization ヘッダ自体を付けない（空 Bearer で弾かれるのを防ぐ）。
    let request = crate::tools::http::client()
        .post(url)
        .json(&body)
        .timeout(Duration::from_secs(600));
    let request = if api_key.is_empty() {
        request
    } else {
        request.bearer_auth(api_key)
    };
    let res = request
        .send()
        .await
        .map_err(super::http_error::send_error)?;

    let res =
        super::http_error::provider_response(cfg, "OpenAI互換API", "/chat/completions", Ok(res))
            .await?;

    let mut full_content = String::new();
    let mut partial_calls: Vec<PartialToolCall> = Vec::new();
    let mut buf = String::new();
    // チャンク境界で日本語が分断されても化けないよう、逐次デコーダを通す。
    let mut decoder = crate::text::Utf8Stream::default();

    let mut stream = res.bytes_stream();
    let mut received = 0usize;
    'outer: loop {
        let next =
            tokio::time::timeout(Duration::from_secs(STREAM_IDLE_TIMEOUT_SECS), stream.next())
                .await
                .map_err(|_| anyhow::anyhow!("OpenAI互換APIのストリームが60秒間停止しました"))?;
        let Some(chunk_result) = next else { break };
        let chunk = chunk_result.map_err(super::http_error::send_error)?;
        received = received.saturating_add(chunk.len());
        anyhow::ensure!(
            received <= MAX_STREAM_BYTES,
            "OpenAI互換APIのストリームが上限（16 MiB）を超えました"
        );
        buf.push_str(&decoder.push(&chunk));
        anyhow::ensure!(
            buf.len() <= MAX_SSE_LINE_BYTES,
            "OpenAI互換APIのSSE行が上限（1 MiB）を超えました"
        );

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
                    anyhow::ensure!(
                        full_content.len().saturating_add(content.len()) <= MAX_STREAM_BUFFER_BYTES,
                        "OpenAI互換APIの応答内容が上限（8 MiB）を超えました"
                    );
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
                            anyhow::ensure!(
                                pt.args_buf.len().saturating_add(args.len())
                                    <= MAX_STREAM_BUFFER_BYTES,
                                "OpenAI互換APIのツール引数が上限（8 MiB）を超えました"
                            );
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
            Field::new(
                "OPENAI_COMPAT_API_KEY",
                "互換エンドポイント専用 API キー",
                "password",
                "",
            ),
            Field::new("OPENAI_MODEL", "既定モデル", "text", DEFAULT_MODEL),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("OPENAI_MODEL", DEFAULT_MODEL)
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let key = api_key(cfg);
        if key.is_empty() {
            return Ok(STATIC_MODELS
                .iter()
                .map(|model| model.to_string())
                .collect());
        }
        let url = format!("{}/models", base_url(cfg));
        let res = crate::tools::http::client()
            .get(&url)
            .bearer_auth(&key)
            .timeout(crate::tools::http::metadata_timeout(cfg).map_err(anyhow::Error::msg)?)
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res =
            super::http_error::provider_response(cfg, "OpenAI互換API", "/models", Ok(res)).await?;
        let text = crate::tools::http::response_text(res)
            .await
            .map_err(anyhow::Error::msg)?;
        let data = crate::tools::http::response_json(&text).map_err(anyhow::Error::msg)?;
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

    async fn is_up(&self, cfg: &Config) -> Result<bool, String> {
        let key = api_key(cfg);
        if key.is_empty() {
            return Ok(false);
        }
        let url = format!("{}/models", base_url(cfg));
        super::http_error::provider_probe(
            cfg,
            "OpenAI互換API",
            "/models",
            crate::tools::http::client()
                .get(url)
                .bearer_auth(key)
                .timeout(std::time::Duration::from_secs(3)),
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
        ensure_api_key(cfg)?;
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
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res = super::http_error::provider_response(
            cfg,
            "OpenAI互換API",
            "/chat/completions",
            Ok(res),
        )
        .await?;
        let text = crate::tools::http::response_text(res)
            .await
            .map_err(anyhow::Error::msg)?;
        let data = crate::tools::http::response_json(&text).map_err(anyhow::Error::msg)?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("OpenAI 応答に choices[0].message がありません"))?;
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
        ensure_api_key(cfg)?;
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
        stream_chat(cfg, &url, &api_key(cfg), body, on_chunk).await
    }

    async fn chat_with_options(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        json_mode: bool,
    ) -> anyhow::Result<Message> {
        ensure_api_key(cfg)?;
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
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res = super::http_error::provider_response(
            cfg,
            "OpenAI互換API",
            "/chat/completions",
            Ok(res),
        )
        .await?;
        let text = crate::tools::http::response_text(res)
            .await
            .map_err(anyhow::Error::msg)?;
        let data = crate::tools::http::response_json(&text).map_err(anyhow::Error::msg)?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("OpenAI 応答に choices[0].message がありません"))?;
        let mut msg = from_wire(&message);
        msg.usage = extract_usage(&data);
        Ok(msg)
    }
}

inventory::submit! { ProviderReg { make: || Box::new(OpenAI) } }

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_with_call(arguments: Value) -> Message {
        Message {
            role: "assistant".to_string(),
            content: "呼びます".to_string(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "time".to_string(),
                arguments,
            }]),
            tool_call_id: None,
            name: None,
            images: None,
            usage: None,
        }
    }

    /// OpenAI は `arguments` を **JSON 文字列**で受け取る。
    /// 正規形はオブジェクトなので、ここで文字列化を落とすと引数がそのまま壊れる。
    #[test]
    fn 引数をjson文字列にしてtool_callsを送る() {
        let wire = to_wire(&[assistant_with_call(json!({"city": "東京"}))]);
        let call = &wire[0]["tool_calls"][0];
        assert_eq!(call["type"], "function");
        assert_eq!(call["function"]["name"], "time");
        assert_eq!(call["function"]["arguments"], json!(r#"{"city":"東京"}"#));
    }

    /// `tool_calls` が空配列の assistant を「ツール呼び出しあり」として送ると、
    /// OpenAI は空配列を拒否する。素のテキストメッセージへ落とすこと。
    #[test]
    fn 空のtool_callsは素のメッセージへ落とす() {
        let mut message = assistant_with_call(json!({}));
        message.tool_calls = Some(Vec::new());
        let wire = to_wire(&[message]);
        assert!(wire[0].get("tool_calls").is_none());
        assert_eq!(wire[0]["content"], "呼びます");
    }

    #[test]
    fn toolロールはtool_call_idを添えて送る() {
        let mut message = Message::new("tool", "12時です");
        message.tool_call_id = Some("call_1".to_string());
        let wire = to_wire(&[message]);
        assert_eq!(wire[0]["role"], "tool");
        assert_eq!(wire[0]["tool_call_id"], "call_1");
        assert_eq!(wire[0]["content"], "12時です");
    }

    /// 画像付き user だけを content 配列にする。画像が無いのに配列化すると
    /// 画像非対応の互換サーバーで弾かれる。
    #[test]
    fn 画像付きuserだけをcontent配列にする() {
        let with_image =
            Message::user_with_images("これは？", vec!["data:image/png;base64,AA".to_string()]);
        let wire = to_wire(&[with_image]);
        assert_eq!(wire[0]["content"][0]["type"], "text");
        assert_eq!(
            wire[0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AA"
        );

        let plain = to_wire(&[Message::new("user", "こんにちは")]);
        assert_eq!(plain[0]["content"], "こんにちは");
    }

    #[test]
    fn 応答のjson文字列引数をオブジェクトへ戻す() {
        let message = from_wire(&json!({
            "content": "はい",
            "tool_calls": [{
                "id": "call_9",
                "function": { "name": "time", "arguments": "{\"city\":\"東京\"}" }
            }]
        }));
        let calls = message.tool_calls.unwrap();
        assert_eq!(calls[0].id, "call_9");
        assert_eq!(calls[0].arguments, json!({"city": "東京"}));
    }

    /// 引数が壊れていても捨てない。`_raw` に残してエージェント側が理由を説明できるようにする。
    /// id が無いモデルもあるので、その場合は位置から採番する。
    #[test]
    fn 壊れた引数は_rawへ残しid欠落は採番する() {
        let message = from_wire(&json!({
            "content": "",
            "tool_calls": [{ "function": { "name": "time", "arguments": "{壊れ" } }]
        }));
        let calls = message.tool_calls.unwrap();
        assert_eq!(calls[0].id, "call_0");
        assert_eq!(calls[0].arguments, json!({"_raw": "{壊れ"}));
    }

    #[test]
    fn tool_callsが無ければnoneで返す() {
        let message = from_wire(&json!({ "content": "ただの返事" }));
        assert_eq!(message.role, "assistant");
        assert_eq!(message.content, "ただの返事");
        assert!(message.tool_calls.is_none());
    }

    /// 正規形 → ワイヤ → 正規形で、ツール呼び出しが元に戻ること。
    /// 5プロバイダ（openai / deepseek / xai / mistral / gemini）が共有する中核の契約。
    #[test]
    fn tool_callsは往復しても壊れない() {
        let original = assistant_with_call(json!({"city": "東京", "unit": "c"}));
        let restored = from_wire(&to_wire(std::slice::from_ref(&original))[0]);
        let (before, after) = (original.tool_calls.unwrap(), restored.tool_calls.unwrap());
        assert_eq!(before[0].id, after[0].id);
        assert_eq!(before[0].name, after[0].name);
        assert_eq!(before[0].arguments, after[0].arguments);
    }

    /// APIキー未設定の案内を出すかどうかの判定。ホスト名の部分一致で通すと、
    /// 自前サーバー構成なのにキーを要求して使えなくなる（逆も同様）。
    #[test]
    fn 公式エンドポイントだけを厳密に判定する() {
        assert!(is_official_origin("https://api.openai.com/v1"));
        assert!(is_official_origin("https://api.openai.com:443/v1"));
        for other in [
            "http://api.openai.com/v1",
            "https://api.openai.com.example.com/v1",
            "https://proxy.example.com/v1",
            "https://api.openai.com:8443/v1",
            "not a url",
        ] {
            assert!(!is_official_origin(other), "{other}");
        }
    }

    #[test]
    fn 使用量が無い応答ではnoneを返す() {
        assert!(extract_usage(&json!({ "choices": [] })).is_none());
        let usage = extract_usage(&json!({
            "usage": { "prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14 }
        }))
        .unwrap();
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (10, 4, 14)
        );
    }
}
