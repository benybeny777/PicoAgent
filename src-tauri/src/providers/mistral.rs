// Mistral 固有の設定と差分だけを宣言し、通信は共通層へ委譲する。
use super::compatible::{CompatProvider, CompatSpec};
use super::ProviderReg;

pub(crate) static SPEC: CompatSpec = CompatSpec {
    id: "mistral",
    label: "Mistral",
    api_key_key: "PICOAGENT_MISTRAL_API_KEY",
    base_url_key: "PICOAGENT_MISTRAL_BASE_URL",
    model_key: "PICOAGENT_MISTRAL_MODEL",
    default_base_url: "https://api.mistral.ai/v1",
    default_model: "mistral-medium-latest",
    key_placeholder: "",
    key_label: "Mistral API キー",
    base_label: "エンドポイント",
    static_models: &[
        "mistral-medium-latest",
        "mistral-small-latest",
        "mistral-large-latest",
        "ministral-14b-latest",
        "ministral-8b-latest",
        "ministral-3b-latest",
        "codestral-latest",
    ],
    extra_body: None,
    extra_fields: None,
    model_prefix: None,
};

inventory::submit! { ProviderReg { make: || Box::new(CompatProvider(&SPEC)) } }
