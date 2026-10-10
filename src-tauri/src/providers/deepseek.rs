// DeepSeek 固有の設定と差分だけを宣言し、通信は共通層へ委譲する。
use super::compatible::{CompatProvider, CompatSpec};
use super::ProviderReg;
use crate::config::Config;
use crate::message::Field;
use serde_json::{json, Value};

fn extra_fields() -> Vec<Field> {
    vec![Field::new(
        "PICOAGENT_DEEPSEEK_THINKING",
        "Thinking モード",
        "checkbox",
        "true",
    )]
}
fn extra_body(cfg: &Config, model: &str, body: &mut Value) {
    if matches!(model, "deepseek-v4-flash" | "deepseek-v4-pro") {
        body["thinking"] = json!({"type": if cfg.get_bool("PICOAGENT_DEEPSEEK_THINKING", true) {"enabled"} else {"disabled"}});
    }
}

pub(crate) static SPEC: CompatSpec = CompatSpec {
    id: "deepseek",
    label: "DeepSeek",
    api_key_key: "PICOAGENT_DEEPSEEK_API_KEY",
    base_url_key: "PICOAGENT_DEEPSEEK_BASE_URL",
    model_key: "PICOAGENT_DEEPSEEK_MODEL",
    default_base_url: "https://api.deepseek.com",
    default_model: "deepseek-v4-flash",
    key_placeholder: "sk-...",
    key_label: "DeepSeek API キー",
    base_label: "エンドポイント",
    static_models: &[
        "deepseek-v4-flash",
        "deepseek-v4-pro",
        "deepseek-chat",
        "deepseek-reasoner",
    ],
    extra_body: Some(extra_body),
    extra_fields: Some(extra_fields),
    model_prefix: None,
};

inventory::submit! { ProviderReg { make: || Box::new(CompatProvider(&SPEC)) } }
