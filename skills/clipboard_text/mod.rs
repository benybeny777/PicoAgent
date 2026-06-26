// clipboard_text/mod.rs - クリップボードのテキストを短く取り出すスキル。
//
// 配布時の追加権限を増やさないため、OS 標準コマンドを短時間だけ実行する。
// 取得した本文はサイズ上限で切り、モデルに要約・整形させるための素材として返す。

use std::process::Command;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::time::timeout;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct ClipboardTextTool;

async fn read_clipboard() -> Result<String, String> {
    let task = tokio::task::spawn_blocking(|| {
        #[cfg(target_os = "windows")]
        {
            Command::new("powershell")
                .args(["-NoProfile", "-Command", "Get-Clipboard -Raw"])
                .output()
        }
        #[cfg(target_os = "macos")]
        {
            Command::new("pbpaste").output()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            Command::new("sh")
                .args(["-c", "command -v wl-paste >/dev/null 2>&1 && wl-paste || xclip -selection clipboard -out"])
                .output()
        }
    });
    let output = timeout(Duration::from_secs(3), task)
        .await
        .map_err(|_| "クリップボード取得がタイムアウトしました。".to_string())?
        .map_err(|error| format!("クリップボード取得タスクに失敗しました: {error}"))?
        .map_err(|error| format!("クリップボード取得コマンドに失敗しました: {error}"))?;
    if !output.status.success() {
        return Err("クリップボードのテキストを取得できませんでした。".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[async_trait]
impl Tool for ClipboardTextTool {
    fn name(&self) -> &'static str {
        "clipboard_text"
    }

    fn label(&self) -> &'static str {
        "クリップボード要約"
    }

    fn description(&self) -> String {
        "現在のクリップボードのテキストを取得し、要約・返信文・TODO化などに使える形で返す。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": { "type": "string", "description": "summary/reply/todo など、ユーザーが望む加工方針" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_CLIPBOARD_MAX_CHARS",
            "クリップボード取得の最大文字数（500〜8000）",
            "text",
            "3000",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let max_chars = cfg
            .get("PICOAGENT_CLIPBOARD_MAX_CHARS", "3000")
            .parse::<usize>()
            .ok()
            .map(|value| value.clamp(500, 8000))
            .unwrap_or(3000);
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("summary")
            .trim();
        let text = match read_clipboard().await {
            Ok(text) if !text.is_empty() => text,
            Ok(_) => return "エラー: クリップボードにテキストがありません。".to_string(),
            Err(error) => return format!("エラー: {error}"),
        };
        let clipped: String = text.chars().take(max_chars).collect();
        let omitted = text.chars().count().saturating_sub(clipped.chars().count());
        let mut out = format!(
            "クリップボード本文（mode={mode}、最大{max_chars}文字）:\n---\n{clipped}\n---"
        );
        if omitted > 0 {
            out.push_str(&format!("\n注: 長いため末尾 {omitted} 文字を省略しました。"));
        }
        out
    }
}

inventory::submit! { ToolReg { make: || Box::new(ClipboardTextTool) } }

