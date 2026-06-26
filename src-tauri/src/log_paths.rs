// log_paths.rs - 実行時ログをまとめる共通パス。

use std::path::PathBuf;

use chrono::Local;
use tauri::{AppHandle, Manager};

pub fn data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("PicoAgent")))
        .unwrap_or_else(|| std::env::temp_dir().join("PicoAgent"))
}

pub fn log_dir(app: &AppHandle) -> PathBuf {
    data_dir(app).join("log")
}

pub fn bundled_dir(app: &AppHandle) -> PathBuf {
    data_dir(app).join("bundled")
}

pub fn log_file(app: &AppHandle, name: &str) -> PathBuf {
    log_dir(app).join(name)
}

pub fn ensure_log_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let dir = log_dir(app);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn append_log(app: &AppHandle, name: &str, message: &str) -> anyhow::Result<()> {
    let path = log_file(app, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let now = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = message.replace('\r', "\\r").replace('\n', "\\n");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "[{now}] {line}")?;
    Ok(())
}

pub fn append_app_log(app: &AppHandle, message: &str) {
    // フォールバック許可: ログ書き込み失敗時もアプリ動作を止めない。
    if let Err(error) = append_log(app, "app.log", message) {
        eprintln!("app.log への書き込みに失敗しました: {error}");
    }
}

pub fn migrate_legacy_app_log(app: &AppHandle, name: &str) {
    let new_path = log_file(app, name);
    if new_path.exists() {
        return;
    }
    let mut old_paths = Vec::new();
    if let Ok(old_dir) = app.path().app_log_dir() {
        old_paths.push(old_dir.join(name));
    }
    if let Ok(old_data_dir) = app.path().app_data_dir() {
        old_paths.push(old_data_dir.join("log").join(name));
    }
    let Some(old_path) = old_paths.into_iter().find(|path| path.exists()) else {
        return;
    };
    let result = (|| -> anyhow::Result<()> {
        if let Some(parent) = new_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(old_path, new_path)?;
        Ok(())
    })();
    // フォールバック許可: 旧ログ移行に失敗しても起動とログ出力は継続する。
    if let Err(error) = result {
        append_app_log(app, &format!("旧ログの移行に失敗しました: {error}"));
    }
}
