// Gemini 固有の設定と差分だけを宣言し、通信は共通層へ委譲する。
use super::compatible::{CompatProvider, CompatSpec};
use super::ProviderReg;

pub(crate) static SPEC: CompatSpec = CompatSpec {
    id: "gemini",
    label: "Gemini",
    api_key_key: "PICOAGENT_GEMINI_API_KEY",
    base_url_key: "PICOAGENT_GEMINI_BASE_URL",
    model_key: "PICOAGENT_GEMINI_MODEL",
    default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
    default_model: "gemini-3.6-flash",
    key_placeholder: "AIza...",
    key_label: "Gemini API キー",
    base_label: "エンドポイント（OpenAI 互換）",
    static_models: &["gemini-3.6-flash"],
    extra_body: None,
    extra_fields: None,
    model_prefix: Some("models/"),
};

inventory::submit! { ProviderReg { make: || Box::new(CompatProvider(&SPEC)) } }
