// skills/workspace_files/mod.rs - 許可ルート内のワークスペースファイル操作スキル。
//
// 読み書きは PICOAGENT_WORKSPACE_ROOT 配下に限定し、サイズ上限を設ける。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_READ_BYTES: usize = 128 * 1024;
const MAX_WRITE_BYTES: usize = 128 * 1024;
const MAX_LIST_ENTRIES: usize = 200;

pub struct WorkspaceFilesTool;

fn home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn workspace_root(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.get("PICOAGENT_WORKSPACE_ROOT", &home()))
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

#[cfg(windows)]
fn is_protected_edit_path(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    s.starts_with(r"c:\windows")
        || s.starts_with(r"c:\program files\")
        || s.starts_with(r"c:\program files (x86)\")
}

#[cfg(not(windows))]
fn is_protected_edit_path(_path: &Path) -> bool {
    false
}

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

fn list_dir(path: &Path) -> String {
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
                    "enum": ["list", "read", "write", "append", "mkdir", "stat"],
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
                }
            },
            "required": ["action", "path"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_WORKSPACE_ROOT",
            "workspace_files の許可ルート",
            "text",
            &home(),
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = match args.get("action").and_then(Value::as_str) {
            Some(action) => action,
            None => return "エラー: action が指定されていません。".to_string(),
        };
        let path = match args.get("path").and_then(Value::as_str) {
            Some(path) if !path.trim().is_empty() => path.trim(),
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        let base = workspace_root(cfg);
        let target = match bounded_path(&base, path) {
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
                if is_protected_edit_path(&target) {
                    return format!(
                        "エラー: Windows / Program Files 配下は編集不可です: {}",
                        target.display()
                    );
                }
                let content = match content_arg(&args) {
                    Ok(content) => content,
                    Err(error) => return format!("エラー: {}", error),
                };
                if args
                    .get("create_parent")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    if let Some(parent) = target.parent() {
                        if let Err(error) = std::fs::create_dir_all(parent) {
                            return format!("エラー: 親フォルダ作成に失敗しました（{}）: {}", error, parent.display());
                        }
                    }
                }
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
                    Ok(()) => format!("OK: {} へ {} バイトを書き込みました。", target.display(), content.len()),
                    Err(error) => format!("エラー: 書き込みに失敗しました（{}）: {}", error, target.display()),
                }
            }
            "mkdir" => {
                if is_protected_edit_path(&target) {
                    return format!(
                        "エラー: Windows / Program Files 配下は編集不可です: {}",
                        target.display()
                    );
                }
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
            _ => "エラー: action は list/read/write/append/mkdir/stat のいずれかを指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(WorkspaceFilesTool) } }

#[cfg(test)]
mod tests {
    use super::{bounded_path, normalize};
    use std::path::PathBuf;

    #[test]
    fn 相対パスの親参照を正規化する() {
        let base = PathBuf::from(r"C:\work\root");
        assert_eq!(normalize(&base, r"a\..\b"), PathBuf::from(r"C:\work\root\b"));
    }

    #[test]
    fn 許可ルート外を拒否する() {
        let base = PathBuf::from(r"C:\work\root");
        assert!(bounded_path(&base, r"..\secret.txt").is_err());
    }
}
