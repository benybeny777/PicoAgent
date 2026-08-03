// skills/notebook_edit/mod.rs - Jupyter ノートブック（.ipynb）のセル単位の編集スキル。
//
// .ipynb は「JSON の中に1行ずつ配列で入ったソース」という形なので、`edit_file` の
// 文字列置換ではインデントや改行のエスケープを人手で合わせることになり、壊しやすい。
// セル番号で読み・差し替え・挿入・削除だけを行い、JSON 構造には触らない。
//
// 出力（`outputs` / `execution_count`）はセルを書き換えた時点で古くなるため、
// 該当セルの分だけ消す。ノートブック全体の実行はしない。

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions::{self, SafePath};
use crate::text;

const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 128 * 1024;
const MAX_CELLS: usize = 500;
/// 一覧表示で 1 セルあたりに載せる文字数。
const PREVIEW_CHARS: usize = 200;

/// `source` は「1行ずつの配列」か「1つの文字列」のどちらでも来る。
fn cell_source(cell: &Value) -> String {
    match cell.get("source") {
        Some(Value::Array(lines)) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .concat(),
        Some(Value::String(source)) => source.clone(),
        _ => String::new(),
    }
}

/// 末尾以外の行に改行を残した配列で持たせる（nbformat の慣習に合わせる）。
fn to_source_lines(text: &str) -> Value {
    let mut lines: Vec<Value> = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find('\n') {
        lines.push(Value::String(rest[..=index].to_string()));
        rest = &rest[index + 1..];
    }
    if !rest.is_empty() {
        lines.push(Value::String(rest.to_string()));
    }
    Value::Array(lines)
}

fn new_cell(kind: &str, source: &str) -> Value {
    let mut cell = Map::new();
    cell.insert("cell_type".to_string(), Value::String(kind.to_string()));
    cell.insert("metadata".to_string(), Value::Object(Map::new()));
    cell.insert("source".to_string(), to_source_lines(source));
    if kind == "code" {
        cell.insert("execution_count".to_string(), Value::Null);
        cell.insert("outputs".to_string(), Value::Array(Vec::new()));
    }
    Value::Object(cell)
}

/// 中身を書き換えたセルの実行結果は古くなるので消す。
fn clear_outputs(cell: &mut Value) {
    let Some(object) = cell.as_object_mut() else {
        return;
    };
    if object.get("cell_type").and_then(Value::as_str) == Some("code") {
        object.insert("outputs".to_string(), Value::Array(Vec::new()));
        object.insert("execution_count".to_string(), Value::Null);
    }
}

fn list_cells(cells: &[Value]) -> String {
    if cells.is_empty() {
        return "セルがありません。".to_string();
    }
    let mut out = format!("セル {} 個", cells.len());
    for (index, cell) in cells.iter().enumerate() {
        out.push_str(&format!(
            "\n[{}] {}: {}",
            index,
            cell.get("cell_type").and_then(Value::as_str).unwrap_or("?"),
            text::preview(cell_source(cell).trim(), PREVIEW_CHARS)
        ));
    }
    out
}

fn load(target: &SafePath) -> Result<(Value, Vec<Value>), String> {
    let raw = permissions::fs::read_to_string(target)
        .map_err(|error| format!("読み取りに失敗しました（{error}）: {}", target.display()))?;
    if raw.len() > MAX_FILE_BYTES {
        return Err(format!(
            "{MAX_FILE_BYTES} バイトを超えるノートブックは編集できません: {}",
            target.display()
        ));
    }
    let notebook: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("ノートブックとして読めません（{error}）: {}", target.display()))?;
    let cells = notebook
        .get("cells")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("cells がありません: {}", target.display()))?;
    Ok((notebook, cells))
}

