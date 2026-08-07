// message.rs - プロバイダとエージェントループが共有する正規形メッセージ。

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 引数はオブジェクト。プロバイダ固有の JSON 文字列との変換はアダプタ側で行う。
    pub arguments: Value,
}

/// 1回のモデル呼び出しで消費したトークン数。
/// 履歴へ保存しない実行時情報として扱う。
#[derive(Clone, Debug, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
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
    /// base64 エンコード済み画像の data URL。画像対応プロバイダだけが解釈する。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub images: Option<Vec<String>>,
    /// 履歴へ含めない一時的な使用量情報。
    #[serde(skip)]
    pub usage: Option<TokenUsage>,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: None,
            usage: None,
        }
    }

    /// 画像付きユーザーメッセージを作る。
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: if images.is_empty() {
                None
            } else {
                Some(images)
            },
            usage: None,
        }
    }
}

/// reasoning モデルが返す `<think>...</think>` を最終回答から除く。
/// 閉じタグが無い場合は、開きタグ以降を未完了の推論として落とす。
pub fn strip_reasoning(content: &str) -> String {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    loop {
        match rest.find(OPEN) {
            Some(start) => {
                out.push_str(&rest[..start]);
                let after = &rest[start + OPEN.len()..];
                match after.find(CLOSE) {
                    Some(end) => rest = &after[end + CLOSE.len()..],
                    None => break,
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{strip_reasoning, Message, TokenUsage};

    #[test]
    fn 推論ブロックを取り除く() {
        assert_eq!(
            strip_reasoning("<think>Let me think.</think>\n\nこんにちは！"),
            "こんにちは！"
        );
        assert_eq!(strip_reasoning("やあ、元気？"), "やあ、元気？");
        assert_eq!(strip_reasoning("答えるね<think>reasoning..."), "答えるね");
    }

    #[test]
    fn 使用量は履歴用jsonへ含めない() {
        let mut message = Message::new("assistant", "完了");
        message.usage = Some(TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        });
        let value = serde_json::to_value(message).expect("メッセージをJSON化できる");
        assert!(value.get("usage").is_none());
    }
}
