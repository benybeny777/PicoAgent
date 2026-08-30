// skills/pdf_tools/mod.rs - qpdf による PDF 編集スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::permissions::SafePath;

pub struct PdfToolsTool;

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

#[async_trait]
impl Tool for PdfToolsTool {
    fn name(&self) -> &'static str {
        "pdf_tools"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["pdf", "結合", "抽出", "回転"]
    }

    fn label(&self) -> &'static str {
        "PDF編集"
    }

    fn description(&self) -> String {
        "qpdf でPDF情報取得、ページ抽出、結合、回転、線形化を行い、Poppler pdftotext でPDF本文のテキスト抽出（extract_text）も行う。ツールは同梱（cargo xtask setup tools）→PATH の順で解決。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["info", "split_pages", "merge", "rotate", "linearize", "extract_text"], "description": "PDF操作。extract_text はPDF本文のテキスト抽出。" },
                "input_path": { "type": "string", "description": "入力PDF。" },
                "input_paths": { "type": "array", "items": { "type": "string" }, "description": "merge の入力PDF配列。" },
                "output_path": { "type": "string", "description": "出力先。extract_text で省略するとテキストを直接返す。" },
                "pages": { "type": "string", "description": "split_pages のページ範囲。例: 1-3,5,z" },
                "rotate": { "type": "string", "description": "rotate 指定。例: +90:1-z" },
                "first_page": { "type": "integer", "description": "extract_text の開始ページ（1始まり、省略可）。" },
                "last_page": { "type": "integer", "description": "extract_text の終了ページ（省略可）。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_QPDF_PATH",
                "qpdf 実行ファイルパス（空ならPicoAgent内→PATH）",
                "text",
                "qpdf",
            ),
            Field::new(
                "PICOAGENT_PDFTOTEXT_PATH",
                "pdftotext 実行ファイルパス（空なら同梱→PATH）",
                "text",
                "pdftotext",
            ),
            Field::new(
                "PICOAGENT_PDF_TIMEOUT_SECS",
                "PDF操作タイムアウト秒（5〜300）",
                "text",
                "120",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let timeout = cfg
            .get("PICOAGENT_PDF_TIMEOUT_SECS", "120")
            .parse::<u64>()
            .ok()
            .filter(|value| (5..=300).contains(value))
            .unwrap_or(120);
        // extract_text だけは qpdf ではなく Poppler pdftotext を使う。
        if args.get("action").and_then(Value::as_str) == Some("extract_text") {
            return extract_text(cfg, &args, timeout).await;
        }
        let mut command = crate::tools::process::command(super::resolve_cli_path(
            cfg,
            "PICOAGENT_QPDF_PATH",
            "qpdf",
            "qpdf",
        ));
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "info" => {
                let input = match one_path(cfg, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                command.arg("--show-npages").arg(input);
            }
            "split_pages" => {
                let input = match one_path(cfg, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let output = match one_path(cfg, &args, "output_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let pages = args.get("pages").and_then(Value::as_str).unwrap_or("1-z");
                command.arg(input).arg("--pages").arg(".").arg(pages).arg("--").arg(output);
            }
            "merge" => {
                let output = match one_path(cfg, &args, "output_path") {
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
                    let path = match crate::permissions::resolve(cfg, raw) {
                        Ok(path) => path,
                        Err(error) => return format!("エラー: {}", error),
                    };
                    command.arg(path).arg("1-z");
                }
                command.arg("--").arg(output);
            }
            "rotate" => {
                let input = match one_path(cfg, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let output = match one_path(cfg, &args, "output_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let rotate = args.get("rotate").and_then(Value::as_str).unwrap_or("+90:1-z");
                command.arg("--rotate").arg(rotate).arg(input).arg(output);
            }
            "linearize" => {
                let input = match one_path(cfg, &args, "input_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                let output = match one_path(cfg, &args, "output_path") {
                    Ok(path) => path,
                    Err(error) => return format!("エラー: {}", error),
                };
                command.arg("--linearize").arg(input).arg(output);
            }
            _ => return "エラー: action は info/split_pages/merge/rotate/linearize/extract_text のいずれかを指定してください。".to_string(),
        }
        match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
            Ok(Ok(output)) => format!(
                "終了コード: {}\nstdout:\n{}\nstderr:\n{}",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "シグナル終了".to_string()),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Ok(Err(error)) => format!("エラー: qpdf の起動に失敗しました: {}", error),
            Err(_) => format!("エラー: PDF操作が {} 秒でタイムアウトしました。", timeout),
        }
    }
}

/// Poppler pdftotext でPDF本文をテキスト抽出する。output_path 省略時は本文をそのまま返す。
async fn extract_text(cfg: &Config, args: &Value, timeout: u64) -> String {
    let input = match one_path(cfg, args, "input_path") {
        Ok(path) => path,
        Err(error) => return format!("エラー: {}", error),
    };
    let mut command = crate::tools::process::command(super::resolve_cli_path(
        cfg,
        "PICOAGENT_PDFTOTEXT_PATH",
        "poppler",
        "pdftotext",
    ));
    // -layout は表や段組みのレイアウトを保ってテキスト化する。
    command.arg("-layout");
    if let Some(first) = args.get("first_page").and_then(Value::as_u64) {
        command.arg("-f").arg(first.to_string());
    }
    if let Some(last) = args.get("last_page").and_then(Value::as_u64) {
        command.arg("-l").arg(last.to_string());
    }
    command.arg(&input);
    let output_path = match args
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(raw) => match crate::permissions::resolve(cfg, raw) {
            Ok(path) => Some(path),
            Err(error) => return format!("エラー: {}", error),
        },
        None => None,
    };
    match &output_path {
        Some(path) => {
            command.arg(path);
        }
        None => {
            command.arg("-");
        }
    }
    match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
        Ok(Ok(output)) => {
            if !output.status.success() {
                return format!(
                    "エラー: pdftotext が失敗しました（終了コード {}）:\n{}",
                    output.status.code().map(|code| code.to_string()).unwrap_or_else(|| "シグナル終了".to_string()),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            match output_path {
                Some(path) => format!("PDFのテキストを抽出しました: {}", path.display()),
                None => {
                    let text = String::from_utf8_lossy(&output.stdout);
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        format!("{} からテキストは抽出されませんでした（画像のみのPDFの可能性）。", input.display())
                    } else {
                        format!("{} の抽出テキスト:\n{}", input.display(), trimmed)
                    }
                }
            }
        }
        Ok(Err(error)) => format!(
            "エラー: pdftotext の起動に失敗しました: {}。設定の「追加機能」で公式取得元を確認し、Popplerを導入してください。",
            error
        ),
        Err(_) => format!("エラー: テキスト抽出が {} 秒でタイムアウトしました。", timeout),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn have(cmd: &str) -> bool {
        std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .is_ok()
    }

    // パス解決は共通の permissions::resolve に委ねる（許可ルート外・リンク実体の
    // 逸脱を各スキルで独自判定しない）。統一が外れないよう固定する。
    #[test]
    fn 許可ルート外のパスを拒否する() {
        let dir = std::env::temp_dir().join(format!("picoagent-pdf-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &dir.to_string_lossy());

        // ルート配下の相対パスは解決できる。
        let inside =
            super::one_path(&cfg, &json!({ "input_path": "a.pdf" }), "input_path").unwrap();
        assert!(
            inside.as_path().starts_with(&dir),
            "inside: {}",
            inside.display()
        );

        // ルート外の絶対パスと親参照は拒否する。
        assert!(
            super::one_path(&cfg, &json!({ "input_path": "/etc/passwd" }), "input_path").is_err()
        );
        assert!(super::one_path(
            &cfg,
            &json!({ "input_path": "../secret.pdf" }),
            "input_path"
        )
        .is_err());
        // 未指定はエラー。
        assert!(super::one_path(&cfg, &json!({}), "input_path").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // pdftotext / soffice がある環境だけで走る E2E（無ければ即成功で抜ける）。
    // soffice でテキストからPDFを作り、pdftotext で本文が取り出せることを確認する。
    #[test]
    fn pdfから本文テキストを抽出できる() {
        if !have("pdftotext") || !have("soffice") {
            eprintln!("pdftotext / soffice が無いためスキップ");
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("picoagent-pdf-extract-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "PicoAgent PDF extraction test").unwrap();
        // 並行テスト（ms_office の soffice E2E）とプロファイルロックが競合しないよう、
        // このテスト専用のユーザープロファイルで LibreOffice を起動する。
        let profile = format!(
            "-env:UserInstallation=file://{}",
            dir.join("lo-profile").to_string_lossy()
        );
        let status = std::process::Command::new("soffice")
            .arg(&profile)
            .args(["--headless", "--convert-to", "pdf", "--outdir"])
            .arg(&dir)
            .arg(dir.join("hello.txt"))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(dir.join("hello.pdf").exists());

        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &dir.to_string_lossy());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let args = json!({ "action": "extract_text", "input_path": "hello.pdf" });
        let result = runtime.block_on(super::extract_text(&cfg, &args, 120));
        assert!(
            result.contains("PicoAgent PDF extraction test"),
            "抽出結果: {result}"
        );

        // output_path 指定時はファイルへ保存される。
        let args = json!({ "action": "extract_text", "input_path": "hello.pdf", "output_path": "hello.txt.out" });
        let result = runtime.block_on(super::extract_text(&cfg, &args, 120));
        assert!(result.contains("抽出しました"), "保存結果: {result}");
        let saved = std::fs::read_to_string(dir.join("hello.txt.out")).unwrap();
        assert!(saved.contains("PicoAgent PDF extraction test"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

inventory::submit! { ToolReg { make: || Box::new(PdfToolsTool) } }
