// providers/local.rs - 同梱 llama-server（OpenAI 互換）を叩く既定プロバイダ。
// Ollama 不要。エンジンの起動とモデル取得は engine.rs が担う。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::openai::{from_wire, to_wire};
use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message};

#[path = "../engine_vision_asset.rs"]
pub mod vision_asset;

pub fn vision_enabled(cfg: &Config) -> bool {
    cfg.get_bool("PICOAGENT_LOCAL_VISION_ENABLED", false)
}

pub fn validate_vision_model(cfg: &Config) -> anyhow::Result<()> {
    anyhow::ensure!(
        preset_by_url(&model_url(cfg)).is_some_and(|preset| preset.id == "small"),
        "画像入力は Gemma 4 E2B（small）に対応しています。モデルを選び直してください"
    );
    Ok(())
}

/// 通常チャットとスキルの双方で、画像を黙って無視させない。
fn validate_image_input(cfg: &Config, messages: &[Message]) -> anyhow::Result<()> {
    if messages.iter().any(|message| {
        message
            .images
            .as_ref()
            .is_some_and(|images| !images.is_empty())
    }) {
        crate::engine::ensure_vision_available(cfg).map_err(|error| anyhow::anyhow!(
            "{error}。この会話には添付画像が含まれています。画像入力を有効にするか、画像を含まない会話で続けてください"
        ))?;
    }
    Ok(())
}

pub struct Local;

/// 計測値は公開Messageへ混ぜず、明示された診断sinkだけへ渡す。
#[derive(Debug, Clone, PartialEq)]
pub struct LocalTimings {
    pub cache_n: u64,
    pub prompt_n: u64,
    pub prompt_ms: f64,
    pub predicted_n: u64,
    pub predicted_ms: f64,
}

#[derive(Debug, Clone)]
pub struct TimingReport {
    pub stream: bool,
    pub completed: bool,
    pub timings: Result<Option<LocalTimings>, String>,
    pub elapsed: std::time::Duration,
    pub first_text: Option<std::time::Duration>,
}

pub type TimingSink = std::sync::Arc<dyn Fn(TimingReport) + Send + Sync>;
tokio::task_local! { static TIMING_SINK: TimingSink; }

pub async fn with_timings<F: std::future::Future>(future: F, sink: TimingSink) -> F::Output {
    TIMING_SINK.scope(sink, future).await
}

fn parse_timings(data: &Value) -> Result<Option<LocalTimings>, String> {
    let Some(value) = data.get("timings") else {
        return Ok(None);
    };
    let integer = |key| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("ローカル計測の{key}が非負整数ではありません"))
    };
    let duration = |key| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| format!("ローカル計測の{key}が有効な時間ではありません"))
    };
    Ok(Some(LocalTimings {
        cache_n: integer("cache_n")?,
        prompt_n: integer("prompt_n")?,
        prompt_ms: duration("prompt_ms")?,
        predicted_n: integer("predicted_n")?,
        predicted_ms: duration("predicted_ms")?,
    }))
}

pub(crate) struct TimingMeasurement {
    sink: TimingSink,
    started: std::time::Instant,
    report: TimingReport,
}
impl TimingMeasurement {
    pub(crate) fn new(stream: bool) -> Option<Self> {
        TIMING_SINK
            .try_with(|sink| Self {
                sink: sink.clone(),
                started: std::time::Instant::now(),
                report: TimingReport {
                    stream,
                    completed: false,
                    timings: Ok(None),
                    elapsed: std::time::Duration::ZERO,
                    first_text: None,
                },
            })
            .ok()
    }
    pub(crate) fn observe(&mut self, data: &Value) {
        // 最終イベントに値が無ければ未取得とし、途中イベントの値を流用しない。
        self.report.timings = parse_timings(data);
    }
    pub(crate) fn invalid_json(&mut self) {
        self.report.timings = Err("ローカル計測のストリームJSONが不正です".into());
    }
    pub(crate) fn first_text(&mut self) {
        if self.report.first_text.is_none() {
            self.report.first_text = Some(self.started.elapsed());
        }
    }
    pub(crate) fn complete(&mut self) {
        self.report.completed = true;
    }
}
impl Drop for TimingMeasurement {
    fn drop(&mut self) {
        self.report.elapsed = self.started.elapsed();
        (self.sink)(self.report.clone());
    }
}

