// log_paths.rs - 実行時ログをまとめる共通パス。

use std::path::PathBuf;

use chrono::{Duration, Local, NaiveDate};
use tauri::{AppHandle, Manager};

// ログの保持期間（日）。1日ごとにファイルを分け、これより古い日付のログは消す。
// 履歴（history-<キャラID>.json）は対話データなのでローテーション対象に含めない。
const LOG_RETAIN_DAYS: i64 = 7;

pub fn data_dir(app: &AppHandle) -> PathBuf {
    match app.path().app_data_dir() {
        Ok(path) => path
            .parent()
            .map(|parent| parent.join("PicoAgent"))
            .unwrap_or(path),
        // フォールバック許可: 保存先取得自体の失敗を記録するため、ログだけは一時領域へ退避する。
        Err(error) => {
            eprintln!("ログ保存先を取得できないため一時領域を使います: {error}");
            std::env::temp_dir().join("PicoAgent")
        }
    }
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
    // 旧版の診断書き出し名も保持期間へ乗せる。日付より後ろの時刻は検査しない。
    if let Some(rest) = name.strip_prefix("diagnostics-") {
        return rest
            .get(..10)
            .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok());
    }
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

/// 保持期間の対象にするファイル名か。
///
/// `.log` に加えて、診断情報の書き出し（`diagnostics.<日付>.<時刻>.txt`）も対象にする。
/// ログと同じ置き場へ、押すたびに版・設定・直近ログを丸ごと書き出すため、対象から外すと
/// 「ログは1週間で自動削除」と案内しているのに、中身の濃いファイルだけが残り続ける。
/// 履歴 JSON など、それ以外のファイルは触らない。
fn is_prunable_log(name: &str) -> bool {
    name.ends_with(".log")
        || ((name.starts_with("diagnostics.") || name.starts_with("diagnostics-"))
            && name.ends_with(".txt"))
}

/// 保持期間（`LOG_RETAIN_DAYS` 日）より古い日付のログファイルを削除する。
/// 当日を含めて直近 `LOG_RETAIN_DAYS` 日分だけ残す。
pub fn prune_old_logs(app: &AppHandle) {
    let dir = log_dir(app);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = Local::now().date_naive() - Duration::days(LOG_RETAIN_DAYS - 1);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_prunable_log(&name) {
            continue;
        }
        let Some(date) = date_in_name(&name) else {
            continue;
        };
        if date < cutoff {
            // フォールバック許可: 使用中ファイル等で削除に失敗しても起動を止めない。
            if let Err(error) = std::fs::remove_file(entry.path()) {
                append_app_log(app, &format!("古いログを削除できません: {error}"));
            }
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

/// ログファイルの末尾を読む。`max_bytes` だけ読み、切り出しで途中から始まった
/// 先頭行は捨てる（読みかけの行を原因のように見せない）。読めない場合は空を返す。
pub fn read_tail(app: &AppHandle, name: &str, max_lines: usize, max_bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let path = log_file(app, name);
    let Ok(mut file) = std::fs::File::open(&path) else {
        return String::new();
    };
    let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let partial_head = size > max_bytes;
    if partial_head && file.seek(SeekFrom::End(-(max_bytes as i64))).is_err() {
        return String::new();
    }
    let mut buffer = Vec::new();
    if file.take(max_bytes).read_to_end(&mut buffer).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&buffer);
    tail_lines(&text, max_lines, partial_head)
}

/// 末尾から `max_lines` 行を取り出す。`partial_head` のときは、切り出しで
/// 途中から始まった先頭行を捨てる（読みかけの行を原因のように見せない）。
fn tail_lines(text: &str, max_lines: usize, partial_head: bool) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    if partial_head && lines.len() > 1 {
        lines.remove(0);
    }
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
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
        std::fs::remove_file(&old_path)?;
        Ok(())
    })();
    // フォールバック許可: 旧ログ移行に失敗しても起動とログ出力は継続する。
    if let Err(error) = result {
        append_app_log(app, &format!("旧ログの移行に失敗しました: {error}"));
    }
}

#[cfg(test)]
mod guard {
    use crate::guard_scan;

    /// 画面操作・通知の失敗を捨てると、設定や内部状態だけ変わって表示が追従しない。
    #[test]
    fn 画面操作と通知の失敗を無視しない() {
        let operations = [".emit(", ".show(", ".set_focus(", ".unminimize(", ".close("];
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "log_paths.rs" {
                continue;
            }
            for line in source.body().lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("let _ =")
                    && operations
                        .iter()
                        .any(|operation| trimmed.contains(operation))
                {
                    violations.push(format!("{}: {trimmed}", source.path.display()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "画面操作と通知の失敗は診断ログへ残してください:\n{}",
            violations.join("\n")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{date_in_name, is_prunable_log, tail_lines};

    #[test]
    fn 診断情報の書き出しも保持期間の対象にする() {
        assert!(is_prunable_log("diagnostics-2026-09-03-080000.txt"));
        assert_eq!(
            date_in_name("diagnostics-2026-09-03-080000.txt"),
            chrono::NaiveDate::from_ymd_opt(2026, 9, 3)
        );
        assert_eq!(date_in_name("diagnostics-不正.txt"), None);
        // 「ログは1週間で自動削除」と案内している置き場なので、同じ規則で消える名前にする。
        assert!(is_prunable_log("diagnostics.2026-09-03.080000.txt"));
        assert_eq!(
            date_in_name("diagnostics.2026-09-03.080000.txt"),
            chrono::NaiveDate::from_ymd_opt(2026, 9, 3)
        );
        assert!(is_prunable_log("app.2026-09-03.log"));
        // 対話データは触らない。
        assert!(!is_prunable_log("history-mugi.json"));
        assert!(!is_prunable_log("メモ.txt"));
    }

    #[test]
    fn 直近ログは末尾だけを出す() {
        let text = "1行目\n2行目\n3行目\n4行目\n";
        assert_eq!(tail_lines(text, 2, false), "3行目\n4行目");
        assert_eq!(tail_lines(text, 9, false), "1行目\n2行目\n3行目\n4行目");
        // 途中から始まった先頭行は捨てる（読みかけを原因のように見せない）。
        assert_eq!(tail_lines(text, 9, true), "2行目\n3行目\n4行目");
        // 1行しか無いときは捨てない（何も出せなくなるため）。
        assert_eq!(tail_lines("だけ", 9, true), "だけ");
        assert_eq!(tail_lines("", 9, false), "");
        assert_eq!(tail_lines("\n\n", 9, false), "\n");
    }
}
