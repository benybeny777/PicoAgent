// startup_cleanup.rs - 起動時にPicoAgent管理下の再生成可能なキャッシュだけを回収する。

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// 設定、会話、完成キャラ、導入済みモデルを避けた起動時削除対象。
/// WebViewのプロファイル全体を消すとlocalStorageまで失われるため、キャッシュ名を列挙する。
fn cache_targets(cache_root: &Path) -> Vec<PathBuf> {
    let webview = cache_root.join("EBWebView");
    let default = webview.join("Default");
    vec![
        cache_root.join("character-maker-installer"),
        cache_root.join("media-model-installer"),
        cache_root.join("component-installer"),
        webview.join("component_crx_cache"),
        webview.join("extensions_crx_cache"),
        webview.join("GPUPersistentCache"),
        webview.join("GrShaderCache"),
        webview.join("ShaderCache"),
        default.join("AutofillAiModelCache"),
        default.join("Cache"),
        default.join("Code Cache"),
        default.join("DawnGraphiteCache"),
        default.join("DawnWebGPUCache"),
        default.join("GPUCache"),
        default.join("optimization_guide_hint_cache_store"),
        default.join("Service Worker/CacheStorage"),
        default.join("Shared Dictionary/cache"),
    ]
}

fn remove_target(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::remove_file(path),
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn run(app: &AppHandle) {
    let Ok(root) = app.path().app_cache_dir() else {
        crate::log_paths::append_app_log(app, "起動時キャッシュの場所を取得できませんでした。");
        return;
    };
    for target in cache_targets(&root) {
        if let Err(error) = remove_target(&target) {
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "起動時キャッシュを削除できません。次回起動時に再試行します: {} ({error})",
                    target.display()
                ),
            );
        }
    }
    let generated = crate::log_paths::data_dir(app).join("generated-media");
    if let Ok(entries) = std::fs::read_dir(&generated) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if (name.starts_with(".generating-") || name.ends_with(".incoming"))
                && remove_target(&entry.path()).is_err()
            {
                crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "生成途中ファイルを削除できません。次回再試行します: {}",
                        entry.path().display()
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_history_characters_and_installed_models_are_not_targets() {
        let root = Path::new("cache-root");
        let targets = cache_targets(root);
        assert!(targets.contains(&root.join("character-maker-installer")));
        assert!(targets.contains(&root.join("media-model-installer")));
        assert!(targets.contains(&root.join("EBWebView/Default/Cache")));
        for preserved in [
            "config.json",
            "history.json",
            "characters",
            "bundled/character-maker",
            "EBWebView/Default/Local Storage",
            "EBWebView/Default/IndexedDB",
        ] {
            assert!(!targets.contains(&root.join(preserved)), "{preserved}");
        }
    }

    #[test]
    fn removes_only_the_named_directory() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("Cache");
        let local_storage = temp.path().join("Local Storage");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::create_dir_all(&local_storage).unwrap();
        std::fs::write(cache.join("stale"), b"old").unwrap();
        std::fs::write(local_storage.join("user-data"), b"keep").unwrap();
        remove_target(&cache).unwrap();
        assert!(!cache.exists());
        assert!(local_storage.join("user-data").is_file());
    }
}
