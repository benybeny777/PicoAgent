// providers/local.rs - 同梱 llama-server（OpenAI 互換）を叩く既定プロバイダ。
// Ollama 不要。エンジンの起動とモデル取得は engine.rs が担う。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::openai::{from_wire, to_wire};
use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message};

pub struct Local;

pub struct LocalModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub url: &'static str,
    pub ctx: &'static str,
    pub recommended_vram_gb: u64,
    pub recommended_ram_gb: u64,
    pub download_size: &'static str,
    pub note: &'static str,
}

pub const LOCAL_MODEL_PRESETS: &[LocalModelPreset] = &[
    LocalModelPreset {
        id: "small",
        label: "軽量: Gemma 3 1B Q4（低VRAM/CPU向け）",
        url: "https://huggingface.co/unsloth/gemma-3-1b-it-GGUF/resolve/main/gemma-3-1b-it-Q4_K_M.gguf",
        ctx: "8192",
        recommended_vram_gb: 0,
        recommended_ram_gb: 8,
        download_size: "約0.8GB",
        note: "VRAMが少ないPCやCPU中心の環境向け。",
    },
    LocalModelPreset {
        id: "balanced",
        label: "標準: Gemma 3n E2B Q4（推奨）",
        url: "https://huggingface.co/unsloth/gemma-3n-E2B-it-GGUF/resolve/main/gemma-3n-E2B-it-Q4_K_M.gguf",
        ctx: "8192",
        recommended_vram_gb: 4,
        recommended_ram_gb: 16,
        download_size: "約3.0GB",
        note: "通常利用の推奨。4GB級VRAMまたは十分なRAMがあるPC向け。",
    },
    LocalModelPreset {
        id: "large",
        label: "高品質: Gemma 3n E4B Q4（VRAM 6GB以上推奨）",
        url: "https://huggingface.co/unsloth/gemma-3n-E4B-it-GGUF/resolve/main/gemma-3n-E4B-it-Q4_K_M.gguf",
        ctx: "8192",
        recommended_vram_gb: 6,
        recommended_ram_gb: 24,
        download_size: "約4.5GB",
        note: "品質優先。起動や応答は重くなる。",
    },
];

pub fn preset(id: &str) -> Option<&'static LocalModelPreset> {
    LOCAL_MODEL_PRESETS.iter().find(|preset| preset.id == id)
}

pub fn preset_url(id: &str) -> Option<&'static str> {
    preset(id).map(|preset| preset.url)
}

pub fn model_url(cfg: &Config) -> String {
    let preset_id = cfg.get("PICOAGENT_LOCAL_MODEL_PRESET", "balanced");
    if preset_id == "custom" {
        return cfg.get("PICOAGENT_MODEL_URL", LOCAL_MODEL_PRESETS[1].url);
    }
    // 設定画面「モデル」欄（PICOAGENT_MODEL）がプリセットIDなら、その選択を優先する。
    // （モデル欄での切り替えをそのまま取得URLへ反映し、未取得なら自動DLされる）
    let model = cfg.get("PICOAGENT_MODEL", "");
    if let Some(url) = preset_url(&model) {
        return url.to_string();
    }
    if let Some(url) = preset_url(&preset_id) {
        return url.to_string();
    }
    cfg.get("PICOAGENT_MODEL_URL", LOCAL_MODEL_PRESETS[1].url)
}

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
        "ローカル"
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_LOCAL_MODEL_PRESET",
                "ローカルモデル（auto/small/balanced/large/custom）",
                "text",
                "auto",
            ),
            Field::new(
                "PICOAGENT_MODEL_URL",
                "カスタムモデル GGUF の取得元 URL",
                "text",
                LOCAL_MODEL_PRESETS[1].url,
            ),
            Field::new("PICOAGENT_LOCAL_PORT", "ローカルエンジンのポート", "text", "11435"),
            Field::new("PICOAGENT_LOCAL_CTX", "コンテキスト長", "text", "8192"),
            Field::new(
                "PICOAGENT_LOCAL_NGL",
                "GPU へオフロードするレイヤ数（999=全レイヤ優先 / 0=CPU 固定）",
                "text",
                "999",
            ),
            Field::new(
                "PICOAGENT_LOCAL_GPU_DEVICE",
                "使用 GPU（空=既定先頭 / 複数 GPU は Vulkan1 等を指定）",
                "text",
                "",
            ),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get("PICOAGENT_MODEL", &cfg.get("PICOAGENT_LOCAL_MODEL_PRESET", "balanced"))
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/models", base_url(cfg));
        let local_presets: Vec<String> = LOCAL_MODEL_PRESETS
            .iter()
            .map(|preset| preset.id.to_string())
            .chain(std::iter::once("custom".to_string()))
            .collect();
        let res = match reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
        {
            Ok(res) => res,
            Err(_) => return Ok(local_presets),
        };
        if !res.status().is_success() {
            return Ok(local_presets);
        }
        let data: Value = res.json().await?;
        let mut out: Vec<String> = data
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for preset in local_presets {
            if !out.contains(&preset) {
                out.push(preset);
            }
        }
        Ok(out)
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
