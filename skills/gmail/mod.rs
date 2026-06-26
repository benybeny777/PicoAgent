// skills/gmail/mod.rs - Gmail API でメール送信または下書き作成を行うスキル。
//
// OAuth アクセストークンは設定から都度読み、エラー文へ秘密値を出さない。

use std::time::Duration;

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_BODY_CHARS: usize = 20_000;
const MAX_SUBJECT_CHARS: usize = 300;

pub struct GmailTool;

fn gmail_base(cfg: &Config) -> String {
    cfg.get("PICOAGENT_GMAIL_BASE_URL", "https://gmail.googleapis.com/gmail/v1")
        .trim_end_matches('/')
        .to_string()
}

fn parse_timeout(cfg: &Config) -> Result<u64, String> {
    let timeout = cfg
        .get("PICOAGENT_GMAIL_TIMEOUT_SECS", "20")
        .parse::<u64>()
        .map_err(|_| "PICOAGENT_GMAIL_TIMEOUT_SECS は 3〜60 の整数で指定してください。".to_string())?;
    if (3..=60).contains(&timeout) {
        Ok(timeout)
    } else {
        Err("PICOAGENT_GMAIL_TIMEOUT_SECS は 3〜60 の範囲で指定してください。".to_string())
    }
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} が指定されていません。", key))
}

fn optional_list(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{} は文字列配列で指定してください。", key))?;
    let mut out = Vec::new();
    for item in array {
        let text = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{} は空でない文字列配列で指定してください。", key))?;
        out.push(text.to_string());
    }
    Ok(out)
}

fn header_value(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn message_id(response: &Value) -> &str {
    response
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| response.get("message").and_then(|m| m.get("id")).and_then(Value::as_str))
        .unwrap_or("unknown")
}

fn build_raw_message(args: &Value) -> Result<String, String> {
    let to = string_arg(args, "to")?;
    let subject = string_arg(args, "subject")?;
    if subject.chars().count() > MAX_SUBJECT_CHARS {
        return Err(format!("subject は {} 文字以内で指定してください。", MAX_SUBJECT_CHARS));
    }
    let body = string_arg(args, "body")?;
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!("body は {} 文字以内で指定してください。", MAX_BODY_CHARS));
    }
    let cc = optional_list(args, "cc")?;
    let bcc = optional_list(args, "bcc")?;
    let reply_to = args
        .get("reply_to")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut raw = String::new();
    raw.push_str(&format!("To: {}\r\n", header_value(to)));
    if !cc.is_empty() {
        raw.push_str(&format!("Cc: {}\r\n", header_value(&cc.join(", "))));
    }
    if !bcc.is_empty() {
        raw.push_str(&format!("Bcc: {}\r\n", header_value(&bcc.join(", "))));
    }
    if let Some(reply_to) = reply_to {
        raw.push_str(&format!("Reply-To: {}\r\n", header_value(reply_to)));
    }
    raw.push_str(&format!("Subject: {}\r\n", header_value(subject)));
    raw.push_str("MIME-Version: 1.0\r\n");
    raw.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    raw.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
    raw.push_str(body);
    Ok(URL_SAFE_NO_PAD.encode(raw.as_bytes()))
}

#[async_trait]
impl Tool for GmailTool {
    fn name(&self) -> &'static str {
        "gmail_message"
    }

    fn label(&self) -> &'static str {
        "Gmail送信/下書き"
    }

    fn description(&self) -> String {
        "Gmail API でメールを送信、または下書きを作成する。設定に OAuth access token が必要。送信はユーザーが宛先・件名・本文を明示した場合だけ行い、不確かな場合は draft を使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["draft", "send"],
                    "description": "draft は下書き作成、send は即送信。省略時は draft。"
                },
                "to": { "type": "string", "description": "宛先メールアドレス。複数の場合はカンマ区切り。" },
                "cc": { "type": "array", "items": { "type": "string" }, "description": "Cc。" },
                "bcc": { "type": "array", "items": { "type": "string" }, "description": "Bcc。" },
                "reply_to": { "type": "string", "description": "Reply-To。" },
                "subject": { "type": "string", "description": "件名。" },
                "body": { "type": "string", "description": "本文。プレーンテキスト。" }
            },
            "required": ["to", "subject", "body"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_GMAIL_ACCESS_TOKEN", "Gmail OAuth access token", "password", "ya29..."),
            Field::new("PICOAGENT_GMAIL_BASE_URL", "Gmail API base URL", "text", "https://gmail.googleapis.com/gmail/v1"),
            Field::new("PICOAGENT_GMAIL_TIMEOUT_SECS", "Gmail API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_GMAIL_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide("Gmail", &["PICOAGENT_GMAIL_ACCESS_TOKEN"], "Gmail API の送信/下書き作成スコープを持つ OAuth access token が必要です。");
        }
        let timeout_secs = match parse_timeout(cfg) {
            Ok(timeout) => timeout,
            Err(error) => return format!("エラー: {}", error),
        };
        let raw = match build_raw_message(&args) {
            Ok(raw) => raw,
            Err(error) => return format!("エラー: {}", error),
        };
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("draft")
            .trim();
        let (url, payload, action_label) = match mode {
            "send" => (
                format!("{}/users/me/messages/send", gmail_base(cfg)),
                json!({ "raw": raw }),
                "送信",
            ),
            "draft" | "" => (
                format!("{}/users/me/drafts", gmail_base(cfg)),
                json!({ "message": { "raw": raw } }),
                "下書き作成",
            ),
            _ => return "エラー: mode は draft または send を指定してください。".to_string(),
        };

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("PicoAgent/1.0 (Gmail tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Gmail API の準備に失敗しました: {}", error),
        };
        let response = match client.post(url).bearer_auth(token).json(&payload).send().await {
            Ok(response) => response,
            Err(error) => return format!("エラー: Gmail API への接続に失敗しました: {}", error),
        };
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return format!("エラー: Gmail API が HTTP {} を返しました。", status);
        }
        let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        format!("OK: Gmail の{}が完了しました。id={}", action_label, message_id(&parsed))
    }
}

inventory::submit! { ToolReg { make: || Box::new(GmailTool) } }

#[cfg(test)]
mod tests {
    use super::{build_raw_message, header_value};
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use serde_json::json;

    #[test]
    fn ヘッダー改行を除去する() {
        assert_eq!(header_value("a\r\nbcc: x"), "a  bcc: x");
    }

    #[test]
    fn gmail_raw_messageをbase64urlで作る() {
        let raw = build_raw_message(&json!({
            "to": "to@example.com",
            "subject": "件名",
            "body": "本文"
        }))
        .unwrap();
        let decoded = URL_SAFE_NO_PAD.decode(raw).unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert!(text.contains("To: to@example.com"));
        assert!(text.contains("Subject: 件名"));
        assert!(text.contains("本文"));
    }
}
