// skills/ms_office/mod.rs - OfficeCLI によるオフィスファイル編集スキル。
//
// 旧実装は Windows + Microsoft Office COM（PowerShell）が必要だったが、
// OfficeCLI（同梱 LibreOffice。npm run setup:office で取得）と OOXML の直接編集へ置き換え、
// OS や Microsoft Office の有無に依存しないようにした。
//
//   replace_text … docx/xlsx/pptx を ZIP として開き、内部 XML の文字列を直接置換する
//                  （Office 本体を起動しないため速く、書式・レイアウトを崩さない）
//   export_pdf  … OfficeCLI（soffice --headless --convert-to pdf）で PDF 出力する

use std::io::{Read, Write};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions::SafePath;
use crate::message::Field;

pub struct MsOfficeTool;

// ZIP 1エントリの上限（zip 爆弾対策）。
const MAX_ZIP_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// 入出力パスを共通の許可ルール（許可ルート・拒否パス・リンク実体）で解決する。
fn one_path(cfg: &Config, args: &Value, key: &str) -> Result<SafePath, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{} が指定されていません。", key))?;
    crate::permissions::resolve(cfg, raw)
}

/// OOXML 内のテキストは XML エスケープされて格納されるため、検索・置換文字列も同じ形へ揃える。
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// OOXML（docx/xlsx/pptx）を ZIP として開き、XML エントリ内の文字列を置換して書き出す。
/// XML 以外のエントリ（画像など）はそのままコピーする。置換した総数を返す。
fn replace_in_ooxml(input: &SafePath, output: &SafePath, needle: &str, replacement: &str) -> Result<usize, String> {
    let file = std::fs::File::open(input).map_err(|e| format!("入力を開けません: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Office ファイルとして開けません（docx/xlsx/pptx のみ対応）: {e}"))?;

    let escaped_needle = xml_escape(needle);
    let escaped_replacement = xml_escape(replacement);

    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut total = 0usize;

    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        if name.ends_with('/') {
            writer.add_directory(&name, options).map_err(|e| e.to_string())?;
            continue;
        }
        let mut data = Vec::new();
        entry
            .take(MAX_ZIP_ENTRY_BYTES)
            .read_to_end(&mut data)
            .map_err(|e| format!("エントリ {name} を読めません: {e}"))?;
        // XML エントリだけ文字列置換の対象にする（本文・共有文字列・スライドなど）。
        if name.ends_with(".xml") {
            if let Ok(text) = String::from_utf8(data.clone()) {
                let count = text.matches(&escaped_needle).count();
                if count > 0 {
                    total += count;
                    data = text.replace(&escaped_needle, &escaped_replacement).into_bytes();
                }
            }
        }
        writer.start_file(&name, options).map_err(|e| e.to_string())?;
        writer.write_all(&data).map_err(|e| e.to_string())?;
    }
    let buffer = writer.finish().map_err(|e| e.to_string())?.into_inner();

    // 書き込みは一時ファイル経由にして、途中失敗で元ファイルを壊さない。
    // 検証済み出力の拡張子だけを変えた同階層の一時ファイル（ルート外へは出ない）。
    let temp = SafePath::app_owned(output.as_path().with_extension("picoagent-tmp"));
    std::fs::write(&temp, &buffer).map_err(|e| format!("出力を書き込めません: {e}"))?;
    std::fs::rename(&temp, output).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("出力の確定に失敗しました: {e}")
    })?;
    Ok(total)
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

