// skills/read_file/mod.rs - エージェント機能コネクタ: ローカルのテキストファイルを読む。
//
// 行番号つきで、必要な範囲だけを読めるようにしてある（Claude Code の Read 相当）。
// 長いファイルを毎回全文読むとコンテキストを食い潰すうえ、`edit_file` で直す場所を
// 指す手掛かりも得られない。行番号は「どこを直すか」を会話で指すために付ける。
//
// 安全策: 全スキル共通の許可ルール（permissions）に限定。許可ルート外・保護パス
// （C:\Windows / Program Files 等）・トラバーサル（..）を拒否。サイズ上限でトリム。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions;
use crate::text;

/// 読み込むファイル全体のバイト上限（これを超えるものはテキスト編集の対象外とみなす）。
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// 1回で返す行数の既定と上限。
const DEFAULT_LINES: usize = 800;
const MAX_LINES: usize = 2000;
/// 1行の表示文字数（最小化されたJSやログの1行が全体を埋めるのを防ぐ）。
const MAX_LINE_CHARS: usize = 800;
/// 返す本文の合計バイト上限（行数上限を通っても、長い行だらけならここで止める）。
const MAX_OUTPUT_BYTES: usize = 128 * 1024;

pub struct ReadFileTool;

/// 指定範囲を行番号つきに整形する。`offset` は 1 始まり。
fn render(content: &str, offset: usize, limit: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    if total == 0 {
        return "（空のファイルです）".to_string();
    }
    if offset > total {
        return format!("エラー: offset({offset}) が全 {total} 行を超えています。");
    }
    let start = offset - 1;
    let end = (start + limit).min(total);
    let mut out = format!("{} 行中 {}〜{} 行目:\n", total, start + 1, end);
    let mut truncated_at = None;
    for (index, line) in lines[start..end].iter().enumerate() {
        let row = format!("{}\t{}\n", start + index + 1, text::preview(line, MAX_LINE_CHARS));
        if out.len() + row.len() > MAX_OUTPUT_BYTES {
            truncated_at = Some(start + index + 1);
            break;
        }
        out.push_str(&row);
    }
    match truncated_at {
        Some(line) => out.push_str(&format!(
            "…（出力が {MAX_OUTPUT_BYTES} バイトに達したため {line} 行目で打ち切り。offset={line} で続きを読めます）"
        )),
        None if end < total => out.push_str(&format!(
            "…（続きがあります。offset={} で次を読めます）",
            end + 1
        )),
        None => {}
    }
    out
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn label(&self) -> &'static str {
        "ファイル読込"
    }
    fn description(&self) -> String {
        "ローカルのテキストファイルを行番号つきで読む。offset / limit で長いファイルの一部だけを読める。読み取れるのは設定「許可」タブの許可ルート配下のみ。edit_file で直す前に必ずこれで現在の内容を確認する。"
            .to_string()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "読みたいファイルのパス（許可ルート配下の絶対パスまたは相対パス）。"
                },
                "offset": {
                    "type": "integer",
                    "description": "読み始める行番号（1始まり）。省略時は先頭から。"
                },
                "limit": {
                    "type": "integer",
                    "description": "読む行数（既定800、最大2000）。"
                }
            },
            "required": ["path"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["このファイルを読んで", "続きの200行を読んで"]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        let target = match permissions::resolve(cfg, path) {
            Ok(target) => target,
            Err(error) => return format!("エラー: {error}"),
        };
        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .map(|offset| (offset as usize).max(1))
            .unwrap_or(1);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| (limit as usize).clamp(1, MAX_LINES))
            .unwrap_or(DEFAULT_LINES);

        let content = match permissions::fs::read_to_string(&target) {
            Ok(content) => content,
            Err(error) => {
                return format!("エラー: 読み取りに失敗しました（{error}）: {}", target.display())
            }
        };
        if content.len() > MAX_FILE_BYTES {
            return format!(
                "エラー: {MAX_FILE_BYTES} バイトを超えるファイルは読めません（{} バイト）: {}",
                content.len(),
                target.display()
            );
        }
        format!(
            "{}\n{}\n（行頭の「行番号+タブ」は表示用です。edit_file の old_string には含めないでください）",
            target.display(),
            render(&content, offset, limit)
        )
    }
}

inventory::submit! { ToolReg { make: || Box::new(ReadFileTool) } }

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn 行番号つきで全文を返す() {
        let out = render("あ\nい\nう\n", 1, 800);
        assert!(out.contains("3 行中 1〜3 行目"), "out: {out}");
        assert!(out.contains("1\tあ\n2\tい\n3\tう\n"), "out: {out}");
        assert!(!out.contains("続きがあります"), "out: {out}");
    }

    /// 途中から読めること、続きの読み方が案内されること。
    #[test]
    fn 範囲指定で一部だけ返す() {
        let content = (1..=10).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let out = render(&content, 3, 2);
        assert!(out.contains("10 行中 3〜4 行目"), "out: {out}");
        assert!(out.contains("3\t3\n4\t4\n"), "out: {out}");
        assert!(out.contains("offset=5"), "out: {out}");
    }

    #[test]
    fn 範囲外と空ファイルを扱える() {
        assert!(render("a\n", 5, 10).starts_with("エラー:"));
        assert_eq!(render("", 1, 10), "（空のファイルです）");
    }
}