fn save(target: &SafePath, mut notebook: Value, cells: Vec<Value>) -> Result<(), String> {
    let Some(object) = notebook.as_object_mut() else {
        return Err("ノートブックの構造が壊れています。".to_string());
    };
    object.insert("cells".to_string(), Value::Array(cells));
    let encoded = serde_json::to_string_pretty(&notebook)
        .map_err(|error| format!("保存用に変換できません: {error}"))?;
    permissions::fs::write(target, encoded.as_bytes())
        .map_err(|error| format!("書き込みに失敗しました（{error}）: {}", target.display()))
}

pub struct NotebookEditTool;

#[async_trait]
impl Tool for NotebookEditTool {
    fn name(&self) -> &'static str {
        "notebook_edit"
    }

    fn label(&self) -> &'static str {
        "ノートブック編集"
    }

    fn description(&self) -> String {
        "Jupyter ノートブック（.ipynb）をセル単位で読み・差し替え・挿入・削除する。ノートブックの実行はしない。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "read", "replace", "insert", "delete"],
                    "description": "list はセル一覧、read は1セルの全文、replace は差し替え、insert は挿入、delete は削除。"
                },
                "path": { "type": "string", "description": "対象の .ipynb（許可ルート配下）。" },
                "cell": {
                    "type": "integer",
                    "description": "対象セル番号（0始まり）。insert では「この位置へ挿入」を意味する。"
                },
                "source": { "type": "string", "description": "replace / insert で入れるセルの中身。" },
                "cell_type": {
                    "type": "string",
                    "enum": ["code", "markdown"],
                    "description": "insert するセルの種類（既定 code）。replace で指定すると種類も変える。"
                },
                "confirm": { "type": "boolean", "description": "delete の実行確認。省略時はプレビューのみ。" }
            },
            "required": ["action", "path"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["ノートブックのセルを一覧して", "3番目のセルをこの内容に差し替えて"]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let path = match args.get("path").and_then(Value::as_str) {
            Some(path) if !path.trim().is_empty() => path.trim(),
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        // 許可ルート内かつ保護パスでないことを共通ルールで検証する。
        let target = match permissions::resolve(cfg, path) {
            Ok(target) => target,
            Err(error) => return format!("エラー: {error}"),
        };
        if target.extension_lossy().to_lowercase() != "ipynb" {
            return format!("エラー: .ipynb を指定してください: {}", target.display());
        }
        let (notebook, mut cells) = match load(&target) {
            Ok(loaded) => loaded,
            Err(error) => return format!("エラー: {error}"),
        };

        let index = args
            .get("cell")
            .and_then(Value::as_u64)
            .map(|cell| cell as usize);
        let source = args.get("source").and_then(Value::as_str).unwrap_or("");
        if matches!(action.as_str(), "replace" | "insert") {
            if source.is_empty() {
                return "エラー: source が指定されていません。".to_string();
            }
            if source.len() > MAX_SOURCE_BYTES {
                return format!("エラー: source は {MAX_SOURCE_BYTES} バイト以内で指定してください。");
            }
        }

        match action.as_str() {
            "list" => return list_cells(&cells),
            "read" => {
                let Some(index) = index else {
                    return "エラー: cell が指定されていません。".to_string();
                };
                let Some(cell) = cells.get(index) else {
                    return format!("エラー: セル {index} はありません（全 {} 個）。", cells.len());
                };
                return format!(
                    "[{index}] {}:\n{}",
                    cell.get("cell_type").and_then(Value::as_str).unwrap_or("?"),
                    cell_source(cell)
                );
            }
            "replace" => {
                let Some(index) = index else {
                    return "エラー: cell が指定されていません。".to_string();
                };
                let Some(cell) = cells.get_mut(index) else {
                    return format!("エラー: セル {index} はありません（全 {} 個）。", cells.len());
                };
                if let Some(object) = cell.as_object_mut() {
                    object.insert("source".to_string(), to_source_lines(source));
                    if let Some(kind) = args.get("cell_type").and_then(Value::as_str) {
                        object.insert("cell_type".to_string(), Value::String(kind.to_string()));
                    }
                }
                clear_outputs(cell);
            }
            "insert" => {
                if cells.len() >= MAX_CELLS {
                    return format!("エラー: セルは {MAX_CELLS} 個までです。");
                }
                let kind = args
                    .get("cell_type")
                    .and_then(Value::as_str)
                    .unwrap_or("code");
                let at = index.unwrap_or(cells.len()).min(cells.len());
                cells.insert(at, new_cell(kind, source));
            }
            "delete" => {
                let Some(index) = index else {
                    return "エラー: cell が指定されていません。".to_string();
                };
                let Some(cell) = cells.get(index) else {
                    return format!("エラー: セル {index} はありません（全 {} 個）。", cells.len());
                };
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    return format!(
                        "プレビュー: セル {index} を削除します:\n{}\n実行するには confirm=true を指定してください。",
                        text::preview(cell_source(cell).trim(), PREVIEW_CHARS)
                    );
                }
                cells.remove(index);
            }
            other => {
                return format!(
                    "エラー: action は list/read/replace/insert/delete のいずれかを指定してください: {other}"
                )
            }
        }

        // 書き込みロックと取り消し履歴は workspace_files と共有する（undo で戻せる）。
        let _mutation = match super::workspace_files::file_mutation().lock() {
            Ok(guard) => guard,
            Err(_) => return "エラー: ファイル変更の実行権を取得できません。".to_string(),
        };
        let snapshot = match super::workspace_files::snapshot_before_write(&target) {
            Ok(snapshot) => snapshot,
            Err(error) => return format!("エラー: {error}"),
        };
        let count = cells.len();
        if let Err(error) = save(&target, notebook, cells) {
            return format!("エラー: {error}");
        }
        match super::workspace_files::remember_change(snapshot) {
            Ok(()) => format!(
                "OK: {} を更新しました（セル {count} 個）。workspace_files の undo で取り消せます。",
                target.display()
            ),
            Err(error) => format!(
                "エラー: 更新は完了しましたが、取り消し履歴を保存できませんでした（{error}）: {}",
                target.display()
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(NotebookEditTool) } }

#[cfg(test)]
mod tests {
    use super::{cell_source, clear_outputs, new_cell, to_source_lines};
    use serde_json::json;

    #[test]
    fn 配列と文字列のどちらのsourceも読める() {
        assert_eq!(cell_source(&json!({ "source": ["a\n", "b"] })), "a\nb");
        assert_eq!(cell_source(&json!({ "source": "a\nb" })), "a\nb");
        assert_eq!(cell_source(&json!({})), "");
    }

    /// 改行は行末に残したまま1行1要素にする（nbformat の慣習）。
    #[test]
    fn 改行つきの行配列へ変換する() {
        assert_eq!(to_source_lines("a\nb"), json!(["a\n", "b"]));
        assert_eq!(to_source_lines("a\n"), json!(["a\n"]));
        assert_eq!(to_source_lines(""), json!([]));
        // 日本語でもバイト境界で壊れない。
        assert_eq!(to_source_lines("あ\nい"), json!(["あ\n", "い"]));
    }

    /// 書き換えたコードセルの実行結果は古くなるので消す（markdown は対象外）。
    #[test]
    fn コードセルの出力だけを消す() {
        let mut code = json!({ "cell_type": "code", "outputs": [1], "execution_count": 3 });
        clear_outputs(&mut code);
        assert_eq!(code["outputs"], json!([]));
        assert_eq!(code["execution_count"], json!(null));

        let mut markdown = json!({ "cell_type": "markdown", "outputs": [1] });
        clear_outputs(&mut markdown);
        assert_eq!(markdown["outputs"], json!([1]));
    }

    #[test]
    fn 新規セルは種類ごとの構造を持つ() {
        let code = new_cell("code", "print(1)");
        assert!(code.get("outputs").is_some());
        let markdown = new_cell("markdown", "# 見出し");
        assert!(markdown.get("outputs").is_none());
    }
}
