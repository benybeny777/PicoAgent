// startup_cleanup.rs - 起動時にPicoAgent管理下の再生成可能なキャッシュだけを回収する。

use std::path::{Path, PathBuf};

use tauri::AppHandle;

/// 設定、会話、完成キャラ、導入済みモデルを避けた起動時削除対象。
/// WebViewのプロファイル全体を消すとlocalStorageまで失われるため、キャッシュ名を列挙する。
fn chromium_cache_targets(root: &Path) -> Vec<PathBuf> {
    let default = root.join("Default");
    vec![
        root.join("component_crx_cache"),
        root.join("extensions_crx_cache"),
        default.join("AutofillAiModelCache"),
        default.join("Cache"),
        default.join("Code Cache"),
        default.join("optimization_guide_hint_cache_store"),
        default.join("Service Worker/CacheStorage"),
        default.join("Shared Dictionary/cache"),
    ]
}

/// GPU・シェーダのキャッシュ。起動のたびに消すと、次の起動でGPUがシェーダを作り直し、
/// Live2D・MMD・メッシュの初回描画が毎回もたつく（容量は合わせて十数MB）。
/// ふだんは残し、描画（WebGL）の初期化失敗や描画面の消失が起きたときだけ、画面から予約を受けて消す。
fn chromium_gpu_cache_targets(root: &Path) -> Vec<PathBuf> {
    let default = root.join("Default");
    vec![
        root.join("GPUPersistentCache"),
        root.join("GrShaderCache"),
        root.join("ShaderCache"),
        default.join("DawnGraphiteCache"),
        default.join("DawnWebGPUCache"),
        default.join("GPUCache"),
    ]
}

/// 次回起動時にGPU・シェーダのキャッシュを消す予約印（データ領域の直下に置く）。
const GPU_CACHE_RESET_MARKER: &str = "gpu-cache-reset-requested";

/// GPU描画のエラーから回復するため、次回起動時にGPU・シェーダのキャッシュを消すよう予約する。
/// 起動中のWebViewが使用中のファイルは消せないため、その場では消さない。
pub fn request_gpu_cache_reset(app: &AppHandle) -> std::io::Result<()> {
    let root = crate::log_paths::data_dir(app);
    crate::store::write_atomic(&root.join(GPU_CACHE_RESET_MARKER), b"requested\n")
        .map_err(std::io::Error::other)
}

fn profile_roots(data_root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![data_root.join("browser-profile")];
    if let Ok(entries) = std::fs::read_dir(data_root.join("webview")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                roots.push(entry.path().join("EBWebView"));
            }
        }
    }
    roots
}

fn gpu_cache_targets(data_root: &Path) -> Vec<PathBuf> {
    profile_roots(data_root)
        .iter()
        .flat_map(|root| chromium_gpu_cache_targets(root))
        .collect()
}

fn cache_targets(data_root: &Path) -> Vec<PathBuf> {
    let cache_root = data_root.join("cache");
    let mut targets = vec![
        cache_root.join("character-maker-installer"),
        cache_root.join("media-model-installer"),
        cache_root.join("component-installer"),
    ];
    for root in profile_roots(data_root) {
        targets.extend(chromium_cache_targets(&root));
    }
    targets
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

/// 削除操作の結果で完了を判定する。存在確認だけではアクセス拒否も「存在しない」になる。
fn reset_gpu_cache(
    marker: &Path,
    targets: &[PathBuf],
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
    mut log: impl FnMut(&str),
) {
    let mut complete = true;
    for target in targets {
        if let Err(error) = remove(target) {
            complete = false;
            log(&format!(
                "描画キャッシュを削除できません: {} ({error})",
                target.display()
            ));
        }
    }
    if !complete {
        log("描画キャッシュの削除は未完了です。予約を残して次回起動時に再試行します");
        return;
    }
    if let Err(error) = remove(marker) {
        log(&format!(
            "描画キャッシュ削除の予約を解除できません: {error}"
        ));
        return;
    }
    log("予約どおり描画（GPU・シェーダ）キャッシュを削除しました");
}

pub fn run(app: &AppHandle) {
    let root = crate::log_paths::data_dir(app);
    let marker = root.join(GPU_CACHE_RESET_MARKER);
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
    if marker.exists() {
        reset_gpu_cache(
            &marker,
            &gpu_cache_targets(&root),
            remove_target,
            |message| crate::log_paths::append_app_log(app, message),
        );
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
    fn gpu削除失敗は予約を残し再試行成功後だけ完了を記録する() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join(GPU_CACHE_RESET_MARKER);
        let cache = temp.path().join("GPUCache");
        std::fs::write(&marker, b"requested").unwrap();
        std::fs::create_dir(&cache).unwrap();
        let targets = vec![cache.clone()];
        let mut logs = Vec::new();
        reset_gpu_cache(
            &marker,
            &targets,
            |path| {
                if path == cache {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "検証用のアクセス拒否",
                    ))
                } else {
                    remove_target(path)
                }
            },
            |message| logs.push(message.to_string()),
        );
        assert!(marker.exists());
        assert!(cache.exists());
        assert!(logs.iter().any(|message| message.contains("未完了")));
        assert!(!logs.iter().any(|message| message.contains("削除しました")));

        logs.clear();
        reset_gpu_cache(&marker, &targets, remove_target, |message| {
            logs.push(message.to_string())
        });
        assert!(!marker.exists());
        assert!(!cache.exists());
        assert!(logs.iter().any(|message| message.contains("削除しました")));
    }

    #[test]
    fn gpuとシェーダのキャッシュは毎回の起動では消さない() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("webview/main")).unwrap();
        let targets = cache_targets(root);
        for kept in [
            "webview/main/EBWebView/GrShaderCache",
            "webview/main/EBWebView/Default/GPUCache",
            "browser-profile/ShaderCache",
        ] {
            assert!(!targets.contains(&root.join(kept)), "{kept}");
        }
        let gpu = gpu_cache_targets(root);
        assert!(gpu.contains(&root.join("webview/main/EBWebView/GrShaderCache")));
        assert!(gpu.contains(&root.join("browser-profile/Default/GPUCache")));
    }

    #[test]
    fn 設定と履歴とキャラと導入済みモデルは消さない() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("webview/main")).unwrap();
        let targets = cache_targets(root);
        assert!(targets.contains(&root.join("cache/character-maker-installer")));
        assert!(targets.contains(&root.join("cache/media-model-installer")));
        assert!(targets.contains(&root.join("browser-profile/Default/Cache")));
        assert!(targets.contains(&root.join("webview/main/EBWebView/Default/Cache")));
        for preserved in [
            "config.json",
            "history.json",
            "characters",
            "bundled/character-maker",
            "webview/main/EBWebView/Default/Local Storage",
            "webview/main/EBWebView/Default/IndexedDB",
        ] {
            assert!(!targets.contains(&root.join(preserved)), "{preserved}");
        }
    }

    #[test]
    fn 名指ししたフォルダだけを消す() {
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

    #[test]
    fn webviewを作る前にキャッシュを回収する() {
        let source = include_str!("lib.rs");
        let cleanup = source
            .find("startup_cleanup::run(app.handle())")
            .expect("起動時キャッシュ回収がある");
        let webview = source
            .find("WebviewWindowBuilder::from_config")
            .expect("メインWebViewを作成する");
        assert!(
            cleanup < webview,
            "WebView作成後はキャッシュがロックされます"
        );
    }
}
