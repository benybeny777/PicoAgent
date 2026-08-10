// providers/mistral.rs - Mistral AI OpenAI 互換プロバイダコネクタ。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::openai::{from_wire, to_wire};
use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message};

pub struct Mistral;

const DEFAULT_BASE_URL: &str = "https://api.mistral.ai/v1";
const DEFAULT_MODEL: &str = "mistral-medium-latest";
const STATIC_MODELS: &[&str] = &[
    "mistral-medium-latest",
    "mistral-small-latest",
    "mistral-large-latest",
    "ministral-14b-latest",
    "ministral-8b-latest",
    "ministral-3b-latest",
    "codestral-latest",
];

fn base_url(cfg: &Config) -> String {
    cfg.get("PICOAGENT_MISTRAL_BASE_URL", DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

fn api_key(cfg: &Config) -> String {
    cfg.get("PICOAGENT_MISTRAL_API_KEY", "")
}

#[async_trait]
impl Provider for Mistral {
    fn id(&self) -> &'static str {
        "mistral"
    }

    fn label(&self) -> &'static str {
        "Mistral"
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_MISTRAL_API_KEY", "Mistral API キー", "password", ""),
            Field::new("PICOAGENT_MISTRAL_MODEL", "既定モデル", "text", DEFAULT_MODEL),
            Field::new("PICOAGENT_MISTRAL_BASE_URL", "エンドポイント", "text", DEFAULT_BASE_URL),
        ]
    }

    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("PICOAGENT_MISTRAL_MODEL", DEFAULT_MODEL)
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
        let key = api_key(cfg);
        if key.is_empty() {
            anyhow::bail!("Mistral API キーが未設定です（設定画面の Mistral で入力してください）");
        }
        let mut body = json!({ "model": model, "messages": to_wire(messages), "temperature": 0.6 });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        let url = format!("{}/chat/completions", base_url(cfg));
        let res = crate::tools::http::client()
            .post(&url)
            .bearer_auth(key)
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = crate::tools::http::response_text(res)
                .await
                .map_err(anyhow::Error::msg)?;
            anyhow::bail!("{}", crate::tools::http::service_error("Mistral", "/chat/completions", status, &text));
        }
        let data: Value = res.json().await?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Mistral 応答に choices[0].message がありません"))?;
        Ok(from_wire(&message))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Mistral) } }
