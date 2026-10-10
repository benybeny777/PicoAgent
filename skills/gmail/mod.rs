// skills/gmail/mod.rs - Gmail API でメール送信・下書き・整理を行うスキル。
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
const MAX_QUERY_CHARS: usize = 500;
const MAX_READ_BODY_CHARS: usize = 8_000;
const GMAIL_PAGE_SIZE: u64 = 500;

pub struct GmailTool;

fn gmail_base(cfg: &Config) -> String {
    cfg.get(
        "PICOAGENT_GMAIL_BASE_URL",
        "https://gmail.googleapis.com/gmail/v1",
    )
    .trim_end_matches('/')
    .to_string()
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

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn bounded_max_results(args: &Value, default: u64, max: u64) -> u64 {
    args.get("max_results")
        .and_then(Value::as_u64)
        .filter(|value| (1..=max).contains(value))
        .unwrap_or(default)
}

fn requested_cleanup_limit(args: &Value, configured_limit: Option<u64>) -> Option<u64> {
    let requested = args
        .get("max_results")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0);
    match (requested, configured_limit) {
        (Some(requested), Some(configured)) => Some(requested.min(configured)),
        (Some(requested), None) => Some(requested),
        (None, Some(configured)) => Some(configured),
        (None, None) => None,
    }
}

fn query_arg(args: &Value) -> Result<&str, String> {
    let q = args.get("q").and_then(Value::as_str).unwrap_or("").trim();
    if q.is_empty() {
        return Err("q が指定されていません。".to_string());
    }
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(format!(
            "q は {} 文字以内で指定してください。",
            MAX_QUERY_CHARS
        ));
    }
    Ok(q)
}

fn ids_for_batch(args: &Value) -> Result<Vec<String>, String> {
    let mut ids = optional_list(args, "ids")?;
    if let Ok(id) = string_arg(args, "id") {
        ids.push(id.to_string());
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err("id または ids が指定されていません。".to_string());
    }
    Ok(ids)
}

fn modify_payload(operation: &str, label_ids: Vec<String>) -> Result<Value, String> {
    let payload = match operation {
        "archive" => json!({ "removeLabelIds": ["INBOX"] }),
        "mark_read" => json!({ "removeLabelIds": ["UNREAD"] }),
        "mark_unread" => json!({ "addLabelIds": ["UNREAD"] }),
        "add_label" => {
            if label_ids.is_empty() {
                return Err("add_label には label_ids が必要です。".to_string());
            }
            json!({ "addLabelIds": label_ids })
        }
        "remove_label" => {
            if label_ids.is_empty() {
                return Err("remove_label には label_ids が必要です。".to_string());
            }
            json!({ "removeLabelIds": label_ids })
        }
        _ => return Err("operation は archive / trash / mark_read / mark_unread / add_label / remove_label を指定してください。".to_string()),
    };
    Ok(payload)
}

async fn list_message_ids(
    client: &reqwest::Client,
    cfg: &Config,
    token: &str,
    q: &str,
    limit: Option<u64>,
) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let remaining = limit.map(|max| max.saturating_sub(ids.len() as u64));
        if remaining == Some(0) {
            break;
        }
        let page_size = remaining.unwrap_or(GMAIL_PAGE_SIZE).min(GMAIL_PAGE_SIZE);
        let max_results_text = page_size.to_string();
        let mut query = vec![("q", q), ("maxResults", max_results_text.as_str())];
        if let Some(token) = page_token.as_deref() {
            query.push(("pageToken", token));
        }
        let response = client
            .get(format!("{}/users/me/messages", gmail_base(cfg)))
            .bearer_auth(token)
            .query(&query)
            .send()
            .await
            .map_err(|error| {
                format!("Gmail API への接続に失敗しました: {}", error.without_url())
            })?;
        let body = crate::tools::http::service_body(cfg, "Gmail API", Ok(response)).await?;
        let parsed: Value = crate::tools::http::response_json(&body)?;
        let page_ids: Vec<String> = parsed
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();
        if page_ids.is_empty() {
            break;
        }
        ids.extend(page_ids);
        page_token = parsed
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if page_token.is_none() {
            break;
        }
    }
    Ok(ids)
}

