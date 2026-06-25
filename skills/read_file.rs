// tools/read_file.rs - エージェント機能コネクタ（例）: ローカルのテキストファイルを読む。
//
// 安全策: 許可ルート CODA_TOOLS_ROOT（既定はホーム）配下に限定。正規化してルート外・
// トラバーサル（..）を拒否。サイズ上限でトリム。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_BYTES: usize = 64 * 1024;

pub struct ReadFileTool;

fn home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn root(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.get("CODA_TOOLS_ROOT", &home()))
}

// パスを正規化（存在しない要素も含め論理的に .. を解決）する。
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

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> String {
        "ローカルのテキストファイルを読んで内容を返す。読み取れるのは設定で指定した許可ルート配下のみ。"
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
    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new("CODA_TOOLS_ROOT", "read_file の許可ルート", "text", &home())]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        let base = root(cfg);
        let target = normalize(&base, path);
        if target != base && !target.starts_with(&base) {
            return format!(
                "エラー: 許可ルート（{}）の外は読み取れません: {}",
                base.display(),
                target.display()
            );
        }
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
