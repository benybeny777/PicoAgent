// clipboard_text/mod.rs - クリップボードのテキストを短く取り出すスキル。
//
// 取得は OS の API を直接叩く（`arboard`）。以前は Windows で PowerShell の
// `Get-Clipboard`、macOS で `pbpaste`、Linux で `sh -c "wl-paste || xclip"` を
// 起動していたが、(1) PowerShell の起動は重く実行ポリシーの影響を受ける、
// (2) Linux は xclip / wl-paste を別途入れないと動かない、(3) たかがクリップボード
// 取得のためにシェルを持ち出す必要がない、という理由でネイティブへ置き換えた。
// 取得した本文はサイズ上限で切り、モデルに要約・整形させるための素材として返す。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct ClipboardTextTool;

const RESULT_MATERIAL_PROMPT: &str = include_str!("prompts/result-material.md");

fn render_result_material(user_text: &str, tool_result: &str) -> String {
    crate::tools::render_intent_prompt(RESULT_MATERIAL_PROMPT, user_text, tool_result)
}

async fn read_clipboard() -> Result<String, String> {
    // arboard は同期 API なので、UI を止めないようブロッキングスレッドへ逃がす。
    tokio::task::spawn_blocking(|| {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|error| format!("クリップボードを開けません: {error}"))?;
        clipboard
            .get_text()
            .map(|text| text.trim().to_string())
            .map_err(|error| format!("クリップボードのテキストを取得できません: {error}"))
    })
    .await
    .map_err(|error| format!("クリップボード取得に失敗しました: {error}"))?
}

#[async_trait]
impl Tool for ClipboardTextTool {
    fn name(&self) -> &'static str {
        "clipboard_text"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["クリップボード", "コピーした", "clipboard"]
    }

    fn intent_priority(&self) -> i32 {
        500
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        if !self.matches_intent(text) {
            return None;
        }
        let mode = if text.contains("todo") || text.contains("TODO") || text.contains("タスク") {
            "todo"
        } else if text.contains("返信") || text.contains("返事") {
            "reply"
        } else {
            "summary"
        };
        Some(crate::tools::IntentInvocation::material(
            json!({ "mode": mode }),
            render_result_material,
            false,
        ))
    }

    fn label(&self) -> &'static str {
        "クリップボード要約"
    }

    fn description(&self) -> String {
        "現在のクリップボードのテキストを取得し、要約・返信文・TODO化などに使える形で返す。"
            .to_string()
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
        vec![Field::int(&crate::config::ints::CLIPBOARD_MAX_CHARS)]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let max_chars =
            crate::tool_try!(cfg.int(&crate::config::ints::CLIPBOARD_MAX_CHARS)) as usize;
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
        let mut out =
            format!("クリップボード本文（mode={mode}、最大{max_chars}文字）:\n---\n{clipped}\n---");
        if omitted > 0 {
            out.push_str(&format!(
                "\n注: 長いため末尾 {omitted} 文字を省略しました。"
            ));
        }
        out
    }
}

inventory::submit! { ToolReg { make: || Box::new(ClipboardTextTool) } }
