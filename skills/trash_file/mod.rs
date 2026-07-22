// skills/trash_file/mod.rs - 許可ルート内のファイル/フォルダをOSのゴミ箱へ移動するスキル。
//
// 物理削除は行わず、既存パスだけを対象にする。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions;

pub struct TrashFileTool;

/// 対象（既存パス）を正規化・canonicalize し、共通の許可ルート内かつ保護パスでないことを検証する。
/// 許可ルートそのもの・存在しない対象・保護パスは拒否する。
fn bounded_existing_path(cfg: &Config, path: &str) -> Result<PathBuf, String> {
    let primary = permissions::primary_root(cfg);
    let target = permissions::normalize(&primary, path)
        .canonicalize()
        .map_err(|error| format!("対象が存在しないか確認できません（{}）: {}", error, path))?;
    if permissions::is_protected(cfg, &target) {
        return Err(format!("保護された場所は操作できません: {}", target.display()));
    }
    for root in permissions::allowed_roots(cfg) {
        if let Ok(root) = root.canonicalize() {
            if target == root {
                return Err("許可ルートそのものはゴミ箱へ移動できません。".to_string());
            }
            if target.starts_with(&root) {
                return Ok(target);
            }
        }
    }
    Err(format!(
        "許可ルートの外は操作できません（設定の「許可」タブで許可ルートを追加してください）: {}",
        target.display()
    ))
}

#[cfg(windows)]
pub(super) fn move_to_trash(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
        SHFILEOPSTRUCTW,
    };

    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    let result = unsafe { SHFileOperationW(&mut op) };
    if result == 0 && op.fAnyOperationsAborted == 0 {
        Ok(())
    } else if op.fAnyOperationsAborted != 0 {
        Err("ゴミ箱への移動が中断されました。".to_string())
    } else {
        Err(format!("ゴミ箱への移動に失敗しました。Shell error={}", result))
    }
}

#[cfg(target_os = "macos")]
pub(super) fn move_to_trash(path: &Path) -> Result<(), String> {
    let trash_dir = dirs::home_dir()
        .ok_or_else(|| "ホームフォルダを確認できません。".to_string())?
        .join(".Trash");
    std::fs::create_dir_all(&trash_dir)
        .map_err(|error| format!("ゴミ箱フォルダを作成できません: {}", error))?;
    std::fs::rename(path, unique_trash_path(&trash_dir, path))
        .map_err(|error| format!("ゴミ箱への移動に失敗しました: {}", error))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn move_to_trash(path: &Path) -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "ホームフォルダを確認できません。".to_string())?;
    let trash_files = home.join(".local/share/Trash/files");
    let trash_info = home.join(".local/share/Trash/info");
    std::fs::create_dir_all(&trash_files)
        .map_err(|error| format!("ゴミ箱フォルダを作成できません: {}", error))?;
    std::fs::create_dir_all(&trash_info)
        .map_err(|error| format!("ゴミ箱情報フォルダを作成できません: {}", error))?;
    let dest = unique_trash_path(&trash_files, path);
    let info_path = trash_info.join(format!(
        "{}.trashinfo",
        dest.file_name().and_then(|name| name.to_str()).unwrap_or("trashed")
    ));
    std::fs::rename(path, &dest).map_err(|error| format!("ゴミ箱への移動に失敗しました: {}", error))?;
    let deletion_date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    let info = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        path.to_string_lossy(),
        deletion_date
    );
    std::fs::write(&info_path, info)
        .map_err(|error| format!("ゴミ箱情報の作成に失敗しました: {}", error))?;
    Ok(())
}

#[cfg(not(any(windows, unix)))]
pub(super) fn move_to_trash(_path: &Path) -> Result<(), String> {
    Err("このOSではゴミ箱移動に未対応です。".to_string())
}

#[cfg(unix)]
fn unique_trash_path(trash_dir: &Path, path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut candidate = trash_dir.join(name.as_ref());
    if !candidate.exists() {
        return candidate;
    }
    for index in 1..10_000 {
        candidate = trash_dir.join(format!("{} ({})", name, index));
        if !candidate.exists() {
            return candidate;
        }
    }
    trash_dir.join(format!("{} ({})", name, chrono::Local::now().timestamp()))
}

#[async_trait]
impl Tool for TrashFileTool {
    fn name(&self) -> &'static str {
        "trash_file"
    }

    fn label(&self) -> &'static str {
        "ファイルをゴミ箱へ移動"
    }

    fn description(&self) -> String {
        "許可ルート内の既存ファイルまたはフォルダをOSのゴミ箱へ移動する。物理削除は行わず、実行には confirm=true が必要。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "ゴミ箱へ移動するファイルまたはフォルダ。許可ルート配下の絶対パスまたは相対パス。"
                },
                "confirm": {
                    "type": "boolean",
                    "description": "true のときだけ実行する。false または省略時は preview のみ。"
                }
            },
            "required": ["path"]
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let path = match args.get("path").and_then(Value::as_str).map(str::trim) {
            Some(path) if !path.is_empty() => path,
            _ => return "エラー: path が指定されていません。".to_string(),
        };
        let target = match bounded_existing_path(cfg, path) {
            Ok(target) => target,
            Err(error) => return format!("エラー: {}", error),
        };
        let kind = match std::fs::metadata(&target) {
            Ok(meta) if meta.is_dir() => "フォルダ",
            Ok(meta) if meta.is_file() => "ファイル",
            Ok(_) => "項目",
            Err(error) => return format!("エラー: 対象情報を取得できません（{}）: {}", error, target.display()),
        };
        if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
            return format!(
                "ゴミ箱移動 preview: {} をゴミ箱へ移動できます。実行するには confirm=true を指定してください: {}",
                kind,
                target.display()
            );
        }
        match move_to_trash(&target) {
            Ok(()) => format!("OK: {}をゴミ箱へ移動しました: {}", kind, target.display()),
            Err(error) => format!("エラー: {}", error),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(TrashFileTool) } }