fn chat_body(model: &str, messages: &[Message], tools: &[Value]) -> anyhow::Result<Value> {
    let mut body = json!({
        "model": model, "messages": to_wire(&normalize_for_local(messages)?), "temperature": 0.6
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    Ok(body)
}

fn structured_body(model: &str, messages: &[Message], schema: &Value) -> anyhow::Result<Value> {
    let mut body = chat_body(model, messages, &[])?;
    body["response_format"] = json!({
        "type": "json_schema",
        "json_schema": {"name": "structured_response", "strict": true, "schema": schema}
    });
    // 構造化結果には観測・判断の短い説明だけを返し、内部推論を含めない。
    body["temperature"] = json!(0);
    body["reasoning_effort"] = json!("none");
    body["chat_template_kwargs"] = json!({"enable_thinking": false});
    Ok(body)
}

async fn send_chat(cfg: &Config, body: &Value) -> anyhow::Result<Message> {
    let mut measurement = TimingMeasurement::new(false);
    let url = format!(
        "{}/chat/completions",
        base_url(cfg).map_err(anyhow::Error::msg)?
    );
    let res = crate::tools::http::client()
        .map_err(anyhow::Error::msg)?
        .post(&url)
        .json(body)
        .send()
        .await
        .map_err(super::http_error::send_error)?;
    let res = super::http_error::provider_response(
        cfg,
        "ローカルAIエンジン",
        "/chat/completions",
        Ok(res),
    )
    .await?;
    let data: Value = res.json().await.map_err(super::http_error::send_error)?;
    if let Some(measurement) = &mut measurement {
        measurement.observe(&data);
    }
    let message = data
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| anyhow::anyhow!("local 応答に choices[0].message がありません"))?;
    let message = from_wire(message);
    if let Some(measurement) = &mut measurement {
        if !message.content.is_empty() {
            measurement.first_text();
        }
        measurement.complete();
    }
    Ok(message)
}

/// 画像・テキスト共通のスキーマ制約付き応答。ツールや制約なしの再送は行わない。
pub async fn chat_structured(
    cfg: &Config,
    model: &str,
    messages: &[Message],
    schema: &Value,
) -> anyhow::Result<Message> {
    validate_image_input(cfg, messages)?;
    anyhow::ensure!(
        schema.is_object(),
        "応答スキーマはJSONオブジェクトで指定してください"
    );
    send_chat(cfg, &structured_body(model, messages, schema)?).await
}

#[derive(Clone)]
pub struct LocalModelPreset {
    pub id: &'static str,
    pub family: &'static str,
    pub size: &'static str,
    pub label: &'static str,
    pub url: &'static str,
    pub file_name: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub ctx: &'static str,
    pub recommended_vram_gb: u64,
    pub recommended_ram_gb: u64,
    pub download_size: &'static str,
    pub japanese_note: &'static str,
    pub note: &'static str,
}

/// ローカルエンジンのコンテキスト長。明示された不正値は補正せず返す。
pub fn context_length(cfg: &Config) -> Result<usize, String> {
    cfg.int(&crate::config::ints::LOCAL_CTX)
        .map(|value| value as usize)
}

pub const DEFAULT_LOCAL_MODEL_PRESET: &str = "lfm25_1_2b_jp";
const DEFAULT_LOCAL_MODEL_URL: &str = "https://huggingface.co/LiquidAI/LFM2.5-1.2B-JP-202606-GGUF/resolve/8c74801fdfe71394c59d3f519b86de305ff49f00/LFM2.5-1.2B-JP-202606-Q4_K_M.gguf";

pub const LOCAL_MODEL_PRESETS: &[LocalModelPreset] = &[
    LocalModelPreset {
        id: DEFAULT_LOCAL_MODEL_PRESET,
        family: "Liquid AI",
        size: "LFM2.5 1.2B 日本語",
        label: "軽量: LFM2.5 1.2B 日本語 Q4_K_M（既定）",
        url: DEFAULT_LOCAL_MODEL_URL,
        file_name: "LFM2.5-1.2B-JP-202606-Q4_K_M-8c74801f.gguf",
        size_bytes: 730_894_624,
        sha256: "23d4d4060cb06463563f6e8644bbbc7d1cb0bfe4cf21d5edf3ccc6f515fd44bb",
        ctx: "32768",
        recommended_vram_gb: 0,
        recommended_ram_gb: 8,
        download_size: "約0.731GB",
        japanese_note: "日本語特化",
        note: "軽い日本語会話向け。連続ツール利用を確認済み。人物や引用を取り違える場合がある。",
    },
    LocalModelPreset {
        id: "lfm25_2_6b",
        family: "Liquid AI",
        size: "LFM2.5 2.6B Agentic",
        label: "軽量: LFM2.5 2.6B QAD Q4",
        url: "https://huggingface.co/LiquidAI/LFM2.5-2.6B-GGUF/resolve/84022ce711b28455e8c4fc364ce68c00cf995875/LFM2.5-2.6B-QAD-Q4_0.gguf",
        file_name: "LFM2.5-2.6B-QAD-Q4_0-84022ce7.gguf",
        size_bytes: 1_593_894_944,
        sha256: "a247afd6414918eac8e520a9e6137dc271235461ecbe1180462221d5b8d40b03",
        ctx: "32768",
        recommended_vram_gb: 4,
        recommended_ram_gb: 8,
        download_size: "約1.59GB",
        japanese_note: "日本語対応",
        note: "量子化学習済みQAD版。軽さを保ちながら複数手順の指示とツール利用を優先する候補。",
    },
    LocalModelPreset {
        id: "small",
        family: "Gemma",
        size: "軽量 E2B",
        label: "軽量: Gemma 4 E2B Q4（低VRAM/CPU向け）",
        url: "https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/0314792d7f1f7e229411f620751375812bb9faf2/gemma-4-E2B-it-Q4_K_M.gguf",
        file_name: "gemma-4-E2B-it-Q4_K_M-0314792d.gguf",
        size_bytes: 3_106_738_272,
        sha256: "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8",
        ctx: "32768",
        recommended_vram_gb: 0,
        recommended_ram_gb: 8,
        download_size: "約3.1GB",
        japanese_note: "日本語対応",
        note: "VRAMが少ないPCやCPU中心の環境向け。",
    },
    LocalModelPreset {
        id: "balanced",
        family: "Gemma",
        size: "標準 E4B",
        label: "標準: Gemma 4 E4B Q4（推奨）",
        url: "https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF/resolve/bfc15c382204943c3a8fff0c750b94ae2364d7a3/gemma-4-E4B-it-Q4_K_M.gguf",
        file_name: "gemma-4-E4B-it-Q4_K_M-bfc15c38.gguf",
        size_bytes: 4_977_171_584,
        sha256: "85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87",
        ctx: "32768",
        recommended_vram_gb: 4,
        recommended_ram_gb: 16,
        download_size: "約5.0GB",
        japanese_note: "日本語対応",
        note: "通常利用の推奨。4GB級VRAMまたは十分なRAMがあるPC向け。",
    },
    LocalModelPreset {
        id: "large",
        family: "Gemma",
        size: "高品質 12B",
        label: "高品質: Gemma 4 12B Q4（VRAM 8GB以上推奨）",
        url: "https://huggingface.co/unsloth/gemma-4-12b-it-GGUF/resolve/fc034cfff751157913579611efad8462ac1be606/gemma-4-12b-it-Q4_K_M.gguf",
        file_name: "gemma-4-12b-it-Q4_K_M-fc034cff.gguf",
        size_bytes: 7_121_861_440,
        sha256: "0a270ec9fe6b34f4a0d33992b6135117b484ebc4766ab76b51d4ae8c457e4c42",
        ctx: "32768",
        recommended_vram_gb: 8,
        recommended_ram_gb: 24,
        download_size: "約7.1GB",
        japanese_note: "日本語対応",
        note: "品質優先。起動や応答は重くなる。",
    },
    LocalModelPreset {
        id: "qwen35_4b",
        family: "Qwen",
        size: "Qwen3.5 4B",
        label: "選択: Qwen3.5 4B Q4（軽量・日本語向け）",
        url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/e87f176479d0855a907a41277aca2f8ee7a09523/Qwen3.5-4B-Q4_K_M.gguf",
        file_name: "Qwen3.5-4B-Q4_K_M-e87f1764.gguf",
        size_bytes: 2_740_937_888,
        sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
        ctx: "32768",
        recommended_vram_gb: 4,
        recommended_ram_gb: 12,
        download_size: "約2.7GB",
        japanese_note: "日本語に強い",
        note: "軽さ、日本語、指示追従のバランスを重視する場合の候補。",
    },
    LocalModelPreset {
        id: "qwen35_9b",
        family: "Qwen",
        size: "Qwen3.5 9B",
        label: "選択: Qwen3.5 9B Q4（高品質・日本語向け）",
        url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/3885219b6810b007914f3a7950a8d1b469d598a5/Qwen3.5-9B-Q4_K_M.gguf",
        file_name: "Qwen3.5-9B-Q4_K_M-3885219b.gguf",
        size_bytes: 5_680_522_464,
        sha256: "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
        ctx: "32768",
        recommended_vram_gb: 6,
        recommended_ram_gb: 16,
        download_size: "約5.7GB",
        japanese_note: "日本語に強い",
        note: "日本語、指示追従、ツール利用の品質を優先する場合の候補。",
    },
    LocalModelPreset {
        id: "gpt_oss_20b",
        family: "OpenAI",
        size: "gpt-oss-20b",
        label: "選択: OpenAI gpt-oss-20b MXFP4（高性能PC向け）",
        url: "https://huggingface.co/ggml-org/gpt-oss-20b-GGUF/resolve/ef9b12f2ff56c69cf32153a02784e7a3c88bf524/gpt-oss-20b-MXFP4.gguf",
        file_name: "gpt-oss-20b-MXFP4-ef9b12f2.gguf",
        size_bytes: 12_109_566_624,
        sha256: "27cd6c432c7672cb812a92f611cf3ba7bbc35928262bb1e1253ff4ee6ae35901",
        ctx: "32768",
        recommended_vram_gb: 16,
        recommended_ram_gb: 32,
        download_size: "約12.1GB",
        japanese_note: "日本語対応",
        note: "OpenAIの公開モデル。推論とツール利用を重視する高性能PC向け。",
    },
    LocalModelPreset {
        id: "phi4mini",
        family: "Phi",
        size: "Phi-4 mini Instruct",
        label: "選択: Phi-4 mini Instruct Q4（軽量）",
        url: "https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF/resolve/78eb92a46fc37e6b524df991ed9aca9bc6aa7b80/Phi-4-mini-instruct-Q4_K_M.gguf",
        file_name: "Phi-4-mini-instruct-Q4_K_M-78eb92a4.gguf",
        size_bytes: 2_491_874_272,
        sha256: "88c00229914083cd112853aab84ed51b87bdf6b9ce42f532d8c85c7c63b1730a",
        ctx: "32768",
        recommended_vram_gb: 4,
        recommended_ram_gb: 12,
        download_size: "約2.5GB",
        japanese_note: "日本語対応",
        note: "小型モデルで推論と指示追従を重視する場合の候補。",
    },
    LocalModelPreset {
        id: "llmjp4_8b",
        family: "LLM-jp",
        size: "LLM-jp-4 8B Thinking",
        label: "選択: LLM-jp-4 8B Thinking Q4（日本語推論向け）",
        url: "https://huggingface.co/llm-jp/llm-jp-4-8b-thinking-gguf/resolve/2cc04f0cf7f2bfccca0298aaecfdf8272228ea85/llm-jp-4-8b-thinking-Q4_K_M.gguf",
        file_name: "llm-jp-4-8b-thinking-Q4_K_M-2cc04f0c.gguf",
        size_bytes: 5_304_882_240,
        sha256: "b8a1b3ef962d95dfaa02c241e0d2376e028aaddcda83191cc232e95ed2b14ae8",
        ctx: "32768",
        recommended_vram_gb: 6,
        recommended_ram_gb: 16,
        download_size: "約5.3GB",
        japanese_note: "日本語特化・Thinking",
        note: "国立情報学研究所LLM-jpの日本語推論モデル。応答品質を優先する場合の候補。",
    },
];

const RETIRED_MODEL_FILES: &[&str] = &[
    "DeepSeek-R1-Distill-Qwen-7B-Q4_K_M.gguf",
    "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
    "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
    "Mistral-7B-Instruct-v0.3-Q4_K_M.gguf",
    "Phi-3.5-mini-instruct-Q4_K_M.gguf",
    "Qwen2.5-7B-Instruct-Q4_K_M.gguf",
    "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    "gemma-3-1b-it-Q4_K_M.gguf",
    "gemma-3n-E2B-it-Q4_K_M.gguf",
    "gemma-3n-E4B-it-Q4_K_M.gguf",
];

/// revision入り保存名へ移行済み、またはプリセットから廃止した管理モデル名。
/// 現行版の検証後にだけ削除し、利用者指定のcustomファイルは対象にしない。
///
/// `active` には「いま選ばれているプリセット」だけを渡す。全プリセットの旧保存名を返すと、
/// 利用者が別プリセットとして持っている実体（1件で数GB）まで起動時に消えて再取得になる。
/// 廃止済みプリセット（`RETIRED_MODEL_FILES`）はもう選べないため、選択と無関係に掃除してよい。
pub fn obsolete_model_file_names(active: Option<&LocalModelPreset>) -> Vec<&'static str> {
    let mut names = RETIRED_MODEL_FILES.to_vec();
    if let Some(preset) = active {
        if let Some(name) = preset.url.rsplit('/').next() {
            if name != preset.file_name && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// 旧版で保存されたプリセットIDを、現在提供する同等負荷帯のモデルへ移行する。
pub fn canonical_preset_id(id: &str) -> &str {
    match id {
        "qwen3" | "llama32_3b" => "qwen35_4b",
        "qwen25_7b" | "llama31_8b" | "mistral7b" => "qwen35_9b",
        "phi35mini" => "phi4mini",
        _ => id,
    }
}

pub fn preset(id: &str) -> Option<&'static LocalModelPreset> {
    let id = canonical_preset_id(id);
    LOCAL_MODEL_PRESETS.iter().find(|preset| preset.id == id)
}

/// 設定画面で一般利用者へ候補として案内してよいプリセットだけを返す。
/// LLM-jp-4 Thinkingは既存設定との互換用に残すが、連続ツール利用を
/// 実エンジンで確認できないため新規候補へ出さない。
pub fn settings_candidate_presets() -> impl Iterator<Item = &'static LocalModelPreset> {
    LOCAL_MODEL_PRESETS
        .iter()
        .filter(|preset| preset.id != "llmjp4_8b")
}

pub fn preset_url(id: &str) -> Option<&'static str> {
    preset(id).map(|preset| preset.url)
}

