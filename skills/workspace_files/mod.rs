// skills/workspace_files/mod.rs - 許可ルート内のワークスペースファイル操作スキル。
//
// 読み書きは全スキル共通の許可ルール（permissions）に限定し、サイズ上限を設ける。
// 許可ルート外・保護パス（C:\Windows / Program Files 等）は permissions が拒否する。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions::SafePath;
use crate::permissions;

const MAX_READ_BYTES: usize = 128 * 1024;
const MAX_WRITE_BYTES: usize = 128 * 1024;
const MAX_LIST_ENTRIES: usize = 200;
const MAX_BACKUP_BYTES: u64 = 1024 * 1024;

struct UndoEntry {
    path: PathBuf,
    original: Option<Vec<u8>>,
}

static UNDO_HISTORY: OnceLock<Mutex<Vec<UndoEntry>>> = OnceLock::new();

fn undo_history() -> &'static Mutex<Vec<UndoEntry>> {
    UNDO_HISTORY.get_or_init(|| Mutex::new(Vec::new()))
}

fn snapshot_before_write(path: &SafePath) -> Result<UndoEntry, String> {
    let original = if path.exists() {
        let size = std::fs::metadata(path)
            .map_err(|error| format!("変更前の情報を取得できません: {error}"))?
            .len();
        if size > MAX_BACKUP_BYTES {
            return Err(format!(
                "安全に取り消せる上限（{}バイト）を超えるため変更しません: {}",
                MAX_BACKUP_BYTES,
                path.display()
            ));
        }
        Some(std::fs::read(path).map_err(|error| format!("変更前データを保存できません: {error}"))?)
    } else {
        None
    };
    Ok(UndoEntry {
        path: path.as_path().to_path_buf(),
        original,
    })
}

fn remember_change(entry: UndoEntry) -> Result<(), String> {
    let mut history = undo_history()
        .lock()
        .map_err(|_| "取り消し履歴を取得できません。".to_string())?;
    history.push(entry);
    if history.len() > 20 {
        history.remove(0);
    }
    Ok(())
}

fn undo_last(confirm: bool) -> String {
    let mut history = match undo_history().lock() {
        Ok(history) => history,
        Err(_) => return "エラー: 取り消し履歴を取得できません。".to_string(),
    };
    let Some(entry) = history.last() else {
        return "取り消せるファイル変更はありません。".to_string();
    };
    if !confirm {
        return format!(
            "プレビュー: 直前の変更を取り消します: {}\n実行するには confirm=true を指定してください。",
            entry.path.display()
        );
    }
    let path_label = entry.path.display().to_string();
    let result = match &entry.original {
        Some(bytes) => std::fs::write(&entry.path, bytes).map_err(|error| error.to_string()),
        None if entry.path.exists() => super::trash_file::move_to_trash(&entry.path),
        None => Ok(()),
    };
    match result {
        Ok(()) => {
            history.pop();
            format!("OK: 直前の変更を取り消しました: {path_label}")
        }
        Err(error) => format!("エラー: 変更を取り消せませんでした（{error}）: {path_label}"),
    }
}

pub struct WorkspaceFilesTool;

fn trim_to_bytes(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let mut end = max_bytes;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…（{} バイトで打ち切り）", &content[..end], max_bytes)
}

fn list_dir(path: &SafePath) -> String {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => return format!("エラー: 一覧取得に失敗しました（{}）: {}", error, path.display()),
    };
    let mut rows = Vec::new();
    for entry in entries.take(MAX_LIST_ENTRIES + 1) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                rows.push(format!("エラー項目: {}", error));
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => "dir",
            Ok(file_type) if file_type.is_file() => "file",
            Ok(_) => "other",
            Err(_) => "unknown",
        };
        let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
        rows.push(format!(
            "{}\t{}\t{}",
            file_type,
            size,
            entry.file_name().to_string_lossy()
        ));
        if rows.len() >= MAX_LIST_ENTRIES {
            rows.push(format!("…（{} 件で打ち切り）", MAX_LIST_ENTRIES));
            break;
        }
    }
    if rows.is_empty() {
        "空のフォルダです。".to_string()
    } else {
        rows.join("\n")
    }
}

fn content_arg(args: &Value) -> Result<&str, String> {
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "content が指定されていません。".to_string())?;
    if content.len() > MAX_WRITE_BYTES {
        return Err(format!("content は {} バイト以内で指定してください。", MAX_WRITE_BYTES));
    }
    Ok(content)
}

