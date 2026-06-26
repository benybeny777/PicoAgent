// skills/google_sheets/mod.rs - Google Sheets の範囲読み取り・更新・追記スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const SHEETS_BASE: &str = "https://sheets.googleapis.com/v4/spreadsheets";

pub struct GoogleSheetsTool;

#[async_trait]
impl Tool for GoogleSheetsTool {
    fn name(&self) -> &'static str {
        "google_sheets"
    }

    fn label(&self) -> &'static str {
        "Google Sheets"
    }

    fn description(&self) -> String {
        "Google Sheets API でセル範囲を読み取り、更新、追記する。設定に OAuth access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["read", "update", "append"], "description": "read/update/append。" },
                "spreadsheet_id": { "type": "string", "description": "スプレッドシートID。省略時は設定値。" },
                "range": { "type": "string", "description": "A1表記の範囲。" },
                "values": { "type": "array", "items": { "type": "array" }, "description": "update/append の2次元配列。" }
            },
            "required": ["action", "range"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN", "Google Sheets OAuth access token", "password", "ya29..."),
            Field::new("PICOAGENT_GOOGLE_SHEETS_ID", "Google Sheets 既定 spreadsheet ID", "text", ""),
            Field::new("PICOAGENT_GOOGLE_SHEETS_TIMEOUT_SECS", "Google Sheets API 待ち時間（秒、3〜60）", "text", "20"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide("Google Sheets", &["PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN"], "Google Sheets API を使える OAuth access token が必要です。");
        }
        let spreadsheet_id = args
            .get("spreadsheet_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_GOOGLE_SHEETS_ID", ""));
        if spreadsheet_id.is_empty() {
            return super::config_guide("Google Sheets", &["PICOAGENT_GOOGLE_SHEETS_ID"], "依頼で spreadsheet_id を渡すか、既定の spreadsheet ID を設定してください。");
        }
        let Some(range) = args.get("range").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
            return "エラー: range が指定されていません。".to_string();
        };
        let timeout = cfg
            .get("PICOAGENT_GOOGLE_SHEETS_TIMEOUT_SECS", "20")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(20);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Google Sheets tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Google Sheets API の準備に失敗しました: {}", error),
        };
        let encoded_range: String = url::form_urlencoded::byte_serialize(range.as_bytes()).collect();
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "read" => {
                let response = client
                    .get(format!("{}/{}/values/{}", SHEETS_BASE, spreadsheet_id, encoded_range))
                    .bearer_auth(token)
                    .send()
                    .await;
                handle_text_response(response, "Google Sheets API").await
            }
            "update" | "append" => {
                let values = match args.get("values").and_then(Value::as_array) {
                    Some(values) => values,
                    None => return "エラー: values が指定されていません。".to_string(),
                };
                if values.len() > 1000 {
                    return "エラー: values は1000行以内で指定してください。".to_string();
                }
                let payload = json!({ "values": values });
                let action = match args.get("action").and_then(Value::as_str) {
                    Some(action @ ("update" | "append")) => action,
                    _ => return "エラー: action は update または append を指定してください。".to_string(),
                };
                let url = if action == "append" {
                    format!("{}/{}/values/{}:append", SHEETS_BASE, spreadsheet_id, encoded_range)
                } else {
                    format!("{}/{}/values/{}", SHEETS_BASE, spreadsheet_id, encoded_range)
                };
                let response = if action == "append" {
                    client
                        .post(url)
                        .bearer_auth(token)
                        .query(&[("valueInputOption", "USER_ENTERED")])
                        .json(&payload)
                        .send()
                        .await
                } else {
                    client
                        .put(url)
                        .bearer_auth(token)
                        .query(&[("valueInputOption", "USER_ENTERED")])
                        .json(&payload)
                        .send()
                        .await
                };
                handle_text_response(response, "Google Sheets API").await
            }
            _ => "エラー: action は read/update/append のいずれかを指定してください。".to_string(),
        }
    }
}

async fn handle_text_response(response: Result<reqwest::Response, reqwest::Error>, label: &str) -> String {
    let response = match response {
        Ok(response) => response,
        Err(error) => return format!("エラー: {} への接続に失敗しました: {}", label, error),
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_success() {
        body
    } else {
        format!("エラー: {} が HTTP {} を返しました。", label, status)
    }
}

inventory::submit! { ToolReg { make: || Box::new(GoogleSheetsTool) } }
