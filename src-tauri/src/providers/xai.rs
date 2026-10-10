// xAI Grok 固有の設定と差分だけを宣言し、通信は共通層へ委譲する。
use super::compatible::{CompatProvider, CompatSpec};
use super::ProviderReg;

pub(crate) static SPEC: CompatSpec = CompatSpec {
    id: "xai",
    label: "xAI Grok",
    api_key_key: "PICOAGENT_XAI_API_KEY",
    base_url_key: "PICOAGENT_XAI_BASE_URL",
    model_key: "PICOAGENT_XAI_MODEL",
    default_base_url: "https://api.x.ai/v1",
    default_model: "grok-4.5",
    key_placeholder: "xai-...",
    key_label: "xAI API キー",
    base_label: "エンドポイント",
    static_models: &["grok-4.5"],
    extra_body: None,
    extra_fields: None,
    model_prefix: None,
};

inventory::submit! { ProviderReg { make: || Box::new(CompatProvider(&SPEC)) } }
