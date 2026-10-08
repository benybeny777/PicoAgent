// log_paths.rs - 実行時ログをまとめる共通パス。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDate};
use tauri::{AppHandle, Manager};

// ログの保持期間（日）。1日ごとにファイルを分け、これより古い日付のログは消す。
// 履歴（history-<キャラID>.json）は対話データなのでローテーション対象に含めない。
const LOG_RETAIN_DAYS: i64 = 7;

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

/// OSの利用者データ領域から開発用の保存先を決める。
fn dev_data_root_for(os_data_dir: &Path) -> PathBuf {
    os_data_dir.join("PicoAgent-Dev")
}

/// このビルドの同梱物（`resources/bundled`）の唯一の置き場所。
///
/// 製品ビルドはインストール先の `resources/bundled/`、開発ビルドはリポジトリの
/// `src-tauri/resources/bundled/`。作業フォルダや実行ファイルの位置から推測したり、
/// 無いときに別の置き場所を探したりしない（開発中の資産を製品で使わないため）。
pub fn packaged_bundled_dir(app: &AppHandle) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/bundled"));
    }
    app.path()
        .resource_dir()
        .ok()
        .map(|root| root.join("resources/bundled"))
}

/// キャラ作成の同梱資産（`character-maker`）の唯一の置き場所。考え方は `packaged_bundled_dir` と同じ。
pub fn packaged_character_maker_dir(app: &AppHandle) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|root| root.join("character-maker"));
    }
    app.path()
        .resource_dir()
        .ok()
        .map(|root| root.join("character-maker"))
}

/// 保存先の根と識別子から、このアプリが使う管理領域を決める。
/// 製品の識別子は根をそのまま使い、検査用など別の識別子は `instances/<識別子>` へ分ける。
fn managed_dir(root: PathBuf, identifier: &str) -> PathBuf {
    if identifier == "com.picoagent.desktop" {
        root
    } else {
        root.join("instances").join(identifier)
    }
}

pub fn data_dir(app: &AppHandle) -> PathBuf {
    let identifier = app.config().identifier.as_str();
    if let Some(root) = dev_data_root() {
        return managed_dir(root, identifier);
    }
    match app.path().app_data_dir() {
        Ok(path) => {
            let root = path
                .parent()
                .map(|parent| parent.join("PicoAgent"))
                .unwrap_or(path);
            managed_dir(root, identifier)
        }
        // フォールバック許可: 保存先取得自体の失敗を記録するため、ログだけは一時領域へ退避する。
        Err(error) => {
            eprintln!("ログ保存先を取得できないため一時領域を使います: {error}");
            std::env::temp_dir().join("PicoAgent")
        }
    }
}

/// webview へ asset プロトコルで読ませる保存先配下のフォルダ。
/// `tauri.conf.json` の `assetProtocol.scope`（`$DATA/PicoAgent/<フォルダ>/**`）と同じ並びを保つ。
pub const ASSET_SCOPE_DIRS: [&str; 4] = [
    "characters",
    "display",
    "character-maker/characters",
    "generated-media",
];

/// 静的な許可範囲は通常版の保存先（`$DATA/PicoAgent`）だけを指す。開発ビルドの `PicoAgent-Dev` や
/// 検査用の別識別子（`instances/<識別子>`）でも同じフォルダだけを表示できるよう、起動時に足す。
/// 通常版では静的な範囲と同じ場所なので、許可は広がらない。
pub fn allow_asset_scope(app: &AppHandle) {
    let root = data_dir(app);
    for dir in ASSET_SCOPE_DIRS {
        if let Err(error) = app
            .asset_protocol_scope()
            .allow_directory(root.join(dir), true)
        {
            append_app_log(
                app,
                &format!(
                    "画像の読込許可を追加できません: {} ({error})",
                    root.join(dir).display()
                ),
            );
        }
    }
}

