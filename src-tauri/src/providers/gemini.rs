// providers/gemini.rs - Google Gemini プロバイダコネクタ。
// Gemini の OpenAI 互換エンドポイント（/v1beta/openai）を使うため、ワイヤ変換は
// openai.rs の to_wire/from_wire を再利用する。設定は都度 cfg.get() で読む。
// API キー利用は費用が発生し得るため、ユーザーが gemini を選んだ場合のみ叩く。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::openai::{from_wire, to_wire};
use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message};

pub struct Gemini;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";
const DEFAULT_MODEL: &str = "gemini-2.5-flash";

fn base_url(cfg: &Config) -> String {
    cfg.get("PICOAGENT_GEMINI_BASE_URL", DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}
fn api_key(cfg: &Config) -> String {
    cfg.get("PICOAGENT_GEMINI_API_KEY", "")
}

#[async_trait]
impl Provider for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }
    fn label(&self) -> &'static str {
        "Gemini"
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_GEMINI_API_KEY", "Gemini API キー", "password", "AIza..."),
            Field::new("PICOAGENT_GEMINI_MODEL", "既定モデル", "text", DEFAULT_MODEL),
            Field::new(
                "PICOAGENT_GEMINI_BASE_URL",
                "エンドポイント（OpenAI 互換）",
                "text",
                DEFAULT_BASE_URL,
            ),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("PICOAGENT_GEMINI_MODEL", DEFAULT_MODEL)
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let key = api_key(cfg);
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/models", base_url(cfg));
        let res = crate::tools::http::client().get(&url).bearer_auth(&key).send().await?;
        if !res.status().is_success() {
            anyhow::bail!("Gemini /models failed: {}", res.status());
        }
        let data: Value = res.json().await?;
        Ok(data
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                    // Gemini は "models/gemini-2.5-flash" 形式で返すことがあるため前置詞を外す。
                    .map(|id| id.strip_prefix("models/").unwrap_or(id).to_string())
                    .collect()
            })
            .unwrap_or_default())
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
        let url = format!("{}/chat/completions", base_url(cfg));
        let key = api_key(cfg);
        if key.is_empty() {
            anyhow::bail!("Gemini API キーが未設定です（設定画面の Gemini で入力してください）");
        }
        let send = |with_tools: bool| {
            let mut body = json!({ "model": model, "messages": to_wire(messages), "temperature": 0.6 });
            if with_tools && !tools.is_empty() {
                body["tools"] = json!(tools);
            }
            crate::tools::http::client()
                .post(&url)
                .bearer_auth(&key)
                .json(&body)
                .send()
        };

        // tools 付きで失敗したら、tools 非対応のケースに備えて一度だけ tools 無しで再試行する。
        let mut res = send(!tools.is_empty()).await?;
        if !res.status().is_success() && !tools.is_empty() {
            res = send(false).await?;
        }
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("Gemini /chat/completions failed: {} {}", status, text);
        }
        let data: Value = res.json().await?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Gemini 応答に choices[0].message がありません"))?;
        Ok(from_wire(&message))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Gemini) } }
