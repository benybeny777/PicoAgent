// skills/pdf_tools/mod.rs - qpdf による PDF 編集スキル。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct PdfToolsTool;

fn home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn root(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.get("PICOAGENT_DOCUMENT_ROOT", &home()))
}

fn normalize(base: &Path, input: &str) -> PathBuf {
    let joined = if Path::new(input).is_absolute() { PathBuf::from(input) } else { base.join(input) };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => { out.pop(); }
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
        Err(format!("許可ルート（{}）の外は操作できません: {}", base.display(), target.display()))
    }
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[async_trait]
impl Tool for PdfToolsTool {
    fn name(&self) -> &'static str {
        "pdf_tools"
    }

    fn label(&self) -> &'static str {
        "PDF編集"
    }

    fn description(&self) -> String {
        "qpdf を使ってPDF情報取得、ページ抽出、結合、回転、線形化を行う。設定に qpdf 実行ファイルパスが必要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["info", "split_pages", "merge", "rotate", "linearize"], "description": "PDF操作。" },
                "input_path": { "type": "string", "description": "入力PDF。" },
                "input_paths": { "type": "array", "items": { "type": "string" }, "description": "merge の入力PDF配列。" },
                "output_path": { "type": "string", "description": "出力PDF。" },
                "pages": { "type": "string", "description": "split_pages のページ範囲。例: 1-3,5,z" },
                "rotate": { "type": "string", "description": "rotate 指定。例: +90:1-z" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_DOCUMENT_ROOT", "Office/PDF 操作の許可ルート", "text", &home()),
            Field::new("PICOAGENT_QPDF_PATH", "qpdf 実行ファイルパス", "text", "qpdf"),
            Field::new("PICOAGENT_PDF_TIMEOUT_SECS", "PDF操作タイムアウト秒（5〜300）", "text", "120"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base = root(cfg);
        let timeout = cfg
            .get("PICOAGENT_PDF_TIMEOUT_SECS", "120")
            .parse::<u64>()
            .ok()
            .filter(|value| (5..=300).contains(value))
            .unwrap_or(120);
        let mut command = Command::new(cfg.get("PICOAGENT_QPDF_PATH", "qpdf"));
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "info" => {
                let input = match one_path(&base, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                command.arg("--show-npages").arg(input);
            }
            "split_pages" => {
                let input = match one_path(&base, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let output = match one_path(&base, &args, "output_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let pages = args.get("pages").and_then(Value::as_str).unwrap_or("1-z");
                command.arg(input).arg("--pages").arg(".").arg(pages).arg("--").arg(output);
            }
            "merge" => {
                let output = match one_path(&base, &args, "output_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                command.arg("--empty").arg("--pages");
                let Some(paths) = args.get("input_paths").and_then(Value::as_array) else {
                    return "エラー: input_paths が指定されていません。".to_string();
                };
                for value in paths {
                    let Some(raw) = value.as_str() else {
                        return "エラー: input_paths は文字列配列で指定してください。".to_string();
                    };
                    let path = match bounded_path(&base, raw) {
                        Ok(path) => path,
                        Err(error) => return format!("エラー: {}", error),
                    };
                    command.arg(path).arg("1-z");
                }
                command.arg("--").arg(output);
            }
            "rotate" => {
                let input = match one_path(&base, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let output = match one_path(&base, &args, "output_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let rotate = args.get("rotate").and_then(Value::as_str).unwrap_or("+90:1-z");
                command.arg("--rotate").arg(rotate).arg(input).arg(output);
            }
            "linearize" => {
                let input = match one_path(&base, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let output = match one_path(&base, &args, "output_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                command.arg("--linearize").arg(input).arg(output);
            }
            _ => return "エラー: action は info/split_pages/merge/rotate/linearize のいずれかを指定してください。".to_string(),
        }
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        hide_window(&mut command);
        match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
            Ok(Ok(output)) => format!(
                "終了コード: {}\nstdout:\n{}\nstderr:\n{}",
                output.status.code().map(|code| code.to_string()).unwrap_or_else(|| "シグナル終了".to_string()),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Ok(Err(error)) => format!("エラー: qpdf の起動に失敗しました: {}", error),
            Err(_) => format!("エラー: PDF操作が {} 秒でタイムアウトしました。", timeout),
        }
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

inventory::submit! { ToolReg { make: || Box::new(PdfToolsTool) } }
