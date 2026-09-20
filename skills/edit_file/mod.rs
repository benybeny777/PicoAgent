// skills/edit_file/mod.rs - 既存ファイルの一部だけを厳密一致の文字列置換で書き換えるスキル。
//
// ファイル全体を渡し直す `workspace_files` の write と違い、「元の文字列」と「新しい文字列」を
// 渡して該当箇所だけを差し替える。モデルが長いファイルを丸ごと再生成せずに済むため、
// 出力トークンが減り、書き直しの過程で周辺行を落とす事故も防げる。
//
// 一意性の検査（一致0件・複数件をエラーにする）が安全装置の中心で、
// 「どこを直すつもりだったのか」が曖昧なまま書き換わることを防ぐ。
// 許可判定・書き込み・取り消し履歴は `workspace_files` と同じ共通経路を使う。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions;
use crate::text;

/// 編集対象として読み込むファイルサイズの上限。
const MAX_FILE_BYTES: usize = 512 * 1024;
/// 1回の呼び出しで適用できる置換の件数上限。
const MAX_EDITS: usize = 20;
/// エラー文やプレビューに載せる文字列の長さ。
const SNIPPET_CHARS: usize = 120;

struct Edit {
    old: String,
    new: String,
    replace_all: bool,
}

/// 適用結果（何行目を何件置換したか）。プレビューと実行で同じ表示に使う。
#[derive(Debug)]
struct Applied {
    line: usize,
    hits: usize,
}

