// skills/ai_drive/mod.rs - ローカルAI DriveとGoogle Drive / OneDrive / iCloud Drive間の明示転送。

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{StatusCode, header};
use serde_json::{Value, json};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions::SafePath;

const GOOGLE_API: &str = "https://www.googleapis.com/drive/v3";
const GOOGLE_UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3";
const GRAPH_API: &str = "https://graph.microsoft.com/v1.0";
const MAX_FILE_BYTES: u64 = 50 * 1000 * 1000;
// Googleの256KB単位とOneDriveの320KB単位を両方満たす10MB。
const CHUNK_BYTES: usize = 10 * 1024 * 1024;

pub struct AiDriveTool;

async fn read_local(path: &SafePath) -> Result<Vec<u8>, String> {
    let path = path.clone();
    tokio::task::spawn_blocking(move || crate::permissions::fs::read(&path))
        .await
        .map_err(|error| format!("ファイル読み取りタスクが失敗しました: {error}"))?
}

async fn write_local_atomic(path: &SafePath, bytes: Vec<u8>) -> Result<(), String> {
    let path = path.clone();
    tokio::task::spawn_blocking(move || crate::store::write_atomic(path.as_path(), &bytes))
        .await
        .map_err(|error| format!("ファイル保存タスクが失敗しました: {error}"))?
        .map_err(|error| format!("原子的に保存できません: {error:#}"))
}

fn drive_root(cfg: &Config) -> Result<SafePath, String> {
    let root = crate::permissions::primary_root(cfg).join("PicoAgent-AI-Drive");
    crate::permissions::resolve(cfg, &root.to_string_lossy())
}

fn local_path(cfg: &Config, relative: &str) -> Result<SafePath, String> {
    let root = drive_root(cfg)?;
    root.join(cfg, relative.trim())
}

fn icloud_root(cfg: &Config) -> Result<SafePath, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cfg;
        Err("iCloud Drive連携はmacOSでだけ利用できます。".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or_else(|| "ホームフォルダを取得できません。".to_string())?;
        let root = home.join("Library/Mobile Documents/com~apple~CloudDocs/PicoAgent");
        crate::permissions::resolve(cfg, &root.to_string_lossy()).map_err(|error| {
            format!("iCloud Driveを許可できません: {error}。必要なら安全・許可の追加パスへiCloud Driveを追加してください。")
        })
    }
}

