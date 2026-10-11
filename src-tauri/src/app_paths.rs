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

/// setupで確定した、このアプリインスタンス専用の管理保存先。
pub(crate) struct ManagedDataDir(pub PathBuf);

/// 保存先を起動時に一度だけ確定する。取得不能なら他の領域へ切り替えない。
pub(crate) fn resolve_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    resolve_data_dir_with(
        cfg!(debug_assertions),
        app.config().identifier.as_str(),
        dirs::data_dir,
        || {
            app.path()
                .app_data_dir()
                .map_err(|error| format!("製品の管理保存先を取得できません: {error}"))
        },
    )
}

fn resolve_data_dir_with(
    development: bool,
    identifier: &str,
    os_data: impl FnOnce() -> Option<PathBuf>,
    product_data: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    let root = if development {
        let os = os_data()
            .ok_or("開発版のOS保存先を取得できません。管理データを初期化せず起動を停止します")?;
        dev_data_root_for(&os)
    } else {
        let product = product_data()?;
        let parent = product
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or("製品の管理保存先の親ディレクトリを取得できません。起動を停止します")?;
        product_root(parent)
    };
    Ok(managed_dir(root, identifier))
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

/// 製品のキャラ学習処理だけを明示同梱したスクリプトの保存先。
pub fn character_learning_scripts_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        return Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|root| root.join("scripts"))
            .ok_or_else(|| "開発用の学習処理の保存先が不正です".into());
    }
    app.path()
        .resource_dir()
        .map(|root| root.join("_up_/scripts"))
        .map_err(|error| format!("同梱の学習処理の保存先を取得できません: {error}"))
}

/// setupがManagedDataDirを登録してから、コマンド・ログ・各機能の初期化を始める。
/// AppStateと同じTauriの必須state契約を使う。未初期化状態は呼び出し順の不具合であり、
/// 別領域を返して処理を続けない。複数AppHandleもそれぞれの確定保存先を参照する。
pub fn data_dir(app: &AppHandle) -> PathBuf {
    app.state::<ManagedDataDir>().inner().0.clone()
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
        let root = test_workspace_dir(TestWorkspace::BrowserControl)?;
        Ok(path_at(&root, DataPath::BrowserProfile))
    }
    #[cfg(not(test))]
    Err("エラー: ブラウザ用プロファイルのアプリ情報を取得できません".to_string())
}

/// Tauriを起動しない検査で使う一時領域。任意の利用者パスは受け付けない。
#[cfg(test)]
pub(crate) enum TestWorkspace {
    BrowserControl,
    LoraFixtures,
}

#[cfg(test)]
pub(crate) fn test_workspace_dir(workspace: TestWorkspace) -> Result<PathBuf, String> {
    let name = match workspace {
        TestWorkspace::BrowserControl => format!("browser-control-tests-{}", std::process::id()),
        TestWorkspace::LoraFixtures => "lora-fixtures".to_string(),
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("temp").join(name))
        .ok_or_else(|| "検査用保存先のルートを取得できません".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 保存先取得失敗は別領域を探索せず起動を止める() {
        use std::cell::Cell;
        let product_calls = Cell::new(0);
        let error = resolve_data_dir_with(
            true,
            "com.picoagent.desktop",
            || None,
            || {
                product_calls.set(product_calls.get() + 1);
                Ok(PathBuf::from("/temporary/product"))
            },
        )
        .unwrap_err();
        assert!(error.contains("OS保存先"));
        assert_eq!(product_calls.get(), 0);
        let os_calls = Cell::new(0);
        for product in [
            Err("OSが保存先を返せません".into()),
            Ok(PathBuf::from("/")),
            Ok(PathBuf::from("single")),
        ] {
            assert!(resolve_data_dir_with(
                false,
                "com.picoagent.desktop",
                || {
                    os_calls.set(os_calls.get() + 1);
                    Some(PathBuf::from("/temporary"))
                },
                || product
            )
            .is_err());
        }
        assert_eq!(os_calls.get(), 0);
    }

    #[test]
    fn 起動時の確定保存先で製品開発検査版を分離する() {
        let os = PathBuf::from("/data");
        let mut roots = Vec::new();
        for (development, identifier) in [
            (false, "com.picoagent.desktop"),
            (true, "com.picoagent.desktop"),
            (true, "com.picoagent.ui-check"),
        ] {
            roots.push(
                resolve_data_dir_with(
                    development,
                    identifier,
                    || Some(os.clone()),
                    || Ok(os.join(identifier)),
                )
                .unwrap(),
            );
        }
        assert_eq!(
            roots,
            [
                os.join("PicoAgent"),
                os.join("PicoAgent-Dev"),
                os.join("PicoAgent-Dev/instances/com.picoagent.ui-check")
            ]
        );
        for left in 0..roots.len() {
            for right in left + 1..roots.len() {
                assert_ne!(roots[left], roots[right]);
            }
        }
    }

    #[test]
    fn setupは保存先を確定登録してから依存初期化を始める() {
        let source = crate::guard_scan::backend_sources()
            .iter()
            .find(|source| source.path.ends_with("lib.rs"))
            .unwrap()
            .body();
        let setup = source.split(".setup(|app| {").nth(1).unwrap();
        let resolve = setup.find("app_paths::resolve_data_dir").unwrap();
        let registered = setup.find("app.manage(app_paths::ManagedDataDir").unwrap();
        let first_consumer = setup
            .find("std::fs::create_dir_all(&managed_data)")
            .unwrap();
        assert!(resolve < registered && registered < first_consumer);
        let path_source = crate::guard_scan::backend_sources()
            .iter()
            .find(|source| source.path.ends_with("app_paths.rs"))
            .unwrap()
            .body();
        let lookup = path_source
            .split("pub fn data_dir(")
            .nth(1)
            .unwrap()
            .split("pub fn cache_dir")
            .next()
            .unwrap();
        assert!(lookup.contains("app.state::<ManagedDataDir>()"));
        for alternate in [
            "resolve_data_dir",
            "dirs::data_dir",
            "temp_dir",
            "app_data_dir",
        ] {
            assert!(!lookup.contains(alternate));
        }
    }

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
        assert!(resolve_data_dir_with(
            true,
            "com.picoagent.desktop",
            || Some(os.to_path_buf()),
            || unreachable!("開発版では製品領域を取得しない")
        )
        .unwrap()
        .ends_with("PicoAgent-Dev"));
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