async fn post_gmail_json(
    cfg: &Config,
    client: &reqwest::Client,
    token: &str,
    url: String,
    payload: &Value,
) -> Result<Value, String> {
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(payload)
        .send()
        .await
        .map_err(|error| format!("Gmail API への接続に失敗しました: {}", error.without_url()))?;
    let body = crate::tools::http::service_body(cfg, "Gmail API", Ok(response)).await?;
    crate::tools::http::response_json(&body)
}

async fn trash_messages(
    client: &reqwest::Client,
    cfg: &Config,
    token: &str,
    ids: &[String],
) -> Result<(), String> {
    for id in ids {
        let response = client
            .post(format!(
                "{}/users/me/messages/{}/trash",
                gmail_base(cfg),
                id
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| {
                format!("Gmail API への接続に失敗しました: {}", error.without_url())
            })?;
        crate::tools::http::service_body(cfg, "Gmail API", Ok(response)).await?;
    }
    Ok(())
}

fn header_value(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn message_id(response: &Value) -> &str {
    response
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| {
            response
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
        })
        .unwrap_or("unknown")
}

fn header<'a>(message: &'a Value, name: &str) -> &'a str {
    message
        .get("payload")
        .and_then(|payload| payload.get("headers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|header| {
            header
                .get("name")
                .and_then(Value::as_str)
                .map(|value| value.eq_ignore_ascii_case(name))
                .unwrap_or(false)
        })
        .and_then(|header| header.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn decode_gmail_body(data: &str) -> String {
    URL_SAFE_NO_PAD
        .decode(data)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

fn extract_text_part(payload: &Value) -> String {
    if payload
        .get("mimeType")
        .and_then(Value::as_str)
        .map(|mime| mime.starts_with("text/plain"))
        .unwrap_or(false)
    {
        if let Some(data) = payload
            .get("body")
            .and_then(|body| body.get("data"))
            .and_then(Value::as_str)
        {
            return decode_gmail_body(data);
        }
    }
    for part in payload
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = extract_text_part(part);
        if !text.trim().is_empty() {
            return text;
        }
    }
    String::new()
}

fn build_raw_message(args: &Value) -> Result<String, String> {
    let to = string_arg(args, "to")?;
    let subject = string_arg(args, "subject")?;
    if subject.chars().count() > MAX_SUBJECT_CHARS {
        return Err(format!(
            "subject は {} 文字以内で指定してください。",
            MAX_SUBJECT_CHARS
        ));
    }
    let body = string_arg(args, "body")?;
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!(
            "body は {} 文字以内で指定してください。",
            MAX_BODY_CHARS
        ));
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

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "gmail",
            "メール",
            "下書き",
            "送信",
            "未読",
            "受信",
            "メール一覧",
            "振り分け",
            "ラベル",
            "ゴミメール",
            "掃除",
            "アーカイブ",
            "既読",
            "ゴミ箱",
        ]
    }

    fn label(&self) -> &'static str {
        "Gmail操作"
    }

    fn description(&self) -> String {
        "Gmail API でメール一覧・本文取得・ラベル整理・アーカイブ・既読化・ゴミ箱移動・送信・下書き作成を行う。設定に OAuth access token が必要。送信はユーザーが宛先・件名・本文を明示した場合だけ行い、不確かな場合は draft を使う。検索条件ベースの整理とゴミ箱移動は preview 後に confirm=true で実行する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "get", "labels", "modify", "cleanup", "draft", "send"],
                    "description": "list はメール一覧、get は本文取得、labels はラベル一覧、modify は id/ids の整理、cleanup は q に一致するメールの整理、draft は下書き作成、send は即送信。省略時は mode または draft。"
                },
                "mode": {
                    "type": "string",
                    "enum": ["draft", "send"],
                    "description": "旧互換。draft は下書き作成、send は即送信。"
                },
                "q": { "type": "string", "description": "list / cleanup の Gmail 検索クエリ。例: is:unread newer_than:7d" },
                "max_results": { "type": "integer", "minimum": 1, "description": "list / cleanup の最大件数。cleanup は設定上限を超えない範囲で指定できる。" },
                "id": { "type": "string", "description": "get / modify の Gmail message id。" },
                "ids": { "type": "array", "items": { "type": "string" }, "description": "modify の Gmail message id 配列。" },
                "operation": {
                    "type": "string",
                    "enum": ["archive", "trash", "mark_read", "mark_unread", "add_label", "remove_label"],
                    "description": "modify / cleanup の整理操作。trash はGmailのゴミ箱へ移動し、完全削除はしない。"
                },
                "label_ids": { "type": "array", "items": { "type": "string" }, "description": "add_label / remove_label で使う Gmail label id。" },
                "confirm": { "type": "boolean", "description": "cleanup と trash 実行の確認。false または省略時は preview のみ。" },
                "to": { "type": "string", "description": "宛先メールアドレス。複数の場合はカンマ区切り。" },
                "cc": { "type": "array", "items": { "type": "string" }, "description": "Cc。" },
                "bcc": { "type": "array", "items": { "type": "string" }, "description": "Bcc。" },
                "reply_to": { "type": "string", "description": "Reply-To。" },
                "subject": { "type": "string", "description": "件名。" },
                "body": { "type": "string", "description": "本文。プレーンテキスト。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_GMAIL_ACCESS_TOKEN",
                "Gmail の OAuth アクセストークン",
                "password",
                "ya29...",
            ),
            Field::new(
                "PICOAGENT_GMAIL_BASE_URL",
                "Gmail API のベース URL",
                "text",
                "https://gmail.googleapis.com/gmail/v1",
            ),
            Field::int(&crate::config::ints::GMAIL_TIMEOUT_SECS),
            Field::int(&crate::config::ints::GMAIL_CLEANUP_MAX_RESULTS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_GMAIL_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide(
                "Gmail",
                &["PICOAGENT_GMAIL_ACCESS_TOKEN"],
                "Gmail API の読み取り・変更・送信に必要な OAuth access token が必要です。",
            );
        }
        let timeout_secs =
            crate::tool_try!(cfg.int(&crate::config::ints::GMAIL_TIMEOUT_SECS)) as u64;
        let cleanup_count =
            crate::tool_try!(cfg.int(&crate::config::ints::GMAIL_CLEANUP_MAX_RESULTS)) as u64;
        let cleanup_limit = (cleanup_count != 0).then_some(cleanup_count);
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .or_else(|| args.get("mode").and_then(Value::as_str))
            .unwrap_or("draft")
            .trim();
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("PicoAgent/1.0 (Gmail tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Gmail API の準備に失敗しました: {}", error),
        };
        if action == "labels" {
            let response = match client
                .get(format!("{}/users/me/labels", gmail_base(cfg)))
                .bearer_auth(&token)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    return format!(
                        "エラー: Gmail API への接続に失敗しました: {}",
                        error.without_url()
                    )
                }
            };
            let body = crate::tool_try!(
                crate::tools::http::service_body(cfg, "Gmail API", Ok(response)).await
            );
            let parsed: Value = crate::tool_response_json!(&body);
            let labels = parsed
                .get("labels")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if labels.is_empty() {
                return "Gmail ラベル: ラベルはありません。".to_string();
            }
            let mut out = "Gmail ラベル:".to_string();
            for label in labels {
                out.push_str(&format!(
                    "\n- id={} / name={} / type={}",
                    label.get("id").and_then(Value::as_str).unwrap_or(""),
                    label.get("name").and_then(Value::as_str).unwrap_or(""),
                    label.get("type").and_then(Value::as_str).unwrap_or("")
                ));
            }
            return out;
        }
        if action == "modify" {
            let operation = match args.get("operation").and_then(Value::as_str).map(str::trim) {
                Some(operation) if !operation.is_empty() => operation,
                _ => return "エラー: operation が指定されていません。".to_string(),
            };
            let ids = match ids_for_batch(&args) {
                Ok(ids) => ids,
                Err(error) => return format!("エラー: {}", error),
            };
            let label_ids = match optional_list(&args, "label_ids") {
                Ok(label_ids) => label_ids,
                Err(error) => return format!("エラー: {}", error),
            };
            if operation == "trash" {
                if !bool_arg(&args, "confirm") {
                    return format!(
                        "Gmail ゴミ箱移動 preview: {} 件をゴミ箱へ移動できます。実行するには confirm=true を指定してください。ids={}",
                        ids.len(),
                        ids.join(",")
                    );
                }
                if let Err(error) = trash_messages(&client, cfg, &token, &ids).await {
                    return format!("エラー: {}", error);
                }
                return format!("OK: Gmail の {} 件をゴミ箱へ移動しました。", ids.len());
            }
            let payload = match modify_payload(operation, label_ids) {
                Ok(payload) => payload,
                Err(error) => return format!("エラー: {}", error),
            };
            if ids.len() == 1 {
                let url = format!("{}/users/me/messages/{}/modify", gmail_base(cfg), ids[0]);
                if let Err(error) = post_gmail_json(cfg, &client, &token, url, &payload).await {
                    return format!("エラー: {}", error);
                }
                return format!("OK: Gmail の 1 件を {} で整理しました。", operation);
            } else {
                let url = format!("{}/users/me/messages/batchModify", gmail_base(cfg));
                let count = ids.len();
                let payload = json!({
                    "ids": ids,
                    "addLabelIds": payload.get("addLabelIds").cloned().unwrap_or_else(|| json!([])),
                    "removeLabelIds": payload.get("removeLabelIds").cloned().unwrap_or_else(|| json!([]))
                });
                if let Err(error) = post_gmail_json(cfg, &client, &token, url, &payload).await {
                    return format!("エラー: {}", error);
                }
                return format!("OK: Gmail の {} 件を {} で整理しました。", count, operation);
            }
        }
        if action == "cleanup" {
            let q = match query_arg(&args) {
                Ok(q) => q,
                Err(error) => return format!("エラー: {}", error),
            };
            let operation = match args.get("operation").and_then(Value::as_str).map(str::trim) {
                Some(operation) if !operation.is_empty() => operation,
                _ => return "エラー: operation が指定されていません。".to_string(),
            };
            let limit = requested_cleanup_limit(&args, cleanup_limit);
            let ids = match list_message_ids(&client, cfg, &token, q, limit).await {
                Ok(ids) => ids,
                Err(error) => return format!("エラー: {}", error),
            };
            if ids.is_empty() {
                return format!("Gmail cleanup: q={} に一致するメールはありません。", q);
            }
            let limit_label = limit
                .map(|value| value.to_string())
                .unwrap_or_else(|| "無制限".to_string());
            if !bool_arg(&args, "confirm") {
                let preview_ids = ids.iter().take(20).cloned().collect::<Vec<_>>().join(",");
                return format!(
                    "Gmail cleanup preview: q={} / operation={} / 対象 {} 件 / 上限 {}。実行するには confirm=true を指定してください。先頭id={}",
                    q,
                    operation,
                    ids.len(),
                    limit_label,
                    preview_ids
                );
            }
            if operation == "trash" {
                if let Err(error) = trash_messages(&client, cfg, &token, &ids).await {
                    return format!("エラー: {}", error);
                }
                return format!(
                    "OK: Gmail cleanup で {} 件をゴミ箱へ移動しました。q={}",
                    ids.len(),
                    q
                );
            }
            let label_ids = match optional_list(&args, "label_ids") {
                Ok(label_ids) => label_ids,
                Err(error) => return format!("エラー: {}", error),
            };
            let label_payload = match modify_payload(operation, label_ids) {
                Ok(payload) => payload,
                Err(error) => return format!("エラー: {}", error),
            };
            let url = format!("{}/users/me/messages/batchModify", gmail_base(cfg));
            let payload = json!({
                "ids": ids,
                "addLabelIds": label_payload.get("addLabelIds").cloned().unwrap_or_else(|| json!([])),
                "removeLabelIds": label_payload.get("removeLabelIds").cloned().unwrap_or_else(|| json!([]))
            });
            if let Err(error) = post_gmail_json(cfg, &client, &token, url, &payload).await {
                return format!("エラー: {}", error);
            }
            let count = payload
                .get("ids")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            return format!(
                "OK: Gmail cleanup で {} 件を {} で整理しました。q={}",
                count, operation, q
            );
        }
        if action == "list" {
            let q = args
                .get("q")
                .and_then(Value::as_str)
                .unwrap_or("is:unread newer_than:7d")
                .trim();
            if q.chars().count() > MAX_QUERY_CHARS {
                return format!(
                    "エラー: q は {} 文字以内で指定してください。",
                    MAX_QUERY_CHARS
                );
            }
            let max_results = bounded_max_results(&args, 5, 10);
            let max_results_text = max_results.to_string();
            let response = match client
                .get(format!("{}/users/me/messages", gmail_base(cfg)))
                .bearer_auth(&token)
                .query(&[("q", q), ("maxResults", max_results_text.as_str())])
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    return format!(
                        "エラー: Gmail API への接続に失敗しました: {}",
                        error.without_url()
                    )
                }
            };
            let body = crate::tool_try!(
                crate::tools::http::service_body(cfg, "Gmail API", Ok(response)).await
            );
            let parsed: Value = crate::tool_response_json!(&body);
            let messages: Vec<Value> = parsed
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if messages.is_empty() {
                return "Gmail 一覧: 条件に一致するメールはありません。".to_string();
            }
            let mut out = format!("Gmail 一覧（q={}）:", q);
            for item in messages {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let detail = match client
                    .get(format!("{}/users/me/messages/{}", gmail_base(cfg), id))
                    .bearer_auth(&token)
                    .query(&[
                        ("format", "metadata"),
                        ("metadataHeaders", "From"),
                        ("metadataHeaders", "Subject"),
                        ("metadataHeaders", "Date"),
                    ])
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return format!(
                            "エラー: Gmail API への接続に失敗しました: {}",
                            error.without_url()
                        )
                    }
                };
                let status = detail.status();
                let body = crate::tool_response_text!(detail);
                if !status.is_success() {
                    return format!(
                        "エラー: {}",
                        crate::tools::http::safe_service_error(cfg, "Gmail API", status, &body)
                    );
                }
                let message: Value = crate::tool_response_json!(&body);
                out.push_str(&format!(
                    "\n- id={} / {} / {} / {} / {}",
                    id,
                    header(&message, "Date"),
                    header(&message, "From"),
                    header(&message, "Subject"),
                    message.get("snippet").and_then(Value::as_str).unwrap_or("")
                ));
            }
            return out;
        }
        if action == "get" {
            let id = match args.get("id").and_then(Value::as_str).map(str::trim) {
                Some(id) if !id.is_empty() => id,
                _ => return "エラー: id が指定されていません。".to_string(),
            };
            let response = match client
                .get(format!("{}/users/me/messages/{}", gmail_base(cfg), id))
                .bearer_auth(&token)
                .query(&[("format", "full")])
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    return format!(
                        "エラー: Gmail API への接続に失敗しました: {}",
                        error.without_url()
                    )
                }
            };
            let body = crate::tool_try!(
                crate::tools::http::service_body(cfg, "Gmail API", Ok(response)).await
            );
            let message: Value = crate::tool_response_json!(&body);
            let mut text = message
                .get("payload")
                .map(extract_text_part)
                .unwrap_or_default();
            if text.trim().is_empty() {
                text = message
                    .get("snippet")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
            }
            if text.chars().count() > MAX_READ_BODY_CHARS {
                text = text.chars().take(MAX_READ_BODY_CHARS).collect::<String>();
                text.push_str("\n…（本文を打ち切り）");
            }
            return format!(
                "Gmail 本文:\nFrom: {}\nDate: {}\nSubject: {}\n\n{}",
                header(&message, "From"),
                header(&message, "Date"),
                header(&message, "Subject"),
                text
            );
        }
        let raw = match build_raw_message(&args) {
            Ok(raw) => raw,
            Err(error) => return format!("エラー: {}", error),
        };
        let (url, payload, action_label) = match action {
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
            _ => return "エラー: action は list / get / labels / modify / cleanup / draft / send を指定してください。".to_string(),
        };
        let response = match client
            .post(url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return format!(
                    "エラー: Gmail API への接続に失敗しました: {}",
                    error.without_url()
                )
            }
        };
        let body = crate::tool_try!(
            crate::tools::http::service_body(cfg, "Gmail API", Ok(response)).await
        );
        let parsed: Value = crate::tool_response_json!(&body);
        format!(
            "OK: Gmail の{}が完了しました。id={}",
            action_label,
            message_id(&parsed)
        )
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
