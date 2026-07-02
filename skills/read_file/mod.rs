// skills/read_file/mod.rs - エージェント機能コネクタ（例）: ローカルのテキストファイルを読む。
//
// 安全策: 全スキル共通の許可ルール（permissions）に限定。許可ルート外・保護パス
// （C:\Windows / Program Files 等）・トラバーサル（..）を拒否。サイズ上限でトリム。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions;

const MAX_BYTES: usize = 64 * 1024;

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> String {
        "ローカルのテキストファイルを読んで内容を返す。読み取れるのは設定「許可」タブの許可ルート配下のみ。"
            .to_string()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "読みたいファイルのパス（許可ルート配下の絶対パスまたは相対パス）。"
                }
            },
            "required": ["path"]
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        let target = match permissions::resolve(cfg, path) {
            Ok(target) => target,
            Err(error) => return format!("エラー: {error}"),
        };
        match std::fs::read_to_string(&target) {
            Ok(content) => {
                if content.len() > MAX_BYTES {
                    let mut end = MAX_BYTES;
                    while !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}\n…（{} バイトで打ち切り）", &content[..end], MAX_BYTES)
                } else {
                    content
                }
            }
            Err(e) => format!("エラー: 読み取りに失敗しました（{}）: {}", e, target.display()),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ReadFileTool) } }