pub fn preset_by_url(url: &str) -> Option<&'static LocalModelPreset> {
    LOCAL_MODEL_PRESETS.iter().find(|preset| preset.url == url)
}

/// GGUF ファイル名から、対応するプリセットのラベルを引く（ダウンロード済みモデル一覧の表示用）。
/// 監査済みrevisionを含む保存名が一致するプリセットを返す。該当が無ければ None。
pub fn label_for_filename(file_name: &str) -> Option<&'static str> {
    LOCAL_MODEL_PRESETS
        .iter()
        .find(|preset| preset.file_name == file_name)
        .map(|preset| preset.label)
}

pub fn model_url(cfg: &Config) -> String {
    let preset_id = cfg.get("PICOAGENT_LOCAL_MODEL_PRESET", DEFAULT_LOCAL_MODEL_PRESET);
    if preset_id == "custom" {
        return cfg.get("PICOAGENT_MODEL_URL", DEFAULT_LOCAL_MODEL_URL);
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
    cfg.get("PICOAGENT_MODEL_URL", DEFAULT_LOCAL_MODEL_URL)
}

/// 管理プリセットだけに適用する履歴テンプレートの対象。custom の公式URL指定も除外する。
pub(crate) fn history_template_preset(cfg: &Config) -> Option<&'static LocalModelPreset> {
    if cfg.get("PICOAGENT_LOCAL_MODEL_PRESET", DEFAULT_LOCAL_MODEL_PRESET) == "custom" {
        return None;
    }
    preset_by_url(&model_url(cfg)).filter(|preset| preset.id == "lfm25_1_2b_jp")
}

