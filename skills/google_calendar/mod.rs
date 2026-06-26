// skills/google_calendar/mod.rs - Google Calendar の予定確認・作成スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const CALENDAR_BASE: &str = "https://www.googleapis.com/calendar/v3";
const MAX_SUMMARY_CHARS: usize = 300;
const MAX_DESCRIPTION_CHARS: usize = 4000;

pub struct GoogleCalendarTool;

fn encode_path(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn timeout_secs(cfg: &Config) -> u64 {
    cfg.get("PICOAGENT_GOOGLE_CALENDAR_TIMEOUT_SECS", "20")
        .parse::<u64>()
        .ok()
        .filter(|value| (3..=60).contains(value))
        .unwrap_or(20)
}

#[async_trait]
impl Tool for GoogleCalendarTool {
    fn name(&self) -> &'static str {
        "google_calendar"
    }

    fn label(&self) -> &'static str {
        "Googleカレンダー"
    }

    fn description(&self) -> String {
        "Google Calendar API で予定を一覧取得または作成する。設定に OAuth access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "create"], "description": "list は予定一覧、create は予定作成。" },
                "calendar_id": { "type": "string", "description": "カレンダーID。省略時は設定値または primary。" },
                "time_min": { "type": "string", "description": "list の開始時刻 RFC3339。" },
                "time_max": { "type": "string", "description": "list の終了時刻 RFC3339。" },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 20, "description": "list の最大件数。" },
                "summary": { "type": "string", "description": "create の予定名。" },
                "description": { "type": "string", "description": "create の説明。" },
                "location": { "type": "string", "description": "create の場所。" },
                "start": { "type": "string", "description": "create の開始時刻 RFC3339。" },
                "end": { "type": "string", "description": "create の終了時刻 RFC3339。" },
                "timezone": { "type": "string", "description": "create のタイムゾーン。省略時 Asia/Tokyo。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_GOOGLE_CALENDAR_ACCESS_TOKEN", "Google Calendar OAuth access token", "password", "ya29..."),
            Field::new("PICOAGENT_GOOGLE_CALENDAR_ID", "Google Calendar 既定カレンダーID", "text", "primary"),
            Field::new("PICOAGENT_GOOGLE_CALENDAR_TIMEOUT_SECS", "Google Calendar API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_GOOGLE_CALENDAR_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide("Google Calendar", &["PICOAGENT_GOOGLE_CALENDAR_ACCESS_TOKEN"], "Google Calendar API を使える OAuth access token が必要です。");
        }
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let calendar_id = args
            .get("calendar_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_GOOGLE_CALENDAR_ID", "primary"));
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs(cfg)))
            .user_agent("PicoAgent/1.0 (Google Calendar tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Google Calendar API の準備に失敗しました: {}", error),
        };
        match action {
            "list" => {
                let mut request = client
                    .get(format!("{}/calendars/{}/events", CALENDAR_BASE, encode_path(&calendar_id)))
                    .bearer_auth(token)
                    .query(&[
                        ("singleEvents", "true".to_string()),
                        ("orderBy", "startTime".to_string()),
                        (
                            "maxResults",
                            args.get("max_results")
                                .and_then(Value::as_u64)
                                .filter(|value| (1..=20).contains(value))
                                .unwrap_or(10)
                                .to_string(),
                        ),
                    ]);
                if let Some(time_min) = args.get("time_min").and_then(Value::as_str) {
                    request = request.query(&[("timeMin", time_min)]);
                }
                if let Some(time_max) = args.get("time_max").and_then(Value::as_str) {
                    request = request.query(&[("timeMax", time_max)]);
                }
                let response = match request.send().await {
                    Ok(response) => response,
                    Err(error) => return format!("エラー: Google Calendar API への接続に失敗しました: {}", error),
                };
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    return format!("エラー: Google Calendar API が HTTP {} を返しました。", status);
                }
                let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
                let mut out = String::from("予定一覧:\n");
                for item in parsed.get("items").and_then(Value::as_array).into_iter().flatten() {
                    let summary = item.get("summary").and_then(Value::as_str).unwrap_or("（無題）");
                    let start = item
                        .get("start")
                        .and_then(|s| s.get("dateTime").or_else(|| s.get("date")))
                        .and_then(Value::as_str)
                        .unwrap_or("時刻不明");
                    out.push_str(&format!("- {}: {}\n", start, summary));
                }
                out
            }
            "create" => {
                let summary = match args.get("summary").and_then(Value::as_str).map(str::trim) {
                    Some(summary) if !summary.is_empty() => summary,
                    _ => return "エラー: summary が指定されていません。".to_string(),
                };
                if summary.chars().count() > MAX_SUMMARY_CHARS {
                    return format!("エラー: summary は {} 文字以内で指定してください。", MAX_SUMMARY_CHARS);
                }
                let start = match args.get("start").and_then(Value::as_str).map(str::trim) {
                    Some(start) if !start.is_empty() => start,
                    _ => return "エラー: start が指定されていません。".to_string(),
                };
                let end = match args.get("end").and_then(Value::as_str).map(str::trim) {
                    Some(end) if !end.is_empty() => end,
                    _ => return "エラー: end が指定されていません。".to_string(),
                };
                let timezone = args.get("timezone").and_then(Value::as_str).unwrap_or("Asia/Tokyo");
                let mut payload = json!({
                    "summary": summary,
                    "start": { "dateTime": start, "timeZone": timezone },
                    "end": { "dateTime": end, "timeZone": timezone }
                });
                if let Some(description) = args.get("description").and_then(Value::as_str) {
                    if description.chars().count() > MAX_DESCRIPTION_CHARS {
                        return format!("エラー: description は {} 文字以内で指定してください。", MAX_DESCRIPTION_CHARS);
                    }
                    payload["description"] = json!(description);
                }
                if let Some(location) = args.get("location").and_then(Value::as_str) {
                    payload["location"] = json!(location);
                }
                let response = match client
                    .post(format!("{}/calendars/{}/events", CALENDAR_BASE, encode_path(&calendar_id)))
                    .bearer_auth(token)
                    .json(&payload)
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => return format!("エラー: Google Calendar API への接続に失敗しました: {}", error),
                };
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    return format!("エラー: Google Calendar API が HTTP {} を返しました。", status);
                }
                let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
                format!(
                    "OK: 予定を作成しました。id={}",
                    parsed.get("id").and_then(Value::as_str).unwrap_or("unknown")
                )
            }
            _ => "エラー: action は list または create を指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(GoogleCalendarTool) } }
