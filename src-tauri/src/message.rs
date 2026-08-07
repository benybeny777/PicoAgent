// message.rs - picoagent-core の正規形をアプリ内の既存パスで公開する互換層。

pub use picoagent_core::message::{strip_reasoning, Message, TokenUsage, ToolCall};
use serde::Serialize;

/// コネクタがPicoAgentの設定画面へ申告する設定項目。
#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    /// `text` / `password` / `textarea` / `checkbox` / `number`。
    pub r#type: String,
    pub placeholder: String,
}

impl Field {
    pub fn new(key: &str, label: &str, ty: &str, placeholder: &str) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            r#type: ty.to_string(),
            placeholder: placeholder.to_string(),
        }
    }
}
