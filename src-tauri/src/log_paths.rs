// log_paths.rs - 実行時ログをまとめる共通パス。

use std::path::PathBuf;

use chrono::{Duration, Local, NaiveDate};
use tauri::{AppHandle, Manager};

// ログの保持期間（日）。1日ごとにファイルを分け、これより古い日付のログは消す。
// 履歴（history-<キャラID>.json）は対話データなのでローテーション対象に含めない。
const LOG_RETAIN_DAYS: i64 = 7;

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

/// ベース名（例 `app.log`）に当日の日付を挟んだファイル名を返す（例 `app.2026-06-28.log`）。
/// これにより 1 日ごとにログファイルが切り替わる（日付跨ぎの書き込みも自動で翌日ファイルへ）。
fn dated_name(name: &str) -> String {
    let date = Local::now().format("%Y-%m-%d");
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.{date}.{ext}"),
        None => format!("{name}.{date}"),
    }
}

/// ファイル名に含まれる `YYYY-MM-DD` を取り出す（ローテーション済みログの判定用）。
fn date_in_name(name: &str) -> Option<NaiveDate> {
    name.split('.')
        .find_map(|part| NaiveDate::parse_from_str(part, "%Y-%m-%d").ok())
}

pub fn log_file(app: &AppHandle, name: &str) -> PathBuf {
    log_dir(app).join(dated_name(name))
}

pub fn ensure_log_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let dir = log_dir(app);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 保持期間（`LOG_RETAIN_DAYS` 日）より古い日付のログファイルを削除する。
/// 当日を含めて直近 `LOG_RETAIN_DAYS` 日分だけ残す。`.log` 以外（履歴 JSON 等）は触らない。
pub fn prune_old_logs(app: &AppHandle) {
    let dir = log_dir(app);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = Local::now().date_naive() - Duration::days(LOG_RETAIN_DAYS - 1);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".log") {
            continue;
        }
        let Some(date) = date_in_name(&name) else {
            continue;
        };
        if date < cutoff {
            // フォールバック許可: 使用中ファイル等で削除に失敗しても起動を止めない。
            let _ = std::fs::remove_file(entry.path());
        }
    }
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
        std::fs::copy(&old_path, &new_path)?;
        // 旧ファイルは移行後に消す。残すと日付別ファイルへ毎日コピーし直してしまう。
        let _ = std::fs::remove_file(&old_path);
        Ok(())
    })();
    // フォールバック許可: 旧ログ移行に失敗しても起動とログ出力は継続する。
    if let Err(error) = result {
        append_app_log(app, &format!("旧ログの移行に失敗しました: {error}"));
    }
}
