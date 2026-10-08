// app_paths.rs - 共通の配置定義をOS・Tauriの実行環境へ接続する。

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

use picoagent_core::paths::{
    development_root as dev_data_root_for, managed_dir, packaged_bundled_at,
    packaged_character_maker_at, product_root,
};
pub use picoagent_core::paths::{path_at, DataPath};

pub fn data_path(app: &AppHandle, entry: DataPath<'_>) -> PathBuf {
    path_at(&data_dir(app), entry)
}

/// 開発ビルドの保存先の根（Windowsは `%APPDATA%\PicoAgent-Dev`）。製品ビルドでは `None`。
///
/// 開発中の設定・履歴・キャラ・記憶・モデル・同梱ランタイムを、インストーラで入れた通常版の
/// `PicoAgent` と完全に分ける。起動経路（起動バッチ・`cargo xtask dev`・`cargo tauri dev`）ではなく
/// ビルドの種類で決めるので、どの起動でも通常版の領域へは触れない。リポジトリの外に置き、
/// worktree や clone を作り直しても取得済みのモデル・音声を共有する。
pub fn dev_data_root() -> Option<PathBuf> {
    // フォールバック許可: OSの保存先を取れない開発環境でも、通常版の領域へは降格させない。
    cfg!(debug_assertions)
        .then(|| dev_data_root_for(&dirs::data_dir().unwrap_or_else(std::env::temp_dir)))
}

/// このビルドの同梱物（`resources/bundled`）の唯一の置き場所。
///
/// 製品ビルドはインストール先の `resources/bundled/`、開発ビルドはリポジトリの
/// `src-tauri/resources/bundled/`。作業フォルダや実行ファイルの位置から推測したり、
/// 無いときに別の置き場所を探したりしない（開発中の資産を製品で使わないため）。
pub fn packaged_bundled_dir(app: &AppHandle) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Some(packaged_bundled_at(Path::new(env!("CARGO_MANIFEST_DIR"))));
    }
    app.path()
        .resource_dir()
        .ok()
        .map(|root| packaged_bundled_at(&root))
}

/// キャラ作成の同梱資産（`character-maker`）の唯一の置き場所。考え方は `packaged_bundled_dir` と同じ。
pub fn packaged_character_maker_dir(app: &AppHandle) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(packaged_character_maker_at);
    }
    app.path()
        .resource_dir()
        .ok()
        .map(|root| packaged_character_maker_at(&root))
}

pub fn data_dir(app: &AppHandle) -> PathBuf {
    let identifier = app.config().identifier.as_str();
    if let Some(root) = dev_data_root() {
        return managed_dir(root, identifier);
    }
    match app.path().app_data_dir() {
        Ok(path) => {
            let root = path.parent().map(product_root).unwrap_or(path);
            managed_dir(root, identifier)
        }
        // フォールバック許可: 保存先取得自体の失敗を記録するため、ログだけは一時領域へ退避する。
        Err(error) => {
            eprintln!("ログ保存先を取得できないため一時領域を使います: {error}");
            product_root(&std::env::temp_dir())
        }
    }
}

pub fn cache_dir(app: &AppHandle) -> PathBuf {
    data_path(app, DataPath::Cache)
}

pub fn log_dir(app: &AppHandle) -> PathBuf {
    data_path(app, DataPath::Log)
}

pub fn bundled_dir(app: &AppHandle) -> PathBuf {
    data_path(app, DataPath::Bundled)
}

