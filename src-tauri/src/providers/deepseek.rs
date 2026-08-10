// providers/deepseek.rs - DeepSeek OpenAI 互換プロバイダコネクタ。
// DeepSeek は /chat/completions が OpenAI 形式なので openai.rs の変換を再利用する。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::openai::{from_wire, to_wire};
use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message};

pub struct DeepSeek;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_MODEL: &str = "deepseek-v4-flash";
const STATIC_MODELS: &[&str] = &[
    "deepseek-v4-flash",
    "deepseek-v4-pro",
    "deepseek-chat",
    "deepseek-reasoner",
];

fn base_url(cfg: &Config) -> String {
    cfg.get("PICOAGENT_DEEPSEEK_BASE_URL", DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

fn api_key(cfg: &Config) -> String {
    cfg.get("PICOAGENT_DEEPSEEK_API_KEY", "")
}

#[async_trait]
impl Provider for DeepSeek {
    fn id(&self) -> &'static str {
        "deepseek"
    }

    fn label(&self) -> &'static str {
        "DeepSeek"
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_DEEPSEEK_API_KEY", "DeepSeek API キー", "password", "sk-..."),
            Field::new("PICOAGENT_DEEPSEEK_MODEL", "既定モデル", "text", DEFAULT_MODEL),
            Field::new("PICOAGENT_DEEPSEEK_THINKING", "Thinking モード", "checkbox", "true"),
            Field::new("PICOAGENT_DEEPSEEK_BASE_URL", "エンドポイント", "text", DEFAULT_BASE_URL),
        ]
    }

    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("PICOAGENT_DEEPSEEK_MODEL", DEFAULT_MODEL)
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
            anyhow::bail!("DeepSeek API キーが未設定です（設定画面の DeepSeek で入力してください）");
        }
        let mut body = json!({
            "model": model,
            "messages": to_wire(messages),
            "temperature": 0.6,
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if model == "deepseek-v4-flash" || model == "deepseek-v4-pro" {
            body["thinking"] = json!({
                "type": if cfg.get_bool("PICOAGENT_DEEPSEEK_THINKING", true) { "enabled" } else { "disabled" },
            });
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
            anyhow::bail!("{}", crate::tools::http::service_error("DeepSeek", "/chat/completions", status, &text));
        }
        let data: Value = res.json().await?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("DeepSeek 応答に choices[0].message がありません"))?;
        Ok(from_wire(&message))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(DeepSeek) } }