/// OfficeCLI で PDF へ変換する。出力は入力と同名の .pdf（output_path 指定時はリネーム）。
async fn export_pdf(cfg: &Config, input: &SafePath, output: Option<&SafePath>, timeout: u64) -> String {
    let program = crate::tools::document_convert::office_cli_path(cfg);
    let out_dir = output.unwrap_or(input).parent_or_root(cfg);
    if let Err(error) = crate::permissions::fs::create_dir_all(&out_dir) {
        return format!("エラー: 出力フォルダ作成に失敗しました: {error}");
    }
    let mut command = Command::new(&program);
    command
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(&out_dir)
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut command);
    let result = match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return format!(
                "エラー: OfficeCLI（{program}）を起動できません: {error}\nnpm run setup:office で同梱 LibreOffice を取得するか、PICOAGENT_LIBREOFFICE_PATH を設定してください。"
            );
        }
        Err(_) => return format!("エラー: PDF 変換が {timeout} 秒でタイムアウトしました。"),
    };
    if !result.status.success() {
        return format!(
            "エラー: PDF 変換に失敗しました。\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    // 変換結果（LibreOffice が出力する入力名.pdf）も許可判定を通してから扱う。
    let converted = match out_dir.join(cfg, &format!("{}.pdf", input.file_stem_lossy())) {
        Ok(path) => path,
        Err(error) => return format!("エラー: {error}"),
    };
    // output_path 指定時は変換結果をその名前へ移す。
    if let Some(target) = output {
        if target != &converted {
            if let Err(error) = std::fs::rename(&converted, target) {
                return format!("エラー: 出力先へのリネームに失敗しました: {error}");
            }
            return format!("PDF を出力しました: {}", target.display());
        }
    }
    format!("PDF を出力しました: {}", converted.display())
}

#[async_trait]
impl Tool for MsOfficeTool {
    fn name(&self) -> &'static str {
        "ms_office"
    }

    fn label(&self) -> &'static str {
        "Office編集（OfficeCLI）"
    }

    fn description(&self) -> String {
        "OfficeCLI（同梱 LibreOffice）と OOXML 直接編集で Word/Excel/PowerPoint を扱う。\
         replace_text は docx/xlsx/pptx 内の文字列を書式を保ったまま置換、export_pdf は PDF 出力。\
         Microsoft Office のインストールは不要。"
            .to_string()
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "report.docx の「令和6年」を「令和7年」に置換して",
            "見積書.xlsx をPDFにして",
            "スライドの社名を新社名へ置き換えて",
        ]
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["replace_text", "export_pdf"], "description": "replace_text または export_pdf。" },
                "app": { "type": "string", "enum": ["word", "excel", "powerpoint"], "description": "対象の種類（互換のため任意。拡張子から自動判定する）。" },
                "input_path": { "type": "string", "description": "入力Officeファイル。" },
                "output_path": { "type": "string", "description": "出力先。replace_text では省略時に上書き保存。" },
                "needle": { "type": "string", "description": "replace_text の置換前文字列。" },
                "replacement": { "type": "string", "description": "replace_text の置換後文字列。" }
            },
            "required": ["action", "input_path"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_MS_OFFICE_TIMEOUT_SECS",
            "Office 操作タイムアウト秒（5〜300）",
            "text",
            "120",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let input = match one_path(cfg, &args, "input_path") {
            Ok(path) => path,
            Err(error) => return format!("エラー: {}", error),
        };
        if !input.exists() {
            return format!("エラー: 入力ファイルが見つかりません: {}", input.display());
        }
        let output = match args
            .get("output_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(path) => match crate::permissions::resolve(cfg, path) {
                Ok(path) => Some(path),
                Err(error) => return format!("エラー: {}", error),
            },
            None => None,
        };
        let timeout = cfg
            .get("PICOAGENT_MS_OFFICE_TIMEOUT_SECS", "120")
            .parse::<u64>()
            .ok()
            .filter(|value| (5..=300).contains(value))
            .unwrap_or(120);

        match action {
            "replace_text" => {
                let needle = args.get("needle").and_then(Value::as_str).unwrap_or("");
                if needle.is_empty() {
                    return "エラー: needle が指定されていません。".to_string();
                }
                let replacement = args.get("replacement").and_then(Value::as_str).unwrap_or("");
                let target = output.unwrap_or_else(|| input.clone());
                match replace_in_ooxml(&input, &target, needle, replacement) {
                    Ok(0) => format!(
                        "「{needle}」は見つかりませんでした（変更なしで保存: {}）。\n注意: 文字列が書式の切れ目で分割されている場合は一致しません。",
                        target.display()
                    ),
                    Ok(count) => format!("{count} 箇所を置換して保存しました: {}", target.display()),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "export_pdf" => export_pdf(cfg, &input, output.as_ref(), timeout).await,
            _ => "エラー: action は replace_text または export_pdf を指定してください。".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{replace_in_ooxml, xml_escape};
    use crate::permissions::SafePath;
    use std::io::Write;

    fn make_docx(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("test.docx");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::FileOptions::default();
        writer.start_file("word/document.xml", options).unwrap();
        writer
            .write_all(format!("<w:document><w:t>{}</w:t></w:document>", xml_escape(body)).as_bytes())
            .unwrap();
        writer.start_file("word/media/img.bin", options).unwrap();
        writer.write_all(&[0u8, 159, 146, 150]).unwrap(); // XML でないバイナリはそのまま残る
        writer.finish().unwrap();
        path
    }

    #[test]
    fn ooxml内の文字列を置換する() {
        let dir = std::env::temp_dir().join(format!("picoagent-office-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = make_docx(&dir, "旧社名<A&B>のご案内と旧社名の住所");
        let output = dir.join("out.docx");
        let count = replace_in_ooxml(&SafePath::app_owned(input.clone()), &SafePath::app_owned(output.clone()), "旧社名", "新社名").unwrap();
        assert_eq!(count, 2);

        let file = std::fs::File::open(&output).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("word/document.xml").unwrap(), &mut xml)
            .unwrap();
        assert!(xml.contains("新社名"));
        assert!(!xml.contains("旧社名"));
        assert!(xml.contains("&lt;A&amp;B&gt;")); // 既存のエスケープはそのまま
        assert!(archive.by_name("word/media/img.bin").is_ok()); // バイナリエントリは保持
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ローカルに soffice がある環境だけで走る E2E（無ければ即成功で抜ける）。
    #[test]
    fn officecliでpdf出力できる() {
        if std::process::Command::new("soffice").arg("--version").output().is_err() {
            eprintln!("soffice が無いためスキップ");
            return;
        }
        let dir = std::env::temp_dir().join(format!("picoagent-office-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // LibreOffice が読める最小構成の docx を作る。
        let docx = dir.join("hello.docx");
        {
            let file = std::fs::File::create(&docx).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default();
            writer.start_file("[Content_Types].xml", options).unwrap();
            writer.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.as_bytes()).unwrap();
            writer.start_file("_rels/.rels", options).unwrap();
            writer.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_bytes()).unwrap();
            writer.start_file("word/document.xml", options).unwrap();
            writer.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>PDF出力テスト</w:t></w:r></w:p></w:body></w:document>"#.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        let cfg = crate::config::Config::new();
        std::env::set_var("PICOAGENT_ALLOWED_ROOT", dir.to_string_lossy().to_string());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(super::export_pdf(&cfg, &SafePath::app_owned(docx.clone()), None, 120));
        std::env::remove_var("PICOAGENT_ALLOWED_ROOT");
        assert!(result.contains("PDF を出力しました"), "結果: {result}");
        assert!(dir.join("hello.pdf").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn エスケープを含む文字列も置換できる() {
        let dir = std::env::temp_dir().join(format!("picoagent-office-esc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = make_docx(&dir, "A&B社 と A&B社");
        let output = dir.join("out.docx");
        // 検索語に & を含む場合も、格納形（&amp;）へ揃えて一致する。
        let count = replace_in_ooxml(&SafePath::app_owned(input.clone()), &SafePath::app_owned(output.clone()), "A&B社", "C&D社").unwrap();
        assert_eq!(count, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

inventory::submit! { ToolReg { make: || Box::new(MsOfficeTool) } }
