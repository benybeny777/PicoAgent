// providers/anthropic.rs - Anthropic Claude プロバイダコネクタ。
// Messages API の content block へ正規形メッセージとツール呼び出しを相互変換する。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, ToolCall};

pub struct Anthropic;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_MODEL: &str = "claude-sonnet-5";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const STATIC_MODELS: &[&str] = &[
    "claude-fable-5",
    "claude-opus-5",
    "claude-sonnet-5",
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
            Field::new(
                "PICOAGENT_ANTHROPIC_API_KEY",
                "Anthropic API キー",
                "password",
                "sk-ant-...",
            ),
            Field::new(
                "PICOAGENT_ANTHROPIC_MODEL",
                "既定モデル",
                "text",
                DEFAULT_MODEL,
            ),
            Field::new(
                "PICOAGENT_ANTHROPIC_BASE_URL",
                "エンドポイント",
                "text",
                DEFAULT_BASE_URL,
            ),
        ]
    }

    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("PICOAGENT_ANTHROPIC_MODEL", DEFAULT_MODEL)
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
            .map_err(anyhow::Error::msg)?
            .get(&url)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .timeout(crate::tools::http::metadata_timeout(cfg).map_err(anyhow::Error::msg)?)
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res = super::http_error::provider_response(cfg, "Claude", "/models", Ok(res)).await?;
        let data: Value = res.json().await.map_err(super::http_error::send_error)?;
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
            "Claude",
            "/models",
            crate::tools::http::client()?
                .get(url)
                .header("x-api-key", key)
                .header("anthropic-version", ANTHROPIC_VERSION)
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
        let res = crate::tools::http::client()
            .map_err(anyhow::Error::msg)?
            .post(&url)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res = super::http_error::provider_response(cfg, "Claude", "/messages", Ok(res)).await?;
        let data: Value = res.json().await.map_err(super::http_error::send_error)?;
        Ok(from_anthropic_message(&data))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Anthropic) } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 共通protocolの固定systemと画像と呼出pairを変えず新資料を末尾へ送る() {
        let (before, after, _) = crate::agent::context::protocol_fixture();
        let (system, earlier) = to_anthropic_messages(&before);
        let (next_system, later) = to_anthropic_messages(&after);
        assert_eq!(system, next_system);
        assert_eq!(system.as_deref(), Some(after[0].content.as_str()));
        assert_eq!(earlier, later[..earlier.len()]);
        assert_eq!(later[0]["content"][0]["source"]["data"], "aW1hZ2U=");
        assert_eq!(later[0]["content"][1]["text"], after[1].content);
        assert_eq!(
            later[1]["content"][0]["id"],
            after[2].tool_calls.as_ref().unwrap()[0].id
        );
        assert_eq!(
            later[1]["content"][0]["input"],
            after[2].tool_calls.as_ref().unwrap()[0].arguments
        );
        assert_eq!(
            later[2]["content"][0]["tool_use_id"],
            after[3].tool_call_id.as_ref().unwrap().as_str()
        );
        assert_eq!(
            later.last().unwrap()["content"],
            after.last().unwrap().content
        );
    }

    fn assistant_with_call(arguments: Value) -> Message {
        Message {
            role: "assistant".to_string(),
            content: "呼びます".to_string(),
            tool_calls: Some(vec![ToolCall {
                id: "toolu_1".to_string(),
                name: "time".to_string(),
                arguments,
            }]),
            tool_call_id: None,
            name: None,
            images: None,
            usage: None,
        }
    }

    /// Anthropic は system をメッセージ列ではなく専用フィールドで受け取る。
    /// 複数の system をそのまま並べると無視されるので、分離して連結すること。
    #[test]
    fn systemは分離して連結する() {
        let (system, messages) = to_anthropic_messages(&[
            Message::new("system", "あなたはペットです"),
            Message::new("system", "日本語で話す"),
            Message::new("system", ""),
            Message::new("user", "こんにちは"),
        ]);
        assert_eq!(system.unwrap(), "あなたはペットです\n\n日本語で話す");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    /// Anthropic に `tool` ロールは無い。user の tool_result ブロックへ移す。
    #[test]
    fn toolロールはuserのtool_resultへ移す() {
        let mut message = Message::new("tool", "12時です");
        message.tool_call_id = Some("toolu_1".to_string());
        let (_, messages) = to_anthropic_messages(&[message]);
        assert_eq!(messages[0]["role"], "user");
        let block = &messages[0]["content"][0];
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["tool_use_id"], "toolu_1");
        assert_eq!(block["content"], "12時です");
    }

    /// OpenAI と違い、Anthropic の `input` は**オブジェクトのまま**渡す。
    /// ここで文字列化すると、モデルが引数を読めない。
    #[test]
    fn 引数はオブジェクトのままtool_useへ載せる() {
        let (_, messages) = to_anthropic_messages(&[assistant_with_call(json!({"city": "東京"}))]);
        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "tool_use");
        assert_eq!(blocks[1]["id"], "toolu_1");
        assert_eq!(blocks[1]["input"], json!({"city": "東京"}));
    }

    /// 中身の無い assistant を空の content で送ると API がエラーを返す。空テキストで埋める。
    #[test]
    fn 空のassistantは空テキストブロックで埋める() {
        let (_, messages) = to_anthropic_messages(&[Message::new("assistant", "")]);
        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], json!({ "type": "text", "text": "" }));
    }

    /// base64 の data URL だけを image ブロックへ載せる。外部URLは載せられないので飛ばし、
    /// 1枚も載らなければ本文だけを送る（画像を理由に発話ごと落とさない）。
    #[test]
    fn data_url以外の画像は飛ばす() {
        let mixed = Message::user_with_images(
            "これは？",
            vec![
                "https://example.com/a.png".to_string(),
                "data:image/png;base64,AA".to_string(),
            ],
        );
        let (_, messages) = to_anthropic_messages(&[mixed]);
        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["source"]["media_type"], "image/png");
        assert_eq!(blocks[0]["source"]["data"], "AA");
        assert_eq!(blocks[1]["text"], "これは？");

        let only_url =
            Message::user_with_images("これは？", vec!["https://example.com/a.png".to_string()]);
        let (_, plain) = to_anthropic_messages(&[only_url]);
        assert_eq!(plain[0]["content"], "これは？");
    }

    #[test]
    fn 応答のtool_useとテキストを正規形へ戻す() {
        let message = from_anthropic_message(&json!({
            "content": [
                { "type": "text", "text": "調べます" },
                { "type": "text", "text": "" },
                { "type": "thinking", "thinking": "無視される種別" },
                { "type": "tool_use", "name": "time", "input": { "city": "東京" } }
            ]
        }));
        assert_eq!(message.role, "assistant");
        assert_eq!(message.content, "調べます");
        let calls = message.tool_calls.unwrap();
        assert_eq!(calls[0].name, "time");
        assert_eq!(calls[0].arguments, json!({"city": "東京"}));
        // id を返さないブロックでも、後続の tool_result が対応づけられるよう位置から採番する。
        assert_eq!(calls[0].id, "toolu_3");
    }

    #[test]
    fn ツール定義はfunctionを持つものだけ変換する() {
        let tools = to_anthropic_tools(&[
            json!({ "type": "function", "function": { "name": "time", "description": "時刻", "parameters": { "type": "object" } } }),
            json!({ "type": "function", "function": { "name": "bare" } }),
            json!({ "type": "unknown" }),
        ]);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "time");
        assert_eq!(tools[0]["input_schema"], json!({ "type": "object" }));
        // parameters 欠落でも空スキーマを補う（欠けたまま送ると API が拒否する）。
        assert_eq!(tools[1]["input_schema"], json!({ "type": "object" }));
        assert_eq!(tools[1]["description"], "");
    }
}