// Gemma などのチャットテンプレートは user/assistant の厳密な交互（かつ user 始まり）を要求し、
// 崩れると 400「Conversation roles must alternate」を返す。自発雑談や履歴復元で assistant が
// 連続・先頭になり得るため、送信直前に安全な形へ正規化する:
//   - 非公開systemは共通所有者で位置を保ってuserへ畳み、固定systemだけを先頭へまとめる
//   - 先頭の assistant は user が来るまで捨てる（user 始まりにする）
//   - 連続する同role（user/user, assistant/assistant）は内容を連結して1つに畳む
// tool 呼び出しを含むメッセージ（agent ループ中）はそのまま通す。
pub(crate) fn normalize_for_local(messages: &[Message]) -> anyhow::Result<Vec<Message>> {
    let messages = crate::agent::context::fold_private_context(messages)?;
    let mut system_parts: Vec<String> = Vec::new();
    let mut convo: Vec<Message> = Vec::new();
    for m in &messages {
        if m.role == "system" {
            if !m.content.is_empty() {
                system_parts.push(m.content.clone());
            }
        } else {
            convo.push(m.clone());
        }
    }
    // user 始まりにするため、先頭の純粋な assistant 発話は落とす。
    while convo
        .first()
        .is_some_and(|m| m.role == "assistant" && m.tool_calls.is_none())
    {
        convo.remove(0);
    }
    let mut merged: Vec<Message> = Vec::new();
    for m in convo {
        if let Some(last) = merged.last_mut() {
            let mergeable = last.role == m.role
                && (m.role == "user" || m.role == "assistant")
                && last.tool_calls.is_none()
                && m.tool_calls.is_none()
                && last.tool_call_id.is_none()
                && m.tool_call_id.is_none();
            if mergeable {
                if !m.content.is_empty() {
                    if !last.content.is_empty() {
                        last.content.push_str("\n\n");
                    }
                    last.content.push_str(&m.content);
                }
                if let Some(images) = m.images {
                    last.images.get_or_insert_with(Vec::new).extend(images);
                }
                continue;
            }
        }
        merged.push(m);
    }
    let mut out: Vec<Message> = Vec::new();
    if !system_parts.is_empty() {
        out.push(Message::new("system", system_parts.join("\n\n")));
    }
    out.extend(merged);
    Ok(out)
}