pub fn cache_dir(app: &AppHandle) -> PathBuf {
    data_dir(app).join("cache")
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

/// `AppHandle` を引数で受け取れない位置（終了処理、コールバック、スキルの内部）から
/// app ログへ残す。
///
/// GUIアプリには標準エラー出力の行き先が無い（Windowsはコンソールへ接続しない）。
/// `eprintln!` で書いた失敗はどこにも残らないため、原因の追跡ができなくなる。
/// ハンドルがまだ無い起動最初期だけ、最後の手段として stderr へ落とす。
pub fn log_detached(message: &str) {
    match crate::durable_jobs::app_handle() {
        Some(app) => append_app_log(app, message),
        None => eprintln!("{message}"),
    }
}

#[cfg(test)]
mod guard {
    use crate::guard_scan;

    /// OSの保存先や同梱物の場所を直接取ると、開発ビルドが通常版の利用者データ（`%APPDATA%` など）を
    /// 触ったり、同じPCでビルドした製品が開発中の資産を使ったりする。保存先は `data_dir()`、
    /// 同梱物は `packaged_bundled_dir()` / `components::runtime_dirs()` を通す。
    #[test]
    fn 保存先と同梱物はlog_pathsを通して開発ビルドと製品を分ける() {
        const DIRECT: &[&str] = &[
            ".app_data_dir()",
            ".app_local_data_dir()",
            ".app_config_dir()",
            ".app_log_dir()",
            "dirs::data_dir()",
            "dirs::data_local_dir()",
            "dirs::config_dir()",
            ".resource_dir()",
            "current_dir()",
            "current_exe()",
            "CARGO_MANIFEST_DIR",
        ];
        // 理由付きの例外。
        const ALLOWED: &[&str] = &[
            "src/log_paths.rs",       // 保存先・同梱物の正本（開発ビルドの判定はここだけ）
            "src/migration.rs",       // 旧識別子フォルダの移行（開発ビルドは冒頭で抜ける）
            "browser_control/cdp.rs", // ブラウザ専用プロファイル（開発ビルドは dev_data_root を先に返す）
            "src/doc_sync.rs",        // テスト専用モジュール（文書と実装の突き合わせ）
            "src/guard_scan.rs",      // テスト専用モジュール（設計ガードの走査範囲）
        ];
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            let path = source.path.display().to_string().replace('\\', "/");
            if ALLOWED.iter().any(|allowed| path.ends_with(allowed)) {
                continue;
            }
            for line in source.body().lines() {
                let trimmed = line.trim();
                if !trimmed.starts_with("//")
                    && DIRECT.iter().any(|direct| trimmed.contains(direct))
                {
                    violations.push(format!("{}: {trimmed}", source.path.display()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "保存先は log_paths::data_dir()、同梱物は packaged_bundled_dir() を通してください:\n{}",
            violations.join("\n")
        );
    }

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

    /// GUIアプリには標準エラー出力の行き先が無い（Windowsはコンソールへ接続しない）。
    /// `eprintln!` で書いた失敗はどこにも残らず、握りつぶしたのと同じになる。
    /// 例外は、ログ基盤そのものがまだ使えない起動最初期だけ。
    #[test]
    fn 本番経路の失敗をstderrへ捨てない() {
        // 理由付きの例外。どちらも「ログへ書けないこと自体」を伝える最後の手段。
        const ALLOWED: &[&str] = &[
            "src/log_paths.rs", // ログ保存先の取得失敗と app.log への書き込み失敗
            "src/lib.rs",       // ログ置き場を作れなかった起動最初期
        ];
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            let path = source.path.display().to_string().replace('\\', "/");
            if ALLOWED.iter().any(|allowed| path.ends_with(allowed)) {
                continue;
            }
            for (number, line) in source.body().lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains("eprintln!") {
                    violations.push(format!("{path}:{}", number + 1));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "失敗は log_paths::append_app_log / log_detached でログへ残してください（stderr はGUIでは届きません）: {violations:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        date_in_name, dev_data_root, dev_data_root_for, is_prunable_log, managed_dir, tail_lines,
    };
    use std::path::{Path, PathBuf};

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
    fn 実行時に足す読込許可は静的な許可範囲と同じフォルダだけにする() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let scope: Vec<String> = config["app"]["security"]["assetProtocol"]["scope"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect();
        let expected: Vec<String> = super::ASSET_SCOPE_DIRS
            .iter()
            .map(|dir| format!("$DATA/PicoAgent/{dir}/**"))
            .collect();
        assert_eq!(scope, expected);
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
