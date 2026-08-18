// praise_log/mod.rs - 今日できたことを記録・一覧するスキル。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::daily::{self, PraiseEntry};

pub struct PraiseLogTool;

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn list_text(entries: &[PraiseEntry]) -> String {
    if entries.is_empty() {
        return "まだ、できたことメモはないよ。".to_string();
    }
    let mut out = String::from("最近できたこと:");
    for entry in entries.iter().rev().take(12).rev() {
        out.push_str(&format!("\n- {} {}", entry.at, entry.text));
    }
    out
}

#[async_trait]
impl Tool for PraiseLogTool {
    fn name(&self) -> &'static str {
        "praise_log"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["できた", "書けた", "終わった", "やったこと", "がんばった", "褒め", "ほめ", "praise"]
    }

    fn intent_priority(&self) -> i32 {
        880
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        if !self.matches_intent(text) {
            return None;
        }
        let args = if ["一覧", "見せて", "まとめ", "リスト"]
            .iter()
            .any(|word| text.contains(word))
        {
            json!({ "action": "list" })
        } else {
            json!({ "action": "add", "text": text })
        };
        Some(crate::tools::IntentInvocation::direct(args))
    }

    fn label(&self) -> &'static str {
        "できたことメモ"
    }

    fn description(&self) -> String {
        "ユーザーのできたこと・進捗・がんばったことを保存し、あとで振り返れるようにする。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["add", "list", "clear"] },
                "text": { "type": "string", "description": "記録するできたこと" }
            },
            "required": ["action"]
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = str_arg(&args, "action").to_lowercase();
        // 読む→直す→書き戻すの間、同じ一覧を触る並列ツール実行と競合しないようにする。
        let _guard = crate::store::lock_state(daily::PRAISE_KEY);
        let mut entries = match daily::load_praise(cfg) {
            Ok(entries) => entries,
            Err(error) => return format!("エラー: {error}"),
        };
        match action.as_str() {
            "list" => list_text(&entries),
            "clear" => {
                entries.clear();
                match daily::save_praise(cfg, &entries) {
                    Ok(()) => "できたことメモを全部消したよ。".to_string(),
                    Err(error) => format!("エラー: できたことメモの保存に失敗しました: {error}"),
                }
            }
            "add" => {
                let text = str_arg(&args, "text");
                if text.is_empty() {
                    return "エラー: 記録する内容を指定してね。".to_string();
                }
                entries.push(PraiseEntry {
                    id: daily::new_id("pr"),
                    at: daily::now_local_minute(),
                    text: text.clone(),
                });
                if entries.len() > 100 {
                    entries.drain(0..entries.len() - 100);
                }
                match daily::save_praise(cfg, &entries) {
                    Ok(()) => format!("ちゃんと覚えたよ。{text}、えらい。"),
                    Err(error) => format!("エラー: できたことメモの保存に失敗しました: {error}"),
                }
            }
            other => format!("エラー: 未知の action \"{other}\" です。add/list/clear を指定してね。"),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(PraiseLogTool) } }