async fn icloud_upload(cfg: &Config, source: &SafePath, folder: &str, remote_name: &str) -> String {
    let root = match icloud_root(cfg) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    let directory = if folder.trim().is_empty() { root } else { match root.join(cfg, folder) { Ok(value) => value, Err(error) => return format!("エラー: {error}") } };
    if let Err(error) = crate::permissions::fs::create_dir_all(&directory) { return format!("エラー: {error}"); }
    let name = if remote_name.trim().is_empty() { source.file_name_lossy() } else { remote_name.to_string() };
    let target = match directory.join(cfg, &name) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    if target.exists() { return format!("エラー: iCloud Driveの保存先が既に存在します。別名または別フォルダを指定してください: {}", target.display()); }
    let bytes = match read_local(source).await { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    if bytes.len() as u64 > MAX_FILE_BYTES { return "エラー: AI Drive転送は1ファイル50 MBまでです。".to_string(); }
    write_local_atomic(&target, bytes).await
        .map(|_| format!("iCloud Driveへ保存しました: {}", target.display()))
        .unwrap_or_else(|error| format!("エラー: iCloud Driveへ保存できません: {error}"))
}

async fn icloud_download(cfg: &Config, cloud_relative: &str, local_relative: &str) -> String {
    let root = match icloud_root(cfg) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    let source = match root.join(cfg, cloud_relative) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    let target = match local_path(cfg, local_relative) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    if target.exists() { return format!("エラー: AI Driveの保存先が既に存在します。別名を指定してください: {}", target.display()); }
    let bytes = match read_local(&source).await { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    if bytes.len() as u64 > MAX_FILE_BYTES { return "エラー: AI Drive転送は1ファイル50 MBまでです。".to_string(); }
    write_local_atomic(&target, bytes).await
        .map(|_| format!("iCloud DriveからAI Driveへ保存しました: {}", target.display()))
        .unwrap_or_else(|error| format!("エラー: AI Driveへ保存できません: {error}"))
}

fn timeout(cfg: &Config, provider: &str) -> Duration {
    let key = if provider == "google" { "PICOAGENT_GOOGLE_DRIVE_TIMEOUT_SECS" } else { "PICOAGENT_MS_GRAPH_TIMEOUT_SECS" };
    crate::tools::http::timeout(cfg, key, 20, 3, 60)
}

fn token(cfg: &Config, provider: &str) -> Result<String, String> {
    let key = match provider {
        "google" => "PICOAGENT_GOOGLE_DRIVE_ACCESS_TOKEN",
        "onedrive" => "PICOAGENT_MS_GRAPH_ACCESS_TOKEN",
        _ => return Err("provider は google または onedrive を指定してください。".to_string()),
    };
    let value = cfg.get(key, "");
    if value.trim().is_empty() {
        Err(format!("設定画面で {key} を設定してください。OAuth access token が必要です。"))
    } else {
        Ok(value)
    }
}

fn encoded_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>().replace('+', "%20")
}

fn auth_error(provider: &str) -> String {
    format!("エラー: {provider} の認証更新が必要です（HTTP 401）。設定画面でアクセストークンを更新してからジョブを再試行してください。")
}

async fn response_text(response: reqwest::Response) -> String {
    crate::tools::http::response_text(response).await.unwrap_or_default()
}

async fn google_upload(cfg: &Config, path: &SafePath, folder_id: &str, job_id: &str, remote_name: &str, idempotency_key: &str) -> String {
    let token = match token(cfg, "google") { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    let bytes = match read_local(path).await { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    if bytes.is_empty() { return "エラー: 空ファイルはGoogle Driveへアップロードできません。".to_string(); }
    if bytes.len() as u64 > MAX_FILE_BYTES { return "エラー: AI Drive転送は1ファイル50 MBまでです。".to_string(); }
    let client = crate::tools::http::client();
    let stable_key = if idempotency_key.is_empty() { job_id } else { idempotency_key };
    let query = format!("appProperties has {{ key='picoagentTransferId' and value='{stable_key}' }} and trashed=false");
    let existing = match client.get(format!("{GOOGLE_API}/files"))
        .timeout(timeout(cfg, "google")).bearer_auth(&token)
        .query(&[("q", query.as_str()), ("pageSize", "1"), ("fields", "files(id,name)")]).send().await {
            Ok(value) => value, Err(error) => return format!("エラー: Google Driveの重複確認に失敗しました: {error}")
        };
    if existing.status() == StatusCode::UNAUTHORIZED { return auth_error("Google Drive"); }
    let existing_status = existing.status();
    let existing_body = response_text(existing).await;
    if !existing_status.is_success() { return format!("エラー: Google Driveが重複確認時にHTTP {existing_status}を返しました。"); }
    if serde_json::from_str::<Value>(&existing_body).ok()
        .and_then(|value| value.get("files").and_then(Value::as_array).map(|files| !files.is_empty()))
        .unwrap_or(false) {
        return format!("Google Driveへのアップロードは完了済みです: {}", path.file_name_lossy());
    }
    let name = if remote_name.trim().is_empty() { path.file_name_lossy() } else { remote_name.to_string() };
    let mut metadata = json!({"name": name, "appProperties": {"picoagentTransferId": stable_key}});
    if !folder_id.is_empty() { metadata["parents"] = json!([folder_id]); }
    let response = match client.post(format!("{GOOGLE_UPLOAD}/files?uploadType=resumable"))
        .timeout(timeout(cfg, "google")).bearer_auth(&token)
        .header("X-Upload-Content-Length", bytes.len()).json(&metadata).send().await {
            Ok(value) => value, Err(error) => return format!("エラー: Google Driveへの接続に失敗しました: {error}")
        };
    if response.status() == StatusCode::UNAUTHORIZED { return auth_error("Google Drive"); }
    if !response.status().is_success() { let status = response.status(); return format!("エラー: Google Driveがアップロード開始時にHTTP {status}を返しました。"); }
    // セッションURLは資格情報相当なので、ジョブ状態・ログ・結果へ保存しない。
    let Some(session) = response.headers().get(header::LOCATION).and_then(|value| value.to_str().ok()).map(str::to_string) else {
        return "エラー: Google Driveからアップロード先が返りませんでした。".to_string();
    };
    let mut offset = 0usize;
    while offset < bytes.len() {
        if crate::durable_jobs::is_cancelled(job_id) { return "エラー: ジョブはキャンセルされました。".to_string(); }
        let end = (offset + CHUNK_BYTES).min(bytes.len());
        let response = match client.put(&session).timeout(timeout(cfg, "google"))
            .header(header::CONTENT_LENGTH, end - offset)
            .header(header::CONTENT_RANGE, format!("bytes {offset}-{}/{}", end - 1, bytes.len()))
            .body(bytes[offset..end].to_vec()).send().await {
                Ok(value) => value, Err(_) => return "エラー: Google Driveへの送信に失敗しました。通信状態を確認して再試行してください。".to_string()
            };
        if response.status() == StatusCode::UNAUTHORIZED { return auth_error("Google Drive"); }
        if !(response.status().is_success() || response.status().as_u16() == 308) {
            let status = response.status(); return format!("エラー: Google Driveがアップロード中にHTTP {status}を返しました。");
        }
        offset = end;
    }
    format!("Google Driveへアップロードしました: {}", path.file_name_lossy())
}

async fn onedrive_upload(cfg: &Config, path: &SafePath, folder_id: &str, job_id: &str, remote_name: &str) -> String {
    let token = match token(cfg, "onedrive") { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    let bytes = match read_local(path).await { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    if bytes.is_empty() { return "エラー: 空ファイルはOneDriveへアップロードできません。".to_string(); }
    if bytes.len() as u64 > MAX_FILE_BYTES { return "エラー: AI Drive転送は1ファイル50 MBまでです。".to_string(); }
    let display_name = if remote_name.trim().is_empty() { path.file_name_lossy() } else { remote_name.to_string() };
    let name = encoded_segment(&display_name);
    let endpoint = if folder_id.is_empty() {
        format!("{GRAPH_API}/me/drive/root:/{name}:/createUploadSession")
    } else {
        format!("{GRAPH_API}/me/drive/items/{}:/{name}:/createUploadSession", encoded_segment(folder_id))
    };
    let client = crate::tools::http::client();
    let response = match client.post(endpoint).timeout(timeout(cfg, "onedrive")).bearer_auth(&token)
        .json(&json!({"item":{"@microsoft.graph.conflictBehavior":"fail","name":display_name}})).send().await {
            Ok(value) => value, Err(error) => return format!("エラー: OneDriveへの接続に失敗しました: {error}")
        };
    if response.status() == StatusCode::UNAUTHORIZED { return auth_error("OneDrive"); }
    let status = response.status();
    let body = response_text(response).await;
    if !status.is_success() { return format!("エラー: OneDriveがアップロード開始時にHTTP {status}を返しました。"); }
    // uploadUrlは事前認証済みURLなので、永続化もログ出力もしない。
    let Some(session) = serde_json::from_str::<Value>(&body).ok().and_then(|value| value.get("uploadUrl").and_then(Value::as_str).map(str::to_string)) else {
        return "エラー: OneDriveからアップロード先が返りませんでした。".to_string();
    };
    let mut offset = 0usize;
    while offset < bytes.len() {
        if crate::durable_jobs::is_cancelled(job_id) { return "エラー: ジョブはキャンセルされました。".to_string(); }
        let end = (offset + CHUNK_BYTES).min(bytes.len());
        let response = match client.put(&session).timeout(timeout(cfg, "onedrive"))
            .header(header::CONTENT_LENGTH, end - offset)
            .header(header::CONTENT_RANGE, format!("bytes {offset}-{}/{}", end - 1, bytes.len()))
            .body(bytes[offset..end].to_vec()).send().await {
                Ok(value) => value, Err(_) => return "エラー: OneDriveへの送信に失敗しました。通信状態を確認して再試行してください。".to_string()
            };
        if response.status() == StatusCode::UNAUTHORIZED { return auth_error("OneDrive"); }
        if !(response.status().is_success() || response.status() == StatusCode::ACCEPTED) {
            let status = response.status(); return format!("エラー: OneDriveがアップロード中にHTTP {status}を返しました。");
        }
        offset = end;
    }
    format!("OneDriveへアップロードしました: {}", path.file_name_lossy())
}

async fn download(cfg: &Config, provider: &str, file_id: &str, relative: &str) -> String {
    if provider == "icloud" { return icloud_download(cfg, file_id, relative).await; }
    let token = match token(cfg, provider) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    let target = match local_path(cfg, relative) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
    if target.exists() {
        return format!("エラー: AI Driveの保存先が既に存在します。上書きを避けるため別名を指定してください: {}", target.display());
    }
    let url = match provider {
        "google" => format!("{GOOGLE_API}/files/{}?alt=media", encoded_segment(file_id)),
        "onedrive" => format!("{GRAPH_API}/me/drive/items/{}/content", encoded_segment(file_id)),
        _ => return "エラー: provider は google、onedrive、icloud のいずれかを指定してください。".to_string(),
    };
    let response = match crate::tools::http::client().get(url).timeout(timeout(cfg, provider)).bearer_auth(token).send().await {
        Ok(value) => value, Err(error) => return format!("エラー: クラウドファイルの取得に失敗しました: {error}")
    };
    if response.status() == StatusCode::UNAUTHORIZED { return auth_error(if provider == "google" { "Google Drive" } else { "OneDrive" }); }
    if !response.status().is_success() { let status = response.status(); return format!("エラー: クラウドストレージがHTTP {status}を返しました。"); }
    if response.content_length().is_some_and(|size| size > MAX_FILE_BYTES) { return "エラー: AI Drive転送は1ファイル50 MBまでです。".to_string(); }
    let mut response = response;
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) as u64 > MAX_FILE_BYTES {
                    return "エラー: AI Drive転送は1ファイル50 MBまでです。".to_string();
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => return format!("エラー: クラウドファイルを読み取れません: {error}"),
        }
    }
    match write_local_atomic(&target, bytes).await {
        Ok(()) => format!("AI Driveへ保存しました: {}", target.display()),
        Err(error) => format!("エラー: AI Driveへ保存できません: {error}"),
    }
}

#[async_trait]
impl Tool for AiDriveTool {
    fn name(&self) -> &'static str { "ai_drive" }
    fn label(&self) -> &'static str { "AI Drive" }
    fn intent_keywords(&self) -> &'static [&'static str] { &["ai drive", "onedrive", "google drive", "icloud", "クラウド保存"] }
    fn description(&self) -> String {
        "許可ルート内のPicoAgent-AI-Driveを正本にし、Google Drive／OneDrive／macOSのiCloud Driveとのアップロード・ダウンロードを永続ジョブで行う。アップロードは送信先を示した明示確認が必要。".to_string()
    }
    fn parameters(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "action":{"type":"string","enum":["list","import","upload","download"]},
                "provider":{"type":"string","enum":["google","onedrive","icloud"]},
                "relative_path":{"type":"string","description":"AI Driveルートからの相対パス。"},
                "source_path":{"type":"string","description":"importする許可済みローカルファイル。"},
                "file_id":{"type":"string","description":"downloadするクラウドファイルID。iCloudではPicoAgentフォルダ内の相対パス。"},
                "folder_id":{"type":"string","description":"upload先フォルダID。iCloudではPicoAgentフォルダ内の相対フォルダ。空ならルート。"},
                "confirm":{"type":"boolean","description":"アップロード内容と送信先を確認済みならtrue。"}
            },"required":["action"]
        })
    }
    async fn run(&self, cfg: &Config, mut args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        if let Some(job_id) = crate::durable_jobs::worker_job_id(&args).map(str::to_string) {
            let provider = args.get("provider").and_then(Value::as_str).unwrap_or("");
            return match action {
                "upload_execute" => {
                    let relative = args.get("relative_path").and_then(Value::as_str).unwrap_or("");
                    let path = match local_path(cfg, relative) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
                    let folder = args.get("folder_id").and_then(Value::as_str).unwrap_or("");
                    let remote_name = args.get("remote_name").and_then(Value::as_str).unwrap_or("");
                    let idempotency_key = args.get("idempotency_key").and_then(Value::as_str).unwrap_or("");
                    match provider {
                        "google" => google_upload(cfg, &path, folder, &job_id, remote_name, idempotency_key).await,
                        "onedrive" => onedrive_upload(cfg, &path, folder, &job_id, remote_name).await,
                        "icloud" => icloud_upload(cfg, &path, folder, remote_name).await,
                        _ => "エラー: providerが不正です。".to_string(),
                    }
                }
                "download_execute" => download(cfg, provider,
                    args.get("file_id").and_then(Value::as_str).unwrap_or(""),
                    args.get("relative_path").and_then(Value::as_str).unwrap_or("")).await,
                _ => "エラー: 永続ジョブの操作が不正です。".to_string(),
            };
        }
        match action {
            "list" => {
                let root = match drive_root(cfg) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
                if let Err(error) = crate::permissions::fs::create_dir_all(&root) { return format!("エラー: {error}"); }
                let entries = match crate::permissions::fs::read_dir(&root) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
                let mut out = format!("AI Drive: {}\n", root.display());
                for entry in entries.flatten().take(100) { out.push_str(&format!("- {}\n", entry.file_name().to_string_lossy())); }
                out
            }
            "import" => {
                let source_raw = args.get("source_path").and_then(Value::as_str).unwrap_or("");
                let source = match crate::permissions::resolve(cfg, source_raw) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
                let relative = args.get("relative_path").and_then(Value::as_str).filter(|value| !value.trim().is_empty()).map(str::to_string).unwrap_or_else(|| source.file_name_lossy());
                let target = match local_path(cfg, &relative) { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
                let bytes = match read_local(&source).await { Ok(value) => value, Err(error) => return format!("エラー: {error}") };
                if bytes.len() as u64 > MAX_FILE_BYTES { return "エラー: AI Driveへの取り込みは1ファイル50 MBまでです。".to_string(); }
                if target.exists() { return format!("エラー: AI Driveの保存先が既に存在します。別名を指定してください: {}", target.display()); }
                write_local_atomic(&target, bytes).await.map(|_| format!("AI Driveへ取り込みました: {}", target.display())).unwrap_or_else(|error| format!("エラー: AI Driveへ取り込めません: {error}"))
            }
            "upload" => {
                let provider = args.get("provider").and_then(Value::as_str).unwrap_or("").to_string();
                if !matches!(provider.as_str(), "google" | "onedrive" | "icloud") { return "エラー: provider は google、onedrive、icloud のいずれかを指定してください。".to_string(); }
                if provider == "icloud" && cfg!(not(target_os = "macos")) { return "エラー: iCloud Drive連携はmacOSでだけ利用できます。".to_string(); }
                let relative = args.get("relative_path").and_then(Value::as_str).unwrap_or("").to_string();
                let folder = args.get("folder_id").and_then(Value::as_str).unwrap_or("ルート").to_string();
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    return format!("確認が必要です。AI Driveの「{relative}」を {provider} のフォルダ「{folder}」へ送信します。内容と送信先を確認し、confirm=true で再実行してください。");
                }
                if let Some(object) = args.as_object_mut() { object.insert("action".into(), Value::String("upload_execute".into())); object.remove("confirm"); }
                crate::durable_jobs::enqueue("ai_drive", args, format!("{provider}へアップロード: {relative}"))
                    .map(|id| format!("アップロードをジョブへ登録しました: {id}"))
                    .unwrap_or_else(|error| format!("エラー: {error}"))
            }
            "download" => {
                let provider = args.get("provider").and_then(Value::as_str).unwrap_or("").to_string();
                if !matches!(provider.as_str(), "google" | "onedrive" | "icloud") { return "エラー: provider は google、onedrive、icloud のいずれかを指定してください。".to_string(); }
                let relative = args.get("relative_path").and_then(Value::as_str).unwrap_or("").to_string();
                if args.get("file_id").and_then(Value::as_str).unwrap_or("").is_empty() || relative.is_empty() { return "エラー: file_id と relative_path が必要です。".to_string(); }
                if let Some(object) = args.as_object_mut() { object.insert("action".into(), Value::String("download_execute".into())); }
                crate::durable_jobs::enqueue("ai_drive", args, format!("{provider}からダウンロード: {relative}"))
                    .map(|id| format!("ダウンロードをジョブへ登録しました: {id}"))
                    .unwrap_or_else(|error| format!("エラー: {error}"))
            }
            _ => "エラー: action は list/import/upload/download のいずれかです。".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn onedriveのファイル名をurl用に符号化する() {
        assert_eq!(encoded_segment("月次 報告#.pptx"), "%E6%9C%88%E6%AC%A1%20%E5%A0%B1%E5%91%8A%23.pptx");
    }
    #[test]
    fn チャンク寸法は両サービスの単位を満たす() {
        assert_eq!(CHUNK_BYTES % (256 * 1024), 0);
        assert_eq!(CHUNK_BYTES % (320 * 1024), 0);
    }
}

inventory::submit! { ToolReg { make: || Box::new(AiDriveTool) } }