/// AppHandleを引数に持たないブラウザスキルも、同じ保存先の正本を使う。
pub fn browser_profile_dir() -> Result<PathBuf, String> {
    if let Some(app) = crate::durable_jobs::app_handle() {
        return Ok(data_path(app, DataPath::BrowserProfile));
    }
    // 単体テストではTauriを起動しない。アプリのデータを触らない専用領域で実ブラウザを検査する。
    #[cfg(test)]
    {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or_else(|| "検査用保存先のルートを取得できません".to_string())?
            .join("temp")
            .join(format!("browser-control-tests-{}", std::process::id()));
        Ok(path_at(&root, DataPath::BrowserProfile))
    }
    #[cfg(not(test))]
    Err("エラー: ブラウザ用プロファイルのアプリ情報を取得できません".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 開発版の保存で通常版と別識別子の設定や履歴を変更しない() {
        let folder = tempfile::tempdir().unwrap();
        let product = folder.path().join("PicoAgent");
        let development = dev_data_root_for(folder.path());
        let trial = managed_dir(development.clone(), "com.picoagent.ui-check");
        for root in [&product, &development, &trial] {
            std::fs::create_dir_all(root).unwrap();
            let config = crate::config::Config::new();
            config.load(path_at(root, DataPath::Config)).unwrap();
            config
                .set_string("PICOAGENT_CHARACTER", "mugi".into())
                .unwrap();
            std::fs::write(path_at(root, DataPath::History("mugi")), b"[]").unwrap();
        }
        let product_config = std::fs::read(path_at(&product, DataPath::Config)).unwrap();
        let trial_config = std::fs::read(path_at(&trial, DataPath::Config)).unwrap();
        let development_config = crate::config::Config::new();
        development_config
            .load(path_at(&development, DataPath::Config))
            .unwrap();
        development_config
            .set_string("PICOAGENT_CHARACTER", "hiyori".into())
            .unwrap();
        std::fs::write(path_at(&development, DataPath::History("mugi")), b"[1]").unwrap();
        for (root, original) in [(&product, product_config), (&trial, trial_config)] {
            assert_eq!(
                std::fs::read(path_at(root, DataPath::Config)).unwrap(),
                original
            );
            assert_eq!(
                std::fs::read(path_at(root, DataPath::History("mugi"))).unwrap(),
                b"[]"
            );
            for entry in [
                DataPath::BrowserProfile,
                DataPath::Webview("main"),
                DataPath::Models,
                DataPath::Characters,
            ] {
                assert_ne!(path_at(root, entry), path_at(&development, entry));
            }
        }
        let reloaded = crate::config::Config::new();
        reloaded
            .load(path_at(&development, DataPath::Config))
            .unwrap();
        assert_eq!(reloaded.get("PICOAGENT_CHARACTER", ""), "hiyori");
    }

    #[test]
    fn 各機能は管理ファイルの保存先を重複定義しない() {
        let names = [
            "config.json",
            "memory.json",
            "memory-vectors.json",
            "usage.json",
            "schedule-runs.json",
            "durable-jobs.json",
            "browser-profile",
            "office-profile",
            "generation-runtime-data",
            "webview/main",
            "webview/settings",
        ];
        let mut violations = Vec::new();
        for source in crate::guard_scan::all_sources() {
            if source.path.ends_with("app_paths.rs")
                || source.path.ends_with("picoagent-core/src/paths.rs")
            {
                continue;
            }
            for line in source
                .body()
                .lines()
                .filter(|line| !line.trim().starts_with("//"))
            {
                if names
                    .iter()
                    .any(|name| line.contains(&format!(".join(\"{name}\")")))
                {
                    violations.push(format!("{}: {line}", source.path.display()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "管理ファイルの保存先は app_paths::data_path() / path_at() を使ってください: {}",
            violations.join("\n")
        );
    }
    #[test]
    fn 開発用の保存先は通常版と別のフォルダにする() {
        let os = Path::new("/home/me/.local/share");
        let root = dev_data_root_for(os);
        assert_eq!(root, os.join("PicoAgent-Dev"));
        assert_ne!(root, os.join("PicoAgent"));
        // テストは開発ビルドなので、通常版の `PicoAgent` ではなく `PicoAgent-Dev` を指す。
        assert!(dev_data_root().unwrap().ends_with("PicoAgent-Dev"));
    }

    #[test]
    fn 製品の識別子は根を使い別の識別子はinstancesへ分ける() {
        let root = PathBuf::from("/data/PicoAgent");
        assert_eq!(managed_dir(root.clone(), "com.picoagent.desktop"), root);
        assert_eq!(
            managed_dir(root.clone(), "com.picoagent.ui-check"),
            root.join("instances").join("com.picoagent.ui-check")
        );
    }
}