pub fn port(cfg: &Config) -> Result<u16, String> {
    cfg.int(&crate::config::ints::LOCAL_PORT)
        .map(|value| value as u16)
}
fn base_url(cfg: &Config) -> Result<String, String> {
    Ok(format!("http://127.0.0.1:{}/v1", port(cfg)?))
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
                "ローカルモデル（PC向け候補/Liquid AI/Gemma/Qwen/OpenAI/Phi/LLM-jp/custom）",
                "text",
                "auto",
            ),
            Field::new(
                "PICOAGENT_MODEL_URL",
                "カスタムモデル GGUF の取得元 URL",
                "text",
                DEFAULT_LOCAL_MODEL_URL,
            ),
            Field::int(&crate::config::ints::LOCAL_PORT),
            Field::int(&crate::config::ints::LOCAL_CTX),
            Field::new(
                "PICOAGENT_LOCAL_VISION_ENABLED",
                "ローカル画像入力（Gemma 4 E2B・追加資産の明示導入と再起動が必要）",
                "checkbox",
                "false",
            ),
            Field::int(&crate::config::ints::LOCAL_NGL),
            Field::new(
                "PICOAGENT_LOCAL_GPU_DEVICE",
                "使用 GPU（空=既定先頭 / 複数 GPU は Vulkan1 等を指定）",
                "text",
                "",
            ),
        ]
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get(
            "PICOAGENT_MODEL",
            &cfg.get("PICOAGENT_LOCAL_MODEL_PRESET", DEFAULT_LOCAL_MODEL_PRESET),
        )
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/models", base_url(cfg).map_err(anyhow::Error::msg)?);
        let local_presets: Vec<String> = LOCAL_MODEL_PRESETS
            .iter()
            .map(|preset| preset.id.to_string())
            .chain(std::iter::once("custom".to_string()))
            .collect();
        let res = crate::tools::http::client()
            .map_err(anyhow::Error::msg)?
            .get(&url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map_err(super::http_error::send_error)?;
        let res =
            super::http_error::provider_response(cfg, "ローカルAIエンジン", "/models", Ok(res))
                .await?;
        let data: Value = res.json().await.map_err(super::http_error::send_error)?;
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

    async fn is_up(&self, cfg: &Config) -> Result<bool, String> {
        let url = format!("http://127.0.0.1:{}/health", port(cfg)?);
        super::http_error::managed_health_probe(
            cfg,
            "ローカルAIエンジン",
            "/health",
            crate::tools::http::client()?
                .get(url)
                // Windowsはループバックの接続拒否を返すまで約2秒かかる。
                // 先に期限切れへ変換すると、停止中を判別できず冷間起動を止めてしまう。
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
        validate_image_input(cfg, messages)?;
        // tools 付きで送り、失敗しても tools を外して再送しない。外すとツールが使えない
        // ことを誰にも伝えないまま機能だけが落ち、モデルが本文へ `web_search{...}` のような
        // 呼び出しを書いて利用者に見せてしまう。失敗は理由ごと上位へ返す（AGENTS.md の
        // 無言フォールバック禁止）。同梱エンジンは --jinja 起動で、既定の LFM2.5 も Gemma 4 も
        // ツール呼び出しに対応することを実機で確認している。
        send_chat(cfg, &chat_body(model, messages, tools)?).await
    }

    /// ストリーミング chat。同梱 llama-server は OpenAI 互換の SSE に対応しているため、
    /// openai プロバイダと共通の stream_chat でトークンを逐次受信する（UI の逐次表示・
    /// 先行読み上げが既定のローカルエンジンでも効く）。
    async fn chat_stream(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        on_chunk: std::sync::Arc<dyn Fn(String) + Send + Sync>,
    ) -> anyhow::Result<Message> {
        use std::sync::atomic::{AtomicBool, Ordering};

        validate_image_input(cfg, messages)?;
        let url = format!(
            "{}/chat/completions",
            base_url(cfg).map_err(anyhow::Error::msg)?
        );
        let normalized = normalize_for_local(messages)?;
        let wire = to_wire(&normalized);
        let measurement = TimingMeasurement::new(true);
        let make_body = |with_tools: bool| {
            let mut body =
                json!({ "model": model, "messages": wire, "temperature": 0.6, "stream": true });
            if measurement.is_some() {
                body["stream_options"] = json!({"include_usage":true});
            }
            if with_tools && !tools.is_empty() {
                body["tools"] = json!(tools);
            }
            body
        };
        // chat() と同じ方針: tools を外した再送はしない。
        // 何が起きたかを追えるよう、チャンクを流し始めていたかだけはエラーへ添える。
        let emitted = std::sync::Arc::new(AtomicBool::new(false));
        let probe: std::sync::Arc<dyn Fn(String) + Send + Sync> = {
            let emitted = emitted.clone();
            let inner = on_chunk.clone();
            std::sync::Arc::new(move |chunk: String| {
                emitted.store(true, Ordering::Relaxed);
                inner(chunk);
            })
        };
        super::openai::stream_chat_measured(
            cfg,
            &url,
            "",
            make_body(!tools.is_empty()),
            probe,
            measurement,
        )
        .await
        .map_err(|error| {
            if emitted.load(Ordering::Relaxed) {
                error
            } else {
                error.context(format!(
                    "ツール定義 {} 件つきの要求で失敗しました",
                    tools.len()
                ))
            }
        })
    }
}

inventory::submit! { ProviderReg { make: || Box::new(Local) } }

#[cfg(test)]
mod tests {
    #[test]
    fn 日本語履歴テンプレートは実効プリセットだけを対象にしカスタムを除外する() {
        for (selection, model, expected) in [
            ("auto", "", true),
            ("lfm25_2_6b", "", false),
            ("lfm25_1_2b_jp", "lfm25_1_2b_jp", true),
            ("lfm25_1_2b_jp", "lfm25_2_6b", false),
            ("lfm25_2_6b", "lfm25_1_2b_jp", true),
            ("custom", "lfm25_1_2b_jp", false),
            ("lfm25_2_6b", "lfm25_2_6b", false),
        ] {
            let cfg = crate::config::Config::new();
            cfg.set("PICOAGENT_LOCAL_MODEL_PRESET", selection);
            cfg.set("PICOAGENT_MODEL", model);
            cfg.set(
                "PICOAGENT_MODEL_URL",
                super::preset("lfm25_1_2b_jp").unwrap().url,
            );
            assert_eq!(
                super::history_template_preset(&cfg).is_some(),
                expected,
                "{selection}/{model}"
            );
        }
    }
    use super::{
        canonical_preset_id, normalize_for_local, obsolete_model_file_names, preset,
        settings_candidate_presets, DEFAULT_LOCAL_MODEL_PRESET, LOCAL_MODEL_PRESETS,
        RETIRED_MODEL_FILES,
    };
    use crate::message::Message;

    #[test]
    fn 内部systemは元の位置へ畳み前ターンの画像と呼出pairとprefixを維持する() {
        let (before, after, raw) = crate::agent::context::protocol_fixture();
        let normalized = normalize_for_local(&raw).unwrap();
        assert_eq!(
            serde_json::to_value(&normalized).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        let earlier = normalize_for_local(&before).unwrap();
        assert_eq!(
            serde_json::to_value(&earlier).unwrap(),
            serde_json::to_value(&normalized[..earlier.len()]).unwrap()
        );
        assert_eq!(normalized[0].content, before[0].content);
        assert_eq!(normalized[1].images, before[1].images);
        assert_eq!(normalized[2].tool_calls.as_ref().unwrap()[0].id, "pair-1");
        assert_eq!(normalized[3].tool_call_id, before[3].tool_call_id);
        for (request, original) in [
            (&normalized[1], &raw[2]),
            (normalized.last().unwrap(), raw.last().unwrap()),
        ] {
            assert!(request.content.contains(&format!(
                "--- 今回の利用者の質問開始 ---\n{}\n--- 今回の利用者の質問終了 ---",
                original.content
            )));
        }
        // 共通fixtureの引用注釈は、原文区間の後でもJSON全体を無加工で保持する。
        let annotations = |message: &Message| -> serde_json::Value {
            serde_json::from_str(message.content.lines().last().unwrap()).unwrap()
        };
        let actual = annotations(normalized.last().unwrap());
        assert_eq!(actual, annotations(after.last().unwrap()));
        assert_eq!(actual[0]["speaker"], "あおい");
        assert_eq!(actual[0]["recipient"], "けん");
    }

    type TimingReports = std::sync::Arc<std::sync::Mutex<Vec<super::TimingReport>>>;
    fn timing_sink() -> (super::TimingSink, TimingReports) {
        let reports = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stored = reports.clone();
        (
            std::sync::Arc::new(move |report| stored.lock().unwrap().push(report)),
            reports,
        )
    }

    fn timing_value() -> serde_json::Value {
        serde_json::json!({"cache_n":0,"prompt_n":4294967296_u64,"prompt_ms":1.5,"predicted_n":2,"predicted_ms":3.0})
    }

    fn timing_stream_server(body: &str) -> crate::providers::compatible::test_server::Server {
        crate::providers::compatible::test_server::Server::raw(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes())
    }

    #[test]
    fn 計測の欠損とゼロと不正を区別し本文や秘密を理由へ出さない() {
        use serde_json::json;
        assert_eq!(super::parse_timings(&json!({})).unwrap(), None);
        let valid = super::parse_timings(&json!({"timings":timing_value()}))
            .unwrap()
            .unwrap();
        assert_eq!(valid.cache_n, 0);
        assert_eq!(valid.prompt_n, 4294967296);
        for value in [json!(null), json!({}), json!({"cache_n":"secret-body"}), {
            let mut value = timing_value();
            value["prompt_ms"] = json!(-1);
            value
        }] {
            let reason = super::parse_timings(&json!({"timings":value})).unwrap_err();
            assert!(!reason.contains("secret-body"));
        }
    }

    #[tokio::test]
    async fn ローカル非stream計測は診断だけへ渡し公開メッセージを変えない() {
        use crate::providers::compatible::test_server::Server;
        use serde_json::json;
        let server = Server::json(
            200,
            &json!({"choices":[{"message":{"role":"assistant","content":"回答"}}],"timings":timing_value()}),
        );
        let cfg = crate::config::Config::new();
        cfg.set(
            "PICOAGENT_LOCAL_PORT",
            &reqwest::Url::parse(&server.url)
                .unwrap()
                .port()
                .unwrap()
                .to_string(),
        );
        let (sink, reports) = timing_sink();
        let message = super::with_timings(super::send_chat(&cfg, &json!({"messages":[]})), sink)
            .await
            .unwrap();
        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].completed && !reports[0].stream);
        assert_eq!(
            reports[0]
                .timings
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .prompt_n,
            4294967296
        );
        assert_eq!(message.content, "回答");
        assert!(serde_json::to_value(message)
            .unwrap()
            .get("timings")
            .is_none());
        assert_eq!(server.finish().len(), 1);
    }

    #[tokio::test]
    async fn 最終streamの空choicesと欠損と不正と途中終了を別々に記録する() {
        use serde_json::json;
        for (tail, done, measured, invalid) in [
            (
                json!({"choices":[],"timings":timing_value()}),
                true,
                true,
                false,
            ),
            (
                json!({"choices":[{"delta":{},"finish_reason":"stop"}],"timings":timing_value()}),
                true,
                true,
                false,
            ),
            (json!({"choices":[]}), true, false, false),
            (
                json!({"choices":[],"timings":{"cache_n":"secret"}}),
                true,
                false,
                true,
            ),
            (
                json!({"choices":[],"timings":timing_value()}),
                false,
                true,
                false,
            ),
        ] {
            let body = format!(
                "data: {}\n\ndata: {tail}\n\n{}",
                json!({"choices":[{"delta":{"content":"回答"}}],"timings":timing_value()}),
                if done { "data: [DONE]\n\n" } else { "" }
            );
            let server = timing_stream_server(&body);
            let cfg = crate::config::Config::new();
            let (sink, reports) = timing_sink();
            let message = super::with_timings(
                async {
                    super::super::openai::stream_chat_measured(
                        &cfg,
                        &server.url,
                        "",
                        json!({}),
                        std::sync::Arc::new(|_| {}),
                        super::TimingMeasurement::new(true),
                    )
                    .await
                },
                sink,
            )
            .await
            .unwrap();
            let reports = reports.lock().unwrap();
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].completed, done);
            assert_eq!(reports[0].timings.is_err(), invalid);
            if !invalid {
                assert_eq!(reports[0].timings.as_ref().unwrap().is_some(), measured);
            }
            assert!(reports[0].first_text.is_some());
            assert_eq!(message.content, "回答");
            assert!(serde_json::to_value(message)
                .unwrap()
                .get("timings")
                .is_none());
            assert_eq!(server.finish().len(), 1);
        }
    }

    #[tokio::test]
    async fn 他プロバイダの既存stream入口は計測スコープ内でもローカル計測を出さない() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"回答\"}}]}\n\ndata: [DONE]\n\n";
        let server = timing_stream_server(body);
        let cfg = crate::config::Config::new();
        let (sink, reports) = timing_sink();
        let message = super::with_timings(
            super::super::openai::stream_chat(
                &cfg,
                &server.url,
                "",
                serde_json::json!({}),
                std::sync::Arc::new(|_| {}),
            ),
            sink,
        )
        .await
        .unwrap();
        assert_eq!(message.content, "回答");
        assert!(reports.lock().unwrap().is_empty());
        assert_eq!(server.finish().len(), 1);
    }

    #[tokio::test]
    async fn ローカルstreamの最終usage指定は計測sink有効時だけ追加する() {
        use super::Provider;
        use serde_json::json;
        for measured in [false, true] {
            let body = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{"content":"回答"}}]}),
                json!({"choices":[],"usage":{},"timings":timing_value()})
            );
            let server = timing_stream_server(&body);
            let cfg = crate::config::Config::new();
            cfg.set(
                "PICOAGENT_LOCAL_PORT",
                &reqwest::Url::parse(&server.url)
                    .unwrap()
                    .port()
                    .unwrap()
                    .to_string(),
            );
            let messages = [Message::new("user", "質問")];
            let (sink, reports) = timing_sink();
            let provider = super::Local;
            let future =
                provider.chat_stream(&cfg, "test", &messages, &[], std::sync::Arc::new(|_| {}));
            let message = if measured {
                super::with_timings(future, sink).await
            } else {
                future.await
            }
            .unwrap();
            assert_eq!(message.content, "回答");
            let requests = server.finish();
            assert_eq!(requests.len(), 1);
            let request: serde_json::Value =
                serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(
                request.get("stream_options"),
                measured.then(|| json!({"include_usage":true})).as_ref()
            );
            let reports = reports.lock().unwrap();
            assert_eq!(reports.len(), usize::from(measured));
            if measured {
                assert!(reports[0].completed && reports[0].timings.as_ref().unwrap().is_some());
            }
        }
    }

    #[tokio::test]
    async fn future取消も未完了を一度記録し次の要求へ計測器を持ち越さない() {
        let (sink, reports) = timing_sink();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(super::with_timings(
            async {
                let _measurement = super::TimingMeasurement::new(true).unwrap();
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            },
            sink,
        ));
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].completed);
        assert!(reports[0].timings.as_ref().unwrap().is_none());
        assert!(super::TimingMeasurement::new(true).is_none());
    }

    #[tokio::test]
    async fn 不正ポートは疎通失敗や既定ポートに置き換えない() {
        use super::Provider;
        let cfg = crate::config::Config::new();
        for value in ["0", "65536", "bad", "1.5", "-1"] {
            cfg.set("PICOAGENT_LOCAL_PORT", value);
            assert!(super::Local
                .is_up(&cfg)
                .await
                .unwrap_err()
                .contains("PICOAGENT_LOCAL_PORT"));
            assert!(super::Local
                .list_models(&cfg)
                .await
                .unwrap_err()
                .to_string()
                .contains("PICOAGENT_LOCAL_PORT"));
        }
    }

    #[tokio::test]
    async fn 停止中と起動準備中のローカルエンジンは未準備として返す() {
        use super::Provider;
        use crate::providers::compatible::test_server::Server;
        let cfg = crate::config::Config::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        cfg.set("PICOAGENT_LOCAL_PORT", &listener.local_addr().unwrap().port().to_string());
        drop(listener);
        assert!(!super::Local.is_up(&cfg).await.unwrap());
        for status in [200, 503, 401, 500] {
            let server = Server::json(status, &serde_json::json!({"status":"loading model"}));
            let port = server.url.rsplit(':').next().unwrap().trim_end_matches('/');
            cfg.set("PICOAGENT_LOCAL_PORT", port);
            let result = super::Local.is_up(&cfg).await;
            match status {
                200 => assert!(result.unwrap()),
                503 => assert!(!result.unwrap()),
                _ => assert!(result.unwrap_err().contains(&format!("HTTP {status}"))),
            }
            assert_eq!(server.finish().len(), 1);
        }
        let server = Server::raw(Vec::new());
        let port = server.url.rsplit(':').next().unwrap().trim_end_matches('/');
        cfg.set("PICOAGENT_LOCAL_PORT", port);
        assert!(super::Local.is_up(&cfg).await.is_err());
        assert_eq!(server.finish().len(), 1);
    }

    #[test]
    fn 構造化応答は画像を保持しツールを付けず正しい階層へスキーマを渡す() {
        let schema = serde_json::json!({"type":"object","additionalProperties":false,
            "properties":{"label":{"type":"string"}},"required":["label"]});
        let messages = [
            Message::new("user", "説明してください"),
            Message::user_with_images("画像", vec!["data:image/png;base64,AA".into()]),
        ];
        let body = super::structured_body("small", &messages, &schema).unwrap();
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            body["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AA"
        );
        assert!(body.get("tools").is_none());
        assert_eq!(body["reasoning_effort"], "none");
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        let tools = [serde_json::json!({"type":"function","function":{"name":"test"}})];
        let normal = super::chat_body("small", &messages, &tools).unwrap();
        assert_eq!(normal["messages"], body["messages"]);
        assert_eq!(normal["tools"], serde_json::json!(tools));
        assert!(normal.get("response_format").is_none());
        assert!(normal.get("reasoning_effort").is_none());
        assert!(normal.get("chat_template_kwargs").is_none());
    }

    #[test]
    fn 通常チャットでも画像未準備を拒否しテキストのみは利用できる() {
        let cfg = crate::config::Config::new();
        assert!(super::validate_image_input(&cfg, &[Message::new("user", "こんにちは")]).is_ok());
        let image =
            Message::user_with_images("画像を説明して", vec!["data:image/png;base64,AA".into()]);
        assert!(
            super::validate_image_input(&cfg, std::slice::from_ref(&image))
                .unwrap_err()
                .to_string()
                .contains("画像入力が無効")
        );
        cfg.set("PICOAGENT_LOCAL_MODEL_PRESET", "small");
        cfg.set("PICOAGENT_LOCAL_VISION_ENABLED", "true");
        assert!(super::validate_image_input(&cfg, &[image])
            .unwrap_err()
            .to_string()
            .contains("起動が完了していません"));
    }

    #[test]
    fn 画像入力は明示有効化と対応モデルを必要とする() {
        let cfg = crate::config::Config::new();
        assert!(!super::vision_enabled(&cfg));
        assert!(super::validate_vision_model(&cfg).is_err());
        cfg.set("PICOAGENT_LOCAL_MODEL_PRESET", "small");
        assert!(super::validate_vision_model(&cfg).is_ok());
        cfg.set("PICOAGENT_MODEL", "balanced");
        assert!(super::validate_vision_model(&cfg).is_err());
        cfg.set("PICOAGENT_MODEL", "small");
        cfg.set("PICOAGENT_LOCAL_VISION_ENABLED", "true");
        assert!(super::vision_enabled(&cfg));
        assert!(super::validate_vision_model(&cfg).is_ok());
    }

    #[test]
    fn 連続する利用者メッセージの添付画像を順番どおり保持する() {
        let messages = vec![
            Message::new("user", "説明"),
            Message::user_with_images("操作前", vec!["data:image/png;base64,AA".into()]),
            Message::user_with_images("操作後", vec!["data:image/png;base64,BB".into()]),
        ];
        let normalized = normalize_for_local(&messages).unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].content, "説明\n\n操作前\n\n操作後");
        assert_eq!(
            normalized[0].images.as_ref().unwrap(),
            &vec![
                "data:image/png;base64,AA".to_string(),
                "data:image/png;base64,BB".to_string(),
            ]
        );
    }

    #[test]
    fn 全プリセットと設定画面のコンテキスト既定は一致する() {
        use super::{Local, Provider};
        let field = Local
            .config_schema()
            .into_iter()
            .find(|field| field.key == "PICOAGENT_LOCAL_CTX")
            .expect("コンテキスト長は設定画面で変更できる");
        assert_eq!(field.placeholder, "32768");
        for preset in LOCAL_MODEL_PRESETS {
            assert_eq!(
                preset.ctx, field.placeholder,
                "{}のコンテキスト長が既定と違う",
                preset.id
            );
        }
    }

    #[test]
    fn 追加モデルは現行の量子化ggufだけを公開する() {
        let ids: Vec<&str> = LOCAL_MODEL_PRESETS.iter().map(|preset| preset.id).collect();
        assert_eq!(
            ids,
            [
                "lfm25_1_2b_jp",
                "lfm25_2_6b",
                "small",
                "balanced",
                "large",
                "qwen35_4b",
                "qwen35_9b",
                "gpt_oss_20b",
                "phi4mini",
                "llmjp4_8b",
            ]
        );
        assert_eq!(DEFAULT_LOCAL_MODEL_PRESET, "lfm25_1_2b_jp");
        assert_eq!(
            preset(DEFAULT_LOCAL_MODEL_PRESET).unwrap().url,
            super::DEFAULT_LOCAL_MODEL_URL
        );
        assert_eq!(
            settings_candidate_presets()
                .map(|preset| preset.id)
                .collect::<Vec<_>>(),
            [
                "lfm25_1_2b_jp",
                "lfm25_2_6b",
                "small",
                "balanced",
                "large",
                "qwen35_4b",
                "qwen35_9b",
                "gpt_oss_20b",
                "phi4mini",
            ]
        );
        for preset in LOCAL_MODEL_PRESETS {
            assert!(preset.url.ends_with(".gguf"));
            assert!(preset.file_name.ends_with(".gguf"));
            assert!(preset.url.contains("/resolve/") && !preset.url.contains("/resolve/main/"));
            assert!(preset.size_bytes > 0);
            assert_eq!(preset.sha256.len(), 64);
            assert!(!preset.japanese_note.is_empty());
            let file_name = preset.url.rsplit('/').next().unwrap_or("");
            let quantized = [
                "-Q2_", "-Q3_", "-Q4_", "-Q5_", "-Q6_", "-Q8_", "-IQ", "-MXFP", "-NVFP", "-UD-",
            ]
            .iter()
            .any(|marker| file_name.contains(marker));
            assert!(quantized, "量子化形式が明示されていません: {}", preset.url);
        }
    }

    #[test]
    fn 旧プリセットidを同等負荷帯の現行モデルへ移行する() {
        assert_eq!(canonical_preset_id("qwen3"), "qwen35_4b");
        assert_eq!(canonical_preset_id("qwen25_7b"), "qwen35_9b");
        assert_eq!(canonical_preset_id("llama32_3b"), "qwen35_4b");
        assert_eq!(canonical_preset_id("llama31_8b"), "qwen35_9b");
        assert_eq!(canonical_preset_id("mistral7b"), "qwen35_9b");
        assert_eq!(canonical_preset_id("phi35mini"), "phi4mini");
        assert_eq!(preset("mistral7b").map(|item| item.id), Some("qwen35_9b"));
    }

    #[test]
    fn 旧モデル名は現行revision名を含めず重複しない() {
        for active in LOCAL_MODEL_PRESETS {
            let names = obsolete_model_file_names(Some(active));
            let unique: std::collections::HashSet<_> = names.iter().collect();
            assert_eq!(unique.len(), names.len());
            for preset in LOCAL_MODEL_PRESETS {
                assert!(!names.contains(&preset.file_name));
            }
            assert!(names.contains(&"Qwen2.5-7B-Instruct-Q4_K_M.gguf"));
        }
        assert!(obsolete_model_file_names(None).contains(&"gemma-3-1b-it-Q4_K_M.gguf"));
    }

    #[test]
    fn 旧モデル名の掃除は選択中プリセット以外の実体を巻き込まない() {
        // 全プリセットの旧保存名を消していたため、切り替えて使う別モデルの実体まで起動時に失われていた。
        for active in LOCAL_MODEL_PRESETS {
            let names = obsolete_model_file_names(Some(active));
            for other in LOCAL_MODEL_PRESETS {
                if other.id == active.id {
                    continue;
                }
                let Some(legacy) = other.url.rsplit('/').next() else {
                    continue;
                };
                if RETIRED_MODEL_FILES.contains(&legacy) || legacy == other.file_name {
                    continue;
                }
                assert!(
                    !names.contains(&legacy),
                    "{} の掃除対象に別プリセット {} の旧保存名 {legacy} が入っている",
                    active.id,
                    other.id
                );
            }
        }
    }

    // OpenAI 互換 SSE を返すスタブ llama-server に対して、既定のローカルプロバイダが
    // チャンクを逐次受信できることを検証する（実 TCP 経由）。
    #[test]
    fn ローカルエンジンのsseストリーミングを逐次受信できる() {
        use super::Provider;
        use std::sync::{Arc, Mutex};
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf).await.unwrap(); // リクエストは読み捨て
                let body = concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"やあ\"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"、元気？\"}}]}\n\n",
                    "data: [DONE]\n\n"
                );
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(body.as_bytes()).await.unwrap();
            });

            let cfg = crate::config::Config::new();
            cfg.set("PICOAGENT_LOCAL_PORT", &addr.port().to_string());
            let chunks = Arc::new(Mutex::new(Vec::<String>::new()));
            let sink = {
                let chunks = chunks.clone();
                Arc::new(move |chunk: String| chunks.lock().unwrap().push(chunk)) as Arc<dyn Fn(String) + Send + Sync>
            };
            let message = super::Local
                .chat_stream(&cfg, "test", &[Message::new("user", "やあ")], &[], sink)
                .await
                .unwrap();
            assert_eq!(message.content, "やあ、元気？");
            assert_eq!(chunks.lock().unwrap().as_slice(), ["やあ", "、元気？"]);
            server.await.unwrap();
        });
    }

    #[test]
    fn 連続assistantをまとめ先頭assistantを落とす() {
        let input = vec![
            Message::new("system", "人格"),
            Message::new("assistant", "独り言A"), // 先頭の純assistant → 落ちる
            Message::new("user", "やあ"),
            Message::new("assistant", "へんじ1"),
            Message::new("assistant", "独り言B"), // 直前assistantへ連結
            Message::new("user", "つぎ"),
        ];
        let out = normalize_for_local(&input).unwrap();
        let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["system", "user", "assistant", "user"]);
        assert!(out[2].content.contains("へんじ1") && out[2].content.contains("独り言B"));
    }
}
