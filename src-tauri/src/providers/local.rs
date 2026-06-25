// providers/local.rs - 同梱 llama-server（OpenAI 互換）を叩く既定プロバイダ。
// Ollama 不要。エンジンの起動とモデル取得は engine.rs が担う。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::openai::{from_wire, to_wire};
use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message};

pub struct Local;

pub fn port(cfg: &Config) -> u16 {
    cfg.get("PICOAGENT_LOCAL_PORT", "11435").parse().unwrap_or(11435)
}
fn base_url(cfg: &Config) -> String {
    format!("http://127.0.0.1:{}/v1", port(cfg))
}

#[async_trait]
impl Provider for Local {
    fn id(&self) -> &'static str {
        "local"
    }
    fn label(&self) -> &'static str {
        "ローカル(gemma4)"
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_MODEL_URL",
                "モデル GGUF の取得元 URL",
                "text",
                "https://huggingface.co/.../gemma-3n-E2B-it-Q4_K_M.gguf",
            ),
            Field::new("PICOAGENT_LOCAL_PORT", "ローカルエンジンのポート", "text", "11435"),
            Field::new("PICOAGENT_LOCAL_CTX", "コンテキスト長", "text", "4096"),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("PICOAGENT_MODEL", "gemma-3n-e2b")
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/models", base_url(cfg));
        let res = reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await?;
        if !res.status().is_success() {
            anyhow::bail!("local /models failed: {}", res.status());
        }
        let data: Value = res.json().await?;
        Ok(data
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn is_up(&self, cfg: &Config) -> bool {
        // llama-server はモデルロード完了で /health が 200 を返す（ロード中は 503）。
        let url = format!("http://127.0.0.1:{}/health", port(cfg));
        match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
        {
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
        let url = format!("{}/chat/completions", base_url(cfg));
        let wire = to_wire(messages);

        // まず tools 付きで送る。Gemma など tools 非対応テンプレートのモデルでも
        // 会話が成立するよう、失敗したら tools 無しで一度だけ再試行する。
        let send = |with_tools: bool| {
            let mut body = json!({ "model": model, "messages": wire, "temperature": 0.6 });
            if with_tools && !tools.is_empty() {
                body["tools"] = json!(tools);
            }
            reqwest::Client::new().post(&url).json(&body).send()
        };

        let mut res = send(!tools.is_empty()).await?;
        if !res.status().is_success() && !tools.is_empty() {
            res = send(false).await?;
        }
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("local /chat/completions failed: {} {}", status, text);
        }
        let data: Value = res.json().await?;
        let message = data
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("message"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("local 応答に choices[0].message がありません"))?;
        Ok(from_wire(&message))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Local) } }
