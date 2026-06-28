// skills/ms_office/mod.rs - Microsoft Office COM による高精度な Office ファイル編集スキル。
//
// Windows + Microsoft Office インストール環境向け。Word/Excel/PowerPoint の実エンジンを使う。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct MsOfficeTool;

const OFFICE_SCRIPT: &str = include_str!("scripts/office.ps1");

fn home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn root(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.get("PICOAGENT_DOCUMENT_ROOT", &home()))
}

fn normalize(base: &Path, input: &str) -> PathBuf {
    let joined = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        base.join(input)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => {
                out.pop();
            }
            CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn bounded_path(base: &Path, path: &str) -> Result<PathBuf, String> {
    let target = normalize(base, path);
    if target == base || target.starts_with(base) {
        Ok(target)
    } else {
        Err(format!(
            "許可ルート（{}）の外は操作できません: {}",
            base.display(),
            target.display()
        ))
    }
}

fn one_path(base: &Path, args: &Value, key: &str) -> Result<PathBuf, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{} が指定されていません。", key))?;
    bounded_path(base, raw)
}

#[async_trait]
impl Tool for MsOfficeTool {
    fn name(&self) -> &'static str {
        "ms_office"
    }

    fn label(&self) -> &'static str {
        "Microsoft Office高精度編集"
    }

    fn description(&self) -> String {
        "Windows の Microsoft Office COM を使い、Word/Excel/PowerPoint の文字列置換とPDF出力を高精度に行う。Office未インストール環境では使えない。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["replace_text", "export_pdf"], "description": "replace_text または export_pdf。" },
                "app": { "type": "string", "enum": ["word", "excel", "powerpoint"], "description": "対象Officeアプリ。" },
                "input_path": { "type": "string", "description": "入力Officeファイル。" },
                "output_path": { "type": "string", "description": "出力先。replace_text では省略時に上書き保存。" },
                "needle": { "type": "string", "description": "replace_text の置換前文字列。" },
                "replacement": { "type": "string", "description": "replace_text の置換後文字列。" }
            },
            "required": ["action", "app", "input_path"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_DOCUMENT_ROOT", "Office/PDF 操作の許可ルート", "text", &home()),
            Field::new("PICOAGENT_MS_OFFICE_TIMEOUT_SECS", "Microsoft Office 操作タイムアウト秒（5〜300）", "text", "120"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base = root(cfg);
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let app = args.get("app").and_then(Value::as_str).unwrap_or("");
        let input = match one_path(&base, &args, "input_path") {
            Ok(path) => path,
            Err(error) => return format!("エラー: {}", error),
        };
        let output = match args
            .get("output_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(path) => match bounded_path(&base, path) {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(error) => return format!("エラー: {}", error),
            },
            None => String::new(),
        };
        let timeout = cfg
            .get("PICOAGENT_MS_OFFICE_TIMEOUT_SECS", "120")
            .parse::<u64>()
            .ok()
            .filter(|value| (5..=300).contains(value))
            .unwrap_or(120);

        let script_path = std::env::temp_dir().join(format!("picoagent-ms-office-{}.ps1", std::process::id()));
        if let Err(error) = std::fs::write(&script_path, OFFICE_SCRIPT) {
            return format!("エラー: PowerShell スクリプト作成に失敗しました: {}", error);
        }
        let mut command = Command::new("powershell");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script_path)
            .args(["-Action", action, "-App", app, "-InputPath"])
            .arg(&input)
            .args(["-OutputPath", &output, "-Needle"])
            .arg(args.get("needle").and_then(Value::as_str).unwrap_or(""))
            .args(["-Replacement"])
            .arg(args.get("replacement").and_then(Value::as_str).unwrap_or(""))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let result = match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
            Ok(Ok(output)) => format!(
                "終了コード: {}\nstdout:\n{}\nstderr:\n{}",
                output.status.code().map(|code| code.to_string()).unwrap_or_else(|| "シグナル終了".to_string()),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Ok(Err(error)) => format!("エラー: PowerShell の起動に失敗しました: {}", error),
            Err(_) => format!("エラー: Microsoft Office 操作が {} 秒でタイムアウトしました。", timeout),
        };
        let _ = std::fs::remove_file(&script_path);
        result
    }
}

inventory::submit! { ToolReg { make: || Box::new(MsOfficeTool) } }