fn one_edit(item: &Value) -> Result<Edit, String> {
    let old = item
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or_else(|| "old_string が指定されていません。".to_string())?;
    let new = item
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| "new_string が指定されていません。".to_string())?;
    if old.is_empty() {
        return Err(
            "old_string が空です。新規作成や全文の書き換えは workspace_files の write を使ってください。"
                .to_string(),
        );
    }
    if old == new {
        return Err("old_string と new_string が同じです。変更がありません。".to_string());
    }
    Ok(Edit {
        old: old.to_string(),
        new: new.to_string(),
        replace_all: item
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// `edits` 配列（複数置換）と、単発の old_string / new_string の両方を受け付ける。
fn parse_edits(args: &Value) -> Result<Vec<Edit>, String> {
    if let Some(list) = args.get("edits").and_then(Value::as_array) {
        if list.is_empty() {
            return Err("edits が空です。".to_string());
        }
        if list.len() > MAX_EDITS {
            return Err(format!("edits は {MAX_EDITS} 件以内で指定してください。"));
        }
        let mut edits = Vec::with_capacity(list.len());
        for (index, item) in list.iter().enumerate() {
            edits.push(one_edit(item).map_err(|error| format!("edits[{index}]: {error}"))?);
        }
        return Ok(edits);
    }
    Ok(vec![one_edit(args)?])
}

/// 1件の置換を適用する。一致0件・（replace_all でない）複数一致はエラーにする。
fn apply(content: &str, edit: &Edit, number: usize) -> Result<(String, Applied), String> {
    let hits = content.matches(edit.old.as_str()).count();
    if hits == 0 {
        return Err(format!(
            "編集{number}: old_string が見つかりません。読み込んだ内容をそのまま（空白・改行を含めて）指定してください: {}",
            text::preview(&edit.old, SNIPPET_CHARS)
        ));
    }
    if hits > 1 && !edit.replace_all {
        return Err(format!(
            "編集{number}: old_string が {hits} 箇所に一致します。前後の行を足して一意にするか、全件置換なら replace_all=true を指定してください: {}",
            text::preview(&edit.old, SNIPPET_CHARS)
        ));
    }
    // 先頭一致の位置から行番号を求める（変数添字なので文字境界は find が保証する）。
    let offset = content.find(edit.old.as_str()).unwrap_or(0);
    let line = content[..offset].lines().count() + 1;
    let replaced = if edit.replace_all {
        content.replace(edit.old.as_str(), edit.new.as_str())
    } else {
        content.replacen(edit.old.as_str(), edit.new.as_str(), 1)
    };
    Ok((replaced, Applied { line, hits }))
}

fn summary(edits: &[Edit], applied: &[Applied]) -> String {
    let mut out = String::new();
    for (index, (edit, result)) in edits.iter().zip(applied).enumerate() {
        out.push_str(&format!(
            "\n編集{}（{}行目付近 / {}件）\n  - {}\n  + {}",
            index + 1,
            result.line,
            result.hits,
            text::preview(&edit.old, SNIPPET_CHARS),
            text::preview(&edit.new, SNIPPET_CHARS)
        ));
    }
    out
}

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["編集", "書き換え", "置換", "直して", "修正", "リファクタ"]
    }

    fn label(&self) -> &'static str {
        "ファイル部分編集"
    }

    fn description(&self) -> String {
        "既存ファイルの一部を、元の文字列と新しい文字列を指定して厳密一致で置換する。ファイル全体を書き直さずにコードや文書を直したいときに使う。まず read_file か workspace_files の read で現在の内容を確認してから呼ぶこと。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "編集する既存ファイルのパス（許可ルート配下の絶対パスまたは相対パス）。"
                },
                "old_string": {
                    "type": "string",
                    "description": "置換前の文字列。ファイル内で一意になるよう前後の行を含めて指定する。"
                },
                "new_string": {
                    "type": "string",
                    "description": "置換後の文字列。削除したい場合は空文字列を指定する。"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "true なら一致箇所を全件置換する。省略時は複数一致をエラーにする。"
                },
                "edits": {
                    "type": "array",
                    "description": "複数箇所をまとめて編集する場合の一覧（上から順に適用する）。指定した場合は old_string / new_string より優先する。",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" },
                            "replace_all": { "type": "boolean" }
                        },
                        "required": ["old_string", "new_string"]
                    }
                },
                "preview": {
                    "type": "boolean",
                    "description": "true なら書き込まずに置換結果の要約だけを返す。"
                }
            },
            "required": ["path"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "src/main.rs の関数名を直して",
            "README のこの一文だけ書き換えて",
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let path = match args.get("path").and_then(Value::as_str) {
            Some(path) if !path.trim().is_empty() => path.trim(),
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        let edits = match parse_edits(&args) {
            Ok(edits) => edits,
            Err(error) => return format!("エラー: {error}"),
        };
        // 許可ルート内かつ保護パスでないことを共通ルールで検証する。
        let target = match permissions::resolve(cfg, path) {
            Ok(target) => target,
            Err(error) => return format!("エラー: {error}"),
        };
        if !target.exists() {
            return format!(
                "エラー: ファイルがありません。新規作成は workspace_files の write を使ってください: {}",
                target.display()
            );
        }
        // 読む前に実寸で弾く（読んでから弾くと上限ぶんを超えて抱えてしまう）。
        if let Err(size) = permissions::fs::ensure_within(&target, MAX_FILE_BYTES) {
            return format!(
                "エラー: {} バイトを超えるファイルは編集できません（{size} バイト）: {}",
                MAX_FILE_BYTES,
                target.display()
            );
        }
        let mut content = match permissions::fs::read_to_string(&target) {
            Ok(content) => content,
            Err(error) => {
                return format!(
                    "エラー: 読み取りに失敗しました（{error}）: {}",
                    target.display()
                )
            }
        };

        let mut applied = Vec::with_capacity(edits.len());
        for (index, edit) in edits.iter().enumerate() {
            match apply(&content, edit, index + 1) {
                Ok((next, result)) => {
                    content = next;
                    applied.push(result);
                }
                // 途中で失敗したら1件も書き込まない（部分適用の中途半端な状態を残さない）。
                Err(error) => return format!("エラー: {error}"),
            }
        }

        if args
            .get("preview")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return format!(
                "プレビュー: {} を {} 箇所編集します（未書き込み）。{}",
                target.display(),
                applied.len(),
                summary(&edits, &applied)
            );
        }

        // 書き込みと取り消し履歴は workspace_files と共有する（同じファイルを触るため直列化も共通）。
        let _mutation = match super::workspace_files::file_mutation().lock() {
            Ok(guard) => guard,
            Err(_) => return "エラー: ファイル変更の実行権を取得できません。".to_string(),
        };
        let snapshot = match super::workspace_files::snapshot_before_write(&target) {
            Ok(snapshot) => snapshot,
            Err(error) => return format!("エラー: {error}"),
        };
        if let Err(error) = permissions::fs::write(&target, content.as_bytes()) {
            return format!(
                "エラー: 書き込みに失敗しました（{error}）: {}",
                target.display()
            );
        }
        match super::workspace_files::remember_change(snapshot) {
            Ok(()) => format!(
                "OK: {} を {} 箇所編集しました。workspace_files の undo で取り消せます。{}",
                target.display(),
                applied.len(),
                summary(&edits, &applied)
            ),
            Err(error) => format!(
                "エラー: 編集は完了しましたが、取り消し履歴を保存できませんでした（{error}）: {}",
                target.display()
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(EditFileTool) } }

#[cfg(test)]
mod tests {
    use super::{apply, parse_edits, Edit};
    use serde_json::json;

    fn edit(old: &str, new: &str, replace_all: bool) -> Edit {
        Edit {
            old: old.to_string(),
            new: new.to_string(),
            replace_all,
        }
    }

    #[test]
    fn 一意な一致だけを置換する() {
        let (out, result) = apply("a\nb\nc\n", &edit("b", "B", false), 1).unwrap();
        assert_eq!(out, "a\nB\nc\n");
        assert_eq!(result.line, 2);
        assert_eq!(result.hits, 1);
    }

    /// 複数一致を黙って先頭だけ置換すると、モデルが意図していない箇所が残る。
    #[test]
    fn 複数一致はreplace_allなしでは拒否する() {
        let error = apply("x\nx\n", &edit("x", "y", false), 1).unwrap_err();
        assert!(error.contains("2 箇所"), "error: {error}");
        let (out, result) = apply("x\nx\n", &edit("x", "y", true), 1).unwrap();
        assert_eq!(out, "y\ny\n");
        assert_eq!(result.hits, 2);
    }

    #[test]
    fn 見つからない置換はエラーにする() {
        let error = apply("abc", &edit("zzz", "y", false), 1).unwrap_err();
        assert!(error.contains("見つかりません"), "error: {error}");
    }

    /// 日本語（マルチバイト）の一致でも行番号計算が panic しないこと。
    #[test]
    fn 日本語の置換でも行番号を返す() {
        let (out, result) = apply("あいう\nかきく\n", &edit("かきく", "さしす", false), 1).unwrap();
        assert_eq!(out, "あいう\nさしす\n");
        assert_eq!(result.line, 2);
    }

    #[test]
    fn edits配列と単発指定の両方を受け付ける() {
        let single = parse_edits(&json!({ "old_string": "a", "new_string": "b" })).unwrap();
        assert_eq!(single.len(), 1);
        let multi = parse_edits(&json!({
            "edits": [
                { "old_string": "a", "new_string": "b" },
                { "old_string": "c", "new_string": "d", "replace_all": true }
            ]
        }))
        .unwrap();
        assert_eq!(multi.len(), 2);
        assert!(multi[1].replace_all);
        assert!(parse_edits(&json!({ "old_string": "a", "new_string": "a" })).is_err());
    }
}
