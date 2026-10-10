// skills/google_drive/mod.rs - Google Drive の検索・テキストファイル作成スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3";
const DRIVE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";
const MAX_NAME_CHARS: usize = 300;
const MAX_TEXT_BYTES: usize = 128 * 1024;

pub struct GoogleDriveTool;

fn timeout_secs(cfg: &Config) -> u64 {
    cfg.get("PICOAGENT_GOOGLE_DRIVE_TIMEOUT_SECS", "20")
        .parse::<u64>()
        .ok()
        .filter(|value| (3..=60).contains(value))
        .unwrap_or(20)
}

#[async_trait]
impl Tool for GoogleDriveTool {
    fn name(&self) -> &'static str {
        "google_drive"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["drive", "ドライブ"]
    }

    fn label(&self) -> &'static str {
        "Google Drive"
    }

    fn description(&self) -> String {
        "Google Drive API でファイル検索、メタデータ取得、テキストファイル作成を行う。設定に OAuth access token が必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["search", "get", "create_text"], "description": "search は検索、get はメタデータ取得、create_text はテキストファイル作成。" },
                "query": { "type": "string", "description": "search の Drive q 条件。例: name contains 'report' and trashed = false" },
                "file_id": { "type": "string", "description": "get のファイルID。" },
                "name": { "type": "string", "description": "create_text のファイル名。" },
                "mime_type": { "type": "string", "description": "create_text のMIME。省略時 text/plain。" },
                "content": { "type": "string", "description": "create_text の本文。" },
                "folder_id": { "type": "string", "description": "create_text の保存先フォルダID。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_GOOGLE_DRIVE_ACCESS_TOKEN",
                "Google ドライブの OAuth アクセストークン",
                "password",
                "ya29...",
            ),
            Field::new(
                "PICOAGENT_GOOGLE_DRIVE_TIMEOUT_SECS",
                "Google Drive API 待ち時間（秒、3〜60）",
                "text",
                "20",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let token = cfg.get("PICOAGENT_GOOGLE_DRIVE_ACCESS_TOKEN", "");
        if token.is_empty() {
            return super::config_guide(
                "Google Drive",
                &["PICOAGENT_GOOGLE_DRIVE_ACCESS_TOKEN"],
                "Google Drive API を使える OAuth access token が必要です。",
            );
        }
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs(cfg)))
            .user_agent("PicoAgent/1.0 (Google Drive tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                return format!("エラー: Google Drive API の準備に失敗しました: {}", error)
            }
        };
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "search" => {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("trashed = false");
                let response = match client
                    .get(format!("{}/files", DRIVE_BASE))
                    .bearer_auth(token)
                    .query(&[
                        ("q", query),
                        ("pageSize", "10"),
                        ("fields", "files(id,name,mimeType,webViewLink,modifiedTime)"),
                    ])
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return format!(
                            "エラー: Google Drive API への接続に失敗しました: {}",
                            error.without_url()
                        )
                    }
                };
                let body = crate::tool_try!(
                    crate::tools::http::service_body(cfg, "Google Drive API", Ok(response)).await
                );
                let parsed: Value = crate::tool_response_json!(&body);
                let mut out = String::from("Drive検索結果:\n");
                for file in parsed
                    .get("files")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    out.push_str(&format!(
                        "- {} [{}] {}\n",
                        file.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("（無題）"),
                        file.get("id").and_then(Value::as_str).unwrap_or(""),
                        file.get("webViewLink")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                    ));
                }
                out
            }
            "get" => {
                let Some(file_id) = args
                    .get("file_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                else {
                    return "エラー: file_id が指定されていません。".to_string();
                };
                let response = match client
                    .get(format!("{}/files/{}", DRIVE_BASE, file_id))
                    .bearer_auth(token)
                    .query(&[("fields", "id,name,mimeType,size,webViewLink,modifiedTime")])
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return format!(
                            "エラー: Google Drive API への接続に失敗しました: {}",
                            error.without_url()
                        )
                    }
                };
                crate::tools::http::service_reply(cfg, Ok(response), "Google Drive API").await
            }
            "create_text" => {
                let Some(name) = args
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                else {
                    return "エラー: name が指定されていません。".to_string();
                };
                if name.chars().count() > MAX_NAME_CHARS {
                    return format!(
                        "エラー: name は {} 文字以内で指定してください。",
                        MAX_NAME_CHARS
                    );
                }
                let content = args.get("content").and_then(Value::as_str).unwrap_or("");
                if content.len() > MAX_TEXT_BYTES {
                    return format!(
                        "エラー: content は {} バイト以内で指定してください。",
                        MAX_TEXT_BYTES
                    );
                }
                let mime_type = args
                    .get("mime_type")
                    .and_then(Value::as_str)
                    .unwrap_or("text/plain");
                let mut metadata = json!({ "name": name, "mimeType": mime_type });
                if let Some(folder_id) = args
                    .get("folder_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    metadata["parents"] = json!([folder_id]);
                }
                let boundary = "picoagent_boundary";
                let body = format!(
                    "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: {mime_type}\r\n\r\n{}\r\n--{boundary}--",
                    metadata, content
                );
                let response = match client
                    .post(format!("{}/files?uploadType=multipart", DRIVE_UPLOAD_BASE))
                    .bearer_auth(token)
                    .header(
                        "Content-Type",
                        format!("multipart/related; boundary={boundary}"),
                    )
                    .body(body)
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return format!(
                            "エラー: Google Drive API への接続に失敗しました: {}",
                            error.without_url()
                        )
                    }
                };
                let text = crate::tool_try!(
                    crate::tools::http::service_body(cfg, "Google Drive API", Ok(response)).await
                );
                let parsed: Value = crate::tool_response_json!(&text);
                format!(
                    "OK: Drive ファイルを作成しました。id={}",
                    parsed
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                )
            }
            _ => "エラー: action は search/get/create_text のいずれかを指定してください。"
                .to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(GoogleDriveTool) } }
