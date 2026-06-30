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

/// 1回の chat() 呼び出しで消費したトークン数。
/// プロバイダがレスポンスの `usage` フィールドから埋める。シリアライズしない（一時データ）。
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
    /// マルチモーダル: base64 エンコード済み画像（data URL 形式 "data:image/jpeg;base64,..."）。
    /// user メッセージのみ有効。シリアライズするが、ほとんどのプロバイダは無視する。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub images: Option<Vec<String>>,
    /// トークン使用量。履歴保存には含めない（実行時のみ）。
    #[serde(skip)]
    pub usage: Option<TokenUsage>,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Message {
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
        Message {
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

/// reasoning モデル（DeepSeek-R1 / Qwen3 など）が出力する `<think>...</think>` の
/// 推論ブロックを取り除く。閉じタグの無い未完了の `<think>` は、それ以降すべてを
/// 推論とみなして落とす。最終回答・履歴・読み上げに推論（多くは英語/中国語）が
/// 混ざらないようにするための正規化。
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
                    // 閉じタグが無い（推論の途中で打ち切られた）場合は残り全部を推論とみなす。
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
    use super::strip_reasoning;

    #[test]
    fn 推論ブロックを取り除く() {
        assert_eq!(
            strip_reasoning("<think>Let me think in English.</think>\n\nこんにちは！"),
            "こんにちは！"
        );
        // think を含まない通常応答はそのまま（trim のみ）。
        assert_eq!(strip_reasoning("やあ、元気？"), "やあ、元気？");
        // 閉じタグ無しは以降を落とす（回答が出る前に打ち切られたケース）。
        assert_eq!(strip_reasoning("答えるね<think>reasoning..."), "答えるね");
    }
}
