// skills/document_convert/mod.rs - LibreOffice による Office/PDF 変換スキル。

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct DocumentConvertTool;

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
/// 優先順: 明示設定（PICOAGENT_LIBREOFFICE_PATH）→ 同梱（setup office）→ PATH の soffice。
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

// Pandoc の方が高精度に扱える形式（LibreOffice では崩れやすい・開けない）。
const PANDOC_FORMATS: [&str; 7] = ["md", "markdown", "rst", "epub", "org", "tex", "latex"];

/// Pandoc の実行パスを解決する。優先順: 明示設定 → 同梱（setup tools）→ PATH。
fn pandoc_path(cfg: &Config) -> String {
    super::resolve_cli_path(cfg, "PICOAGENT_PANDOC_PATH", "pandoc", "pandoc")
}

/// この変換に Pandoc を使うべきか判定する。
/// auto では Markdown/reST/EPUB 等が入出力に絡む場合だけ Pandoc（PDF 出力は LibreOffice が確実）。
fn should_use_pandoc(engine: &str, input_ext: &str, format: &str) -> bool {
    match engine {
        "pandoc" => true,
        "office" => false,
        _ => {
            format != "pdf"
                && (PANDOC_FORMATS.contains(&input_ext) || PANDOC_FORMATS.contains(&format))
        }
    }
}

#[async_trait]
impl Tool for DocumentConvertTool {
    fn name(&self) -> &'static str {
        "document_convert"
    }

    fn label(&self) -> &'static str {
        "文書変換"
    }

    fn description(&self) -> String {
        "OfficeCLI（同梱 LibreOffice。cargo xtask setup office で取得）で docx/xlsx/pptx/odt 等を PDF や別形式へ変換する。Markdown/reST/EPUB が絡む変換は Pandoc（cargo xtask setup tools で同梱）を自動で使い高精度に変換する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "input_path": { "type": "string", "description": "変換元ファイル。" },
                "output_dir": { "type": "string", "description": "出力フォルダ。省略時は入力ファイルと同じフォルダ。" },
                "format": { "type": "string", "description": "変換形式。例: pdf, docx, xlsx, pptx, html, md。" },
                "engine": { "type": "string", "enum": ["auto", "office", "pandoc"], "description": "変換エンジン。省略時 auto（Markdown 等は Pandoc、それ以外は OfficeCLI）。" }
            },
            "required": ["input_path", "format"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_LIBREOFFICE_PATH", "OfficeCLI 実行ファイルパス（空なら同梱→PATHの soffice）", "text", ""),
            Field::new("PICOAGENT_PANDOC_PATH", "Pandoc 実行ファイルパス（空なら同梱→PATH）", "text", "pandoc"),
            Field::new("PICOAGENT_DOCUMENT_CONVERT_TIMEOUT_SECS", "文書変換タイムアウト秒（5〜300）", "text", "120"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let Some(input_raw) = args.get("input_path").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
            return "エラー: input_path が指定されていません。".to_string();
        };
        let input = match crate::permissions::resolve(cfg, input_raw) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {}", error),
        };
        let format = match args.get("format").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) {
            Some(format) => format,
            None => return "エラー: format が指定されていません。".to_string(),
        };
        let output_dir = match args.get("output_dir").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()) {
            Some(path) => match crate::permissions::resolve(cfg, path) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {}", error),
            },
            None => input.parent_or_root(cfg),
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
        let engine = args.get("engine").and_then(Value::as_str).map(str::trim).unwrap_or("auto");
        let input_ext = input.extension_lossy();
        let format_lower = format.to_ascii_lowercase();
        let use_pandoc = should_use_pandoc(engine, &input_ext, &format_lower);
        let engine_label = if use_pandoc { "Pandoc" } else { "OfficeCLI" };
        // 出力先も許可判定を通す（output_dir 配下の相対名として再検証される）。
        let pandoc_output = match output_dir.join(cfg, &format!("{}.{}", input.file_stem_lossy(), format_lower)) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {}", error),
        };
        let mut command;
        if use_pandoc {
            command = crate::tools::process::command(pandoc_path(cfg));
            // --standalone で単体で開ける完結した文書（html のヘッダ等）を出力する。
            command.arg(&input).arg("--standalone").arg("-o").arg(&pandoc_output);
        } else {
            command = crate::tools::process::command(office_cli_path(cfg));
            command
                .args(["--headless", "--convert-to", format, "--outdir"])
                .arg(&output_dir)
                .arg(&input);
        }
        match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
            Ok(Ok(output)) => {
                if use_pandoc && output.status.success() && pandoc_output.exists() {
                    return format!("Pandoc で変換しました: {}", pandoc_output.display());
                }
                format!(
                    "変換エンジン: {}\n終了コード: {}\nstdout:\n{}\nstderr:\n{}",
                    engine_label,
                    output.status.code().map(|code| code.to_string()).unwrap_or_else(|| "シグナル終了".to_string()),
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            }
            Ok(Err(error)) => format!(
                "エラー: {} の起動に失敗しました: {}。cargo xtask setup {} で同梱ツールを取得してください。",
                engine_label,
                error,
                if use_pandoc { "tools" } else { "office" }
            ),
            Err(_) => format!("エラー: 文書変換が {} 秒でタイムアウトしました。", timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_use_pandoc;

    #[test]
    fn エンジン自動判定が正しい() {
        // Markdown が絡む変換は Pandoc、Office 形式や PDF 出力は OfficeCLI。
        assert!(should_use_pandoc("auto", "md", "docx"));
        assert!(should_use_pandoc("auto", "docx", "md"));
        assert!(should_use_pandoc("auto", "epub", "html"));
        assert!(!should_use_pandoc("auto", "md", "pdf")); // PDF は LaTeX 不要な OfficeCLI で
        assert!(!should_use_pandoc("auto", "docx", "pdf"));
        assert!(!should_use_pandoc("auto", "xlsx", "csv"));
        // 明示指定はそのまま従う。
        assert!(should_use_pandoc("pandoc", "docx", "pdf"));
        assert!(!should_use_pandoc("office", "md", "docx"));
    }

    // pandoc がある環境だけで走る E2E（無ければ即成功で抜ける）。
    #[test]
    fn pandocでmarkdownをhtmlへ変換できる() {
        if std::process::Command::new("pandoc").arg("--version").output().is_err() {
            eprintln!("pandoc が無いためスキップ");
            return;
        }
        let dir = std::env::temp_dir().join(format!("picoagent-pandoc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.md"), "# 見出し\n\nPandoc変換テスト本文\n").unwrap();
        let status = std::process::Command::new("pandoc")
            .arg(dir.join("note.md"))
            .arg("--standalone")
            .arg("-o")
            .arg(dir.join("note.html"))
            .status()
            .unwrap();
        assert!(status.success());
        let html = std::fs::read_to_string(dir.join("note.html")).unwrap();
        assert!(html.contains("Pandoc変換テスト本文"));
        assert!(html.contains("<h1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

inventory::submit! { ToolReg { make: || Box::new(DocumentConvertTool) } }
