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

/// 本文へ漏れたツール呼び出しの検出結果。
#[derive(Debug, PartialEq, Eq)]
pub struct LeakedToolCall {
    /// 漏れた呼び出しのツール名。
    pub name: String,
    /// 呼び出し表記を取り除いた本文。
    pub cleaned: String,
}

/// 本文へ漏れたツール呼び出しを見つけ、表示用に取り除いた本文と一緒に返す。
///
/// 小型のローカルモデルは、テンプレートのツール記法を崩して `web_search{query:...}` の
/// ような文字列を本文へ書くことがある。`tool_calls` が空のままこれを表示すると、利用者
/// には意味不明な文字列だけが届き、ツールも実行されない。引数の解釈までは行わない。壊れ
/// た表記から引数を推測して実行すると、誤った対象へ副作用を与えるおそれがあるため、検出
/// と除去に留めて、呼び出し自体はモデルへ出し直させる。
pub fn find_leaked_tool_call(content: &str, known_names: &[&str]) -> Option<LeakedToolCall> {
    for name in known_names {
        let mut from = 0;
        while let Some(offset) = content[from..].find(*name) {
            let start = from + offset;
            let end = start + name.len();
            from = end;
            // 前後が識別子の一部なら別の語。長い名前の一部を拾わない。
            let boundary_before = content[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            if !boundary_before {
                continue;
            }
            let rest = content[end..].trim_start();
            let close = match rest.chars().next() {
                Some('{') => '}',
                Some('(') => ')',
                _ => continue,
            };
            let open_at = content.len() - rest.len();
            let Some(close_at) = balanced_end(&content[open_at..], close) else {
                continue;
            };
            let mut cleaned = String::with_capacity(content.len());
            cleaned.push_str(&content[..start]);
            cleaned.push_str(&content[open_at + close_at + close.len_utf8()..]);
            return Some(LeakedToolCall {
                name: (*name).to_string(),
                cleaned: cleaned.trim().to_string(),
            });
        }
    }
    None
}

/// 開き括弧から対応する閉じ括弧までの相対位置を返す。
fn balanced_end(text: &str, close: char) -> Option<usize> {
    let open = if close == '}' { '{' } else { '(' };
    let mut depth = 0usize;
    for (index, character) in text.char_indices() {
        if character == open {
            depth += 1;
        } else if character == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{find_leaked_tool_call, strip_reasoning, Message, TokenUsage};

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
    fn 本文へ漏れたツール呼び出しを取り除く() {
        // 実機のローカルモデルが本文へ書いた形。引用符がテンプレートのトークンごと崩れている。
        let leaked = find_leaked_tool_call(
            "web_search{query:<|\">神奈川県 の今日の天気<|\">}",
            &["web_search", "read_file"],
        )
        .expect("漏れた呼び出しを見つける");
        assert_eq!(leaked.name, "web_search");
        assert_eq!(leaked.cleaned, "");
    }

    #[test]
    fn 漏れた呼び出しの前後の文は残す() {
        let leaked = find_leaked_tool_call(
            "調べるね web_search(query: 天気) 少し待ってね",
            &["web_search"],
        )
        .expect("漏れた呼び出しを見つける");
        assert_eq!(leaked.cleaned, "調べるね  少し待ってね");
    }

    #[test]
    fn 普通の本文はツール呼び出しとみなさない() {
        assert!(find_leaked_tool_call("web_search で調べておくね", &["web_search"]).is_none());
        assert!(find_leaked_tool_call("今日はいい天気だよ", &["web_search"]).is_none());
        // 長い名前の一部に一致しただけのものは拾わない。
        assert!(find_leaked_tool_call("my_web_search{a:1}", &["web_search"]).is_none());
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
