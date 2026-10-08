// migration.rs - 識別子別フォルダからPicoAgent共通管理領域へのローカルデータ移行。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

const LEGACY_IDENTIFIERS: [&str; 2] = ["com.picoagent.app", "com.coda.app"];

pub fn migrate_identity_dirs(app: &AppHandle, current_config_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(current_config_dir)?;
    // 開発ビルドは空の専用領域から始め、通常版の旧識別子フォルダやWebView領域を移さない。
    if crate::app_paths::dev_data_root().is_some() {
        return Ok(());
    }
    let mut sources = BTreeSet::new();
    for current in [app.path().app_config_dir()?, app.path().app_data_dir()?] {
        if current != current_config_dir {
            sources.insert(current.clone());
        }
        if app.config().identifier == "com.picoagent.desktop" {
            sources.extend(legacy_sibling_dirs(&current));
        }
    }
    for source in sources {
        if source.is_dir() && source != current_config_dir {
            migrate_directory_contents(&source, current_config_dir, &source, current_config_dir)?;
            remove_empty_directories(&source)?;
        }
    }
    let webview_target = crate::app_paths::path_at(
        current_config_dir,
        crate::app_paths::DataPath::Webview("main"),
    );
    let current_local = app.path().app_local_data_dir()?;
    let mut local_sources = BTreeSet::from([current_local.clone()]);
    if app.config().identifier == "com.picoagent.desktop" {
        local_sources.extend(legacy_sibling_dirs(&current_local));
    }
    for source in local_sources {
        if source.is_dir() && source != current_config_dir && source != webview_target {
            std::fs::create_dir_all(&webview_target)?;
            migrate_directory_contents(&source, &webview_target, &source, current_config_dir)?;
            remove_empty_directories(&source)?;
        }
    }
    Ok(())
}

fn migrate_directory_contents(
    source: &Path,
    target: &Path,
    source_root: &Path,
    managed_root: &Path,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            std::fs::create_dir_all(&target_path)?;
            migrate_directory_contents(&source_path, &target_path, source_root, managed_root)?;
            continue;
        }
        if !target_path.exists() {
            move_file(&source_path, &target_path)?;
            continue;
        }
        if std::fs::read(&source_path)? == std::fs::read(&target_path)? {
            std::fs::remove_file(&source_path)?;
            continue;
        }
        let relative = source_path.strip_prefix(source_root)?;
        let source_name = source_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("legacy");
        let conflict = unique_conflict_path(
            &managed_root
                .join("migration-conflicts")
                .join(source_name)
                .join(relative),
        );
        move_file(&source_path, &conflict)?;
    }
    Ok(())
}

fn move_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(source, target)?;
            std::fs::remove_file(source)?;
            Ok(())
        }
    }
}

fn unique_conflict_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    for index in 1..=9999 {
        let candidate = path.with_extension(format!("migration-{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.with_extension(format!("migration-{}", std::process::id()))
}

fn remove_empty_directories(path: &Path) -> anyhow::Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.path().is_dir() {
            remove_empty_directories(&entry.path())?;
        }
    }
    if std::fs::read_dir(path)?.next().is_none() {
        std::fs::remove_dir(path)?;
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn 識別子別データを管理領域へ移して競合も失わない() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("com.picoagent.desktop");
        let managed = temp.path().join("PicoAgent");
        std::fs::create_dir_all(source.join("characters/mugi")).unwrap();
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(source.join("characters/mugi/model.json"), "model").unwrap();
        std::fs::write(source.join("config.json"), "old").unwrap();
        std::fs::write(managed.join("config.json"), "new").unwrap();

        super::migrate_directory_contents(&source, &managed, &source, &managed).unwrap();
        super::remove_empty_directories(&source).unwrap();

        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(managed.join("characters/mugi/model.json")).unwrap(),
            "model"
        );
        assert_eq!(
            std::fs::read_to_string(managed.join("config.json")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(
                managed.join("migration-conflicts/com.picoagent.desktop/config.json")
            )
            .unwrap(),
            "old"
        );
    }
}
