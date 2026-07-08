// skills/document_convert/mod.rs - LibreOffice による Office/PDF 変換スキル。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct DocumentConvertTool;

fn root(cfg: &Config) -> PathBuf {
    crate::permissions::primary_root(cfg)
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

/// 同梱 OfficeCLI（LibreOffice）の探索先ディレクトリ候補。
fn bundled_office_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("src-tauri/resources/bundled/office"));
        dirs.push(cwd.join("resources/bundled/office"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("resources/bundled/office"));
            dirs.push(parent.join("../Resources/resources/bundled/office"));
            dirs.push(parent.join("../../resources/bundled/office"));
        }
    }
    dirs
}

fn soffice_name() -> &'static str {
    if cfg!(windows) { "soffice.exe" } else { "soffice" }
}

/// ディレクトリ配下から soffice を再帰探索する（MSI 展開・.app などレイアウト差異に強くする）。
fn find_soffice(dir: &Path, depth: usize) -> Option<PathBuf> {
    if depth > 7 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(soffice_name()) {
            return Some(path);
        }
        if path.is_dir() {
            subdirs.push(path);
        }
    }
    subdirs.into_iter().find_map(|sub| find_soffice(&sub, depth + 1))
}

/// OfficeCLI の実行パスを解決する。
/// 優先順: 明示設定（PICOAGENT_LIBREOFFICE_PATH）→ 同梱（setup:office）→ PATH の soffice。
pub fn office_cli_path(cfg: &Config) -> String {
    let configured = cfg.get("PICOAGENT_LIBREOFFICE_PATH", "");
    if !configured.trim().is_empty() && configured != "soffice" {
        return configured;
    }
    for dir in bundled_office_dirs() {
        if let Some(found) = find_soffice(&dir, 0) {
            return found.to_string_lossy().to_string();
        }
    }
    "soffice".to_string()
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[async_trait]
impl Tool for DocumentConvertTool {
    fn name(&self) -> &'static str {
        "document_convert"
    }

    fn label(&self) -> &'static str {
        "文書変換"
    }

    fn description(&self) -> String {
        "OfficeCLI（同梱 LibreOffice。npm run setup:office で取得）で docx/xlsx/pptx/odt 等を PDF や別形式へ変換する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "input_path": { "type": "string", "description": "変換元ファイル。" },
                "output_dir": { "type": "string", "description": "出力フォルダ。省略時は入力ファイルと同じフォルダ。" },
                "format": { "type": "string", "description": "変換形式。例: pdf, docx, xlsx, pptx, html。" }
            },
            "required": ["input_path", "format"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_LIBREOFFICE_PATH", "OfficeCLI 実行ファイルパス（空なら同梱→PATHの soffice）", "text", ""),
            Field::new("PICOAGENT_DOCUMENT_CONVERT_TIMEOUT_SECS", "文書変換タイムアウト秒（5〜300）", "text", "120"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base = root(cfg);
        let Some(input_raw) = args.get("input_path").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
            return "エラー: input_path が指定されていません。".to_string();
        };
        let input = match bounded_path(&base, input_raw) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {}", error),
        };
        let format = match args.get("format").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) {
            Some(format) => format,
            None => return "エラー: format が指定されていません。".to_string(),
        };
        let output_dir = match args.get("output_dir").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) {
            Some(path) => match bounded_path(&base, path) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {}", error),
            },
            None => input.parent().unwrap_or(&base).to_path_buf(),
        };
        if let Err(error) = std::fs::create_dir_all(&output_dir) {
            return format!("エラー: 出力フォルダ作成に失敗しました: {}", error);
        }
        let timeout = cfg
            .get("PICOAGENT_DOCUMENT_CONVERT_TIMEOUT_SECS", "120")
            .parse::<u64>()
            .ok()
            .filter(|value| (5..=300).contains(value))
            .unwrap_or(120);
        let mut command = Command::new(office_cli_path(cfg));
        command
            .args(["--headless", "--convert-to", format, "--outdir"])
            .arg(&output_dir)
            .arg(&input)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        hide_window(&mut command);
        match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
            Ok(Ok(output)) => format!(
                "終了コード: {}\nstdout:\n{}\nstderr:\n{}",
                output.status.code().map(|code| code.to_string()).unwrap_or_else(|| "シグナル終了".to_string()),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Ok(Err(error)) => format!("エラー: LibreOffice の起動に失敗しました: {}", error),
            Err(_) => format!("エラー: 文書変換が {} 秒でタイムアウトしました。", timeout),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(DocumentConvertTool) } }
