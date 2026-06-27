// migration.rs - 旧識別子から現行識別子へのローカルデータ移行。

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

const LEGACY_IDENTIFIERS: [&str; 2] = ["com.picoagent.app", "com.coda.app"];

pub fn migrate_identity_dirs(app: &AppHandle, current_config_dir: &Path) -> anyhow::Result<()> {
    migrate_file_from_siblings(current_config_dir, "config.json")?;

    let current_data_dir = app.path().app_data_dir()?;
    migrate_file_from_siblings(&current_data_dir, "history.json")?;
    migrate_file_from_siblings(&current_data_dir, "memory.json")?;
    Ok(())
}

fn migrate_file_from_siblings(current_dir: &Path, file_name: &str) -> anyhow::Result<()> {
    let target = current_dir.join(file_name);
    if target.exists() {
        return Ok(());
    }
    let Some(source) = legacy_sibling_dirs(current_dir)
        .into_iter()
        .map(|dir| dir.join(file_name))
        .find(|path| path.exists())
    else {
        return Ok(());
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, target)?;
    Ok(())
}

fn legacy_sibling_dirs(current_dir: &Path) -> Vec<PathBuf> {
    let Some(parent) = current_dir.parent() else {
        return Vec::new();
    };
    LEGACY_IDENTIFIERS
        .into_iter()
        .map(|identifier| parent.join(identifier))
        .filter(|path| path != current_dir)
        .collect()
}