#[async_trait]
impl Tool for WorkspaceFilesTool {
    fn name(&self) -> &'static str {
        "workspace_files"
    }

    fn label(&self) -> &'static str {
        "ワークスペースファイル操作"
    }

    fn description(&self) -> String {
        "許可ルート内のファイル一覧、テキスト読込、書込、追記、フォルダ作成を行う。コードやドキュメントの実作業が必要なときに使う。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "read", "write", "append", "mkdir", "stat", "undo"],
                    "description": "実行する操作。write は既存ファイルを上書きする。"
                },
                "path": {
                    "type": "string",
                    "description": "操作対象パス。許可ルート配下の絶対パスまたは相対パス。"
                },
                "content": {
                    "type": "string",
                    "description": "write / append で書き込むテキスト。"
                },
                "create_parent": {
                    "type": "boolean",
                    "description": "write / append 時に親フォルダが無ければ作成する。"
                },
                "preview": { "type": "boolean", "description": "write / append の変更内容だけを確認し、書き込まない。" },
                "confirm": { "type": "boolean", "description": "undo の実行確認。省略時は取り消し対象のプレビューだけを返す。" }
            },
            "required": ["action"]
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = match args.get("action").and_then(Value::as_str) {
            Some(action) => action,
            None => return "エラー: action が指定されていません。".to_string(),
        };
        if action == "undo" {
            return undo_last(args.get("confirm").and_then(Value::as_bool).unwrap_or(false));
        }
        let path = match args.get("path").and_then(Value::as_str) {
            Some(path) if !path.trim().is_empty() => path.trim(),
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        // 許可ルート内かつ保護パスでないことを共通ルールで検証する。
        let target = match permissions::resolve(cfg, path) {
            Ok(target) => target,
            Err(error) => return format!("エラー: {}", error),
        };

        match action {
            "list" => list_dir(&target),
            "read" => match std::fs::read_to_string(&target) {
                Ok(content) => trim_to_bytes(&content, MAX_READ_BYTES),
                Err(error) => format!("エラー: 読み取りに失敗しました（{}）: {}", error, target.display()),
            },
            "write" | "append" => {
                let content = match content_arg(&args) {
                    Ok(content) => content,
                    Err(error) => return format!("エラー: {}", error),
                };
                if args.get("preview").and_then(Value::as_bool).unwrap_or(false) {
                    return format!(
                        "プレビュー: {} へ {} バイトを{}します。\n先頭: {}",
                        target.display(),
                        content.len(),
                        if action == "write" { "書き込み" } else { "追記" },
                        content.chars().take(300).collect::<String>()
                    );
                }
                if args
                    .get("create_parent")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    let parent = target.parent_or_root(cfg);
                    if let Err(error) = crate::permissions::fs::create_dir_all(&parent) {
                        return format!("エラー: 親フォルダ作成に失敗しました（{}）: {}", error, parent.display());
                    }
                }
                let snapshot = match snapshot_before_write(&target) {
                    Ok(snapshot) => snapshot,
                    Err(error) => return format!("エラー: {error}"),
                };
                let result = if action == "write" {
                    std::fs::write(&target, content)
                } else {
                    use std::io::Write;
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&target)
                        .and_then(|mut file| file.write_all(content.as_bytes()))
                };
                match result {
                    Ok(()) => match remember_change(snapshot) {
                        Ok(()) => format!("OK: {} へ {} バイトを書き込みました。直前の変更は undo で取り消せます。", target.display(), content.len()),
                        Err(error) => format!("エラー: 書き込みは完了しましたが、取り消し履歴を保存できませんでした（{error}）: {}", target.display()),
                    },
                    Err(error) => format!("エラー: 書き込みに失敗しました（{}）: {}", error, target.display()),
                }
            }
            "mkdir" => {
                match std::fs::create_dir_all(&target) {
                    Ok(()) => format!("OK: フォルダを作成しました: {}", target.display()),
                    Err(error) => format!("エラー: フォルダ作成に失敗しました（{}）: {}", error, target.display()),
                }
            }
            "stat" => match std::fs::metadata(&target) {
                Ok(meta) => format!(
                    "種類: {}\nサイズ: {} バイト\n読み取り専用: {}",
                    if meta.is_dir() { "dir" } else if meta.is_file() { "file" } else { "other" },
                    meta.len(),
                    meta.permissions().readonly()
                ),
                Err(error) => format!("エラー: 情報取得に失敗しました（{}）: {}", error, target.display()),
            },
            _ => "エラー: action は list/read/write/append/mkdir/stat/undo のいずれかを指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(WorkspaceFilesTool) } }
