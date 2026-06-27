// message.rs - コネクタ間の共通言語（正規形メッセージ）。
// 旧 JS 版の正規形と同義。各プロバイダが自分の API 形式へ相互変換する。

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 引数はオブジェクト（OpenAI の JSON 文字列はプロバイダ内で相互変換）。
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Message {
            role: role.to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }
}

/// 設定画面に並べる設定項目（コネクタが申告する）。
#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    /// "text" | "password" | "textarea" | "checkbox" | "number"
    pub r#type: String,
    pub placeholder: String,
}

impl Field {
    pub fn new(key: &str, label: &str, ty: &str, placeholder: &str) -> Self {
        Field {
            key: key.to_string(),
            label: label.to_string(),
            r#type: ty.to_string(),
            placeholder: placeholder.to_string(),
        }
    }
}
