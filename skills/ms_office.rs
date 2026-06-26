// tools/ms_office.rs - Microsoft Office COM による高精度な Office ファイル編集スキル。
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

fn script() -> &'static str {
    r#"
param(
  [string]$Action,
  [string]$App,
  [string]$InputPath,
  [string]$OutputPath,
  [string]$Needle,
  [string]$Replacement
)
$ErrorActionPreference = "Stop"
function Save-Word($doc, $path) {
  if ($path) { $doc.SaveAs([ref]$path) } else { $doc.Save() }
}
function Save-Excel($wb, $path) {
  if ($path) { $wb.SaveAs($path) } else { $wb.Save() }
}
function Save-PowerPoint($pres, $path) {
  if ($path) { $pres.SaveAs($path) } else { $pres.Save() }
}
if ($Action -eq "replace_text") {
  if (-not $Needle) { throw "needle が空です" }
  if ($App -eq "word") {
    $word = New-Object -ComObject Word.Application
    $word.Visible = $false
    try {
      $doc = $word.Documents.Open($InputPath)
      $find = $doc.Content.Find
      [void]$find.Execute($Needle, $false, $true, $false, $false, $false, $true, 1, $false, $Replacement, 2)
      Save-Word $doc $OutputPath
      $doc.Close()
    } finally { $word.Quit() }
  } elseif ($App -eq "excel") {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    try {
      $wb = $excel.Workbooks.Open($InputPath)
      foreach ($ws in $wb.Worksheets) {
        if ($ws.UsedRange) { [void]$ws.UsedRange.Replace($Needle, $Replacement, 2) }
      }
      Save-Excel $wb $OutputPath
      $wb.Close($true)
    } finally { $excel.Quit() }
  } elseif ($App -eq "powerpoint") {
    $ppt = New-Object -ComObject PowerPoint.Application
    try {
      $pres = $ppt.Presentations.Open($InputPath, $false, $false, $false)
      foreach ($slide in $pres.Slides) {
        foreach ($shape in $slide.Shapes) {
          if ($shape.HasTextFrame -and $shape.TextFrame.HasText) {
            [void]$shape.TextFrame.TextRange.Replace($Needle, $Replacement)
          }
        }
      }
      Save-PowerPoint $pres $OutputPath
      $pres.Close()
    } finally { $ppt.Quit() }
  } else { throw "app は word/excel/powerpoint のいずれかです" }
  Write-Output "OK: replace_text"
} elseif ($Action -eq "export_pdf") {
  if (-not $OutputPath) { throw "output_path が必要です" }
  if ($App -eq "word") {
    $word = New-Object -ComObject Word.Application
    $word.Visible = $false
    try {
      $doc = $word.Documents.Open($InputPath)
      $doc.ExportAsFixedFormat($OutputPath, 17)
      $doc.Close($false)
    } finally { $word.Quit() }
  } elseif ($App -eq "excel") {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    try {
      $wb = $excel.Workbooks.Open($InputPath)
      $wb.ExportAsFixedFormat(0, $OutputPath)
      $wb.Close($false)
    } finally { $excel.Quit() }
  } elseif ($App -eq "powerpoint") {
    $ppt = New-Object -ComObject PowerPoint.Application
    try {
      $pres = $ppt.Presentations.Open($InputPath, $false, $false, $false)
      $pres.SaveAs($OutputPath, 32)
      $pres.Close()
    } finally { $ppt.Quit() }
  } else { throw "app は word/excel/powerpoint のいずれかです" }
  Write-Output "OK: export_pdf"
} else { throw "action は replace_text/export_pdf のいずれかです" }
"#
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
            Field::new("PICOAGENT_POWERSHELL_PATH", "PowerShell 実行ファイルパス", "text", "powershell"),
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
        if let Err(error) = std::fs::write(&script_path, script()) {
            return format!("エラー: PowerShell スクリプト作成に失敗しました: {}", error);
        }
        let mut command = Command::new(cfg.get("PICOAGENT_POWERSHELL_PATH", "powershell"));
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
