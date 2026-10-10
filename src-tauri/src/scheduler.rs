// scheduler.rs - 指定時刻に許可済みコマンドを実行する常駐タスク。
//
// タスク一覧は設定キー `PICOAGENT_SCHEDULED_TASKS` に JSON 文字列で保存する。実行は
// run_command と同じ許可ルート・allowlist・有効化フラグを使い、shell は使わない。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};

use crate::config::Config;

pub const CONFIG_KEY: &str = "PICOAGENT_SCHEDULED_TASKS";
/// 発火判定専用。設定画面が編集する予約定義とは分け、相互上書きを防ぐ。
pub const STATE_KEY: &str = "PICOAGENT_SCHEDULED_TASK_STATE";
pub const TOOL_ALLOWLIST_KEY: &str = "PICOAGENT_SCHEDULED_TOOL_ALLOWLIST";
const DEFAULT_TOOL_ALLOWLIST: &str = "time,weather,web_fetch";
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_RUN_HISTORY: usize = 100;
const MAX_CATCH_UP_MINUTES: i64 = 24 * 60;

// 曜日・時刻の語彙と正規化は `alarm` が唯一の定義元。以前はこのファイルへ
// 丸ごと写されており、`days_label` は既に書き方が分かれていた（挙動は同じだが、
// 片方だけ直せば静かにずれる状態だった）。
pub use crate::alarm::{days_label, normalize_days, normalize_time};

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    /// command なら program/args を実行、tool なら tool_name/tool_args を dispatch する。
    #[serde(default = "default_kind")]
    pub kind: String,
    /// "HH:MM"（24時間表記）。cron を使う場合は空でよい。
    #[serde(default)]
    pub time: String,
    /// cron 風の5フィールド（分 時 日 月 曜日）。`*` と数値、カンマ区切り、`*/N` に対応。
    /// 日と曜日を両方制限した場合は標準cron互換でORとして扱う。
    #[serde(default)]
    pub cron: String,
    /// 曜日 mon..sun。空なら毎日。
    #[serde(default)]
    pub days: Vec<String>,
    /// "YYYY-MM-DD"。指定時だけその日付に限定する。
    #[serde(default)]
    pub date: String,
    /// false なら1回実行後に無効化する。
    #[serde(default = "default_true")]
    pub repeat: bool,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_args: Value,
    #[serde(default)]
    pub last_run_minute: String,
    /// 最後に発火判定を終えた分。スリープ・再起動後の未処理分を限定的に追跡する。
    #[serde(default)]
    pub last_checked_minute: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduledRunRecord {
    pub id: String,
    pub task_id: String,
    pub title: String,
    pub kind: String,
    pub target: String,
    pub ok: bool,
    pub at: String,
    pub body: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ScheduledTaskState {
    #[serde(default)]
    last_run_minute: String,
    #[serde(default)]
    last_checked_minute: String,
}

fn default_kind() -> String {
    "command".to_string()
}

pub fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("st{nanos}")
}

/// 保存済みのスケジュール一覧を読む。壊れていた場合は空一覧にせずエラーを返す
/// （空から始めて書き戻すと、登録済みのタスクがすべて消えるため）。
pub fn load(cfg: &Config) -> Result<Vec<ScheduledTask>, String> {
    let mut tasks: Vec<ScheduledTask> = crate::store::load_list(cfg, CONFIG_KEY)?;
    let state = load_state(cfg)?;
    for task in &mut tasks {
        if let Some(saved) = state.get(&task.id) {
            task.last_run_minute.clone_from(&saved.last_run_minute);
            task.last_checked_minute
                .clone_from(&saved.last_checked_minute);
        }
    }
    Ok(tasks)
}

pub fn save(cfg: &Config, tasks: &[ScheduledTask]) -> anyhow::Result<()> {
    let definitions = task_definitions(tasks);
    let state = task_state(tasks);
    let mut patch = Map::new();
    patch.insert(
        CONFIG_KEY.to_string(),
        Value::String(serde_json::to_string(&definitions)?),
    );
    patch.insert(
        STATE_KEY.to_string(),
        Value::String(serde_json::to_string(&state)?),
    );
    cfg.set_all(patch)
}

fn load_state(cfg: &Config) -> Result<BTreeMap<String, ScheduledTaskState>, String> {
    let raw = cfg.get(STATE_KEY, "");
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(&raw).map_err(|error| {
        format!("保存済みのスケジュール実行状態を読み取れません（{error}）。上書きを中止しました。")
    })
}

fn task_definitions(tasks: &[ScheduledTask]) -> Vec<ScheduledTask> {
    tasks
        .iter()
        .cloned()
        .map(|mut task| {
            task.last_run_minute.clear();
            task.last_checked_minute.clear();
            task
        })
        .collect()
}

fn task_state(tasks: &[ScheduledTask]) -> BTreeMap<String, ScheduledTaskState> {
    tasks
        .iter()
        .filter(|task| !task.last_run_minute.is_empty() || !task.last_checked_minute.is_empty())
        .map(|task| {
            (
                task.id.clone(),
                ScheduledTaskState {
                    last_run_minute: task.last_run_minute.clone(),
                    last_checked_minute: task.last_checked_minute.clone(),
                },
            )
        })
        .collect()
}

/// 設定画面が送った予約定義に、最新の発火状態だけを別キーで維持する。
pub fn reconcile_settings_patch(
    current: &Map<String, Value>,
    patch: &mut Map<String, Value>,
) -> Result<(), String> {
    let Some(raw) = patch.get(CONFIG_KEY).and_then(Value::as_str) else {
        return Ok(());
    };
    let definitions: Vec<ScheduledTask> = serde_json::from_str(raw)
        .map_err(|error| format!("スケジュールタスク一覧を読めません: {error}"))?;
    let ids = definitions
        .iter()
        .map(|task| task.id.as_str())
        .collect::<HashSet<_>>();
    let state_raw = current.get(STATE_KEY).and_then(Value::as_str).unwrap_or("");
    let mut state: BTreeMap<String, ScheduledTaskState> = if state_raw.trim().is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_str(state_raw)
            .map_err(|error| format!("保存済みのスケジュール実行状態を読めません: {error}"))?
    };
    // 旧形式は予約定義の中に状態を持つ。初回の設定保存でも二重実行防止状態を移行する。
    if let Some(current_raw) = current.get(CONFIG_KEY).and_then(Value::as_str) {
        if let Ok(current_tasks) = serde_json::from_str::<Vec<ScheduledTask>>(current_raw) {
            for task in current_tasks {
                if state.contains_key(&task.id)
                    || (task.last_run_minute.is_empty() && task.last_checked_minute.is_empty())
                {
                    continue;
                }
                state.insert(
                    task.id,
                    ScheduledTaskState {
                        last_run_minute: task.last_run_minute,
                        last_checked_minute: task.last_checked_minute,
                    },
                );
            }
        }
    }
    state.retain(|id, _| ids.contains(id.as_str()));
    patch.insert(
        CONFIG_KEY.to_string(),
        Value::String(
            serde_json::to_string(&task_definitions(&definitions)).map_err(|e| e.to_string())?,
        ),
    );
    patch.insert(
        STATE_KEY.to_string(),
        Value::String(serde_json::to_string(&state).map_err(|e| e.to_string())?),
    );
    Ok(())
}

fn run_history_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(crate::app_paths::data_path(
        app,
        crate::app_paths::DataPath::ScheduleRuns,
    ))
}

pub fn load_run_history(app: &AppHandle) -> anyhow::Result<Vec<ScheduledRunRecord>> {
    let _guard = crate::store::lock_state("schedule-runs");
    load_run_history_unlocked(app)
}

fn load_run_history_unlocked(app: &AppHandle) -> anyhow::Result<Vec<ScheduledRunRecord>> {
    let path = run_history_path(app)?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(anyhow::anyhow!(
                "スケジュール実行履歴を読み込めません: {} ({error})",
                path.display()
            ));
        }
    };
    match serde_json::from_str(&text) {
        Ok(records) => Ok(records),
        // フォールバック許可: PR #463 採用案3。退避と通知に成功した領域だけ新規状態で続ける。（根拠: PR #463 採用案。退避・通知と原本保護）
        // 次の保存で上書きして履歴を失わないよう、破損ファイルは退避してから続ける。
        Err(error) => {
            let saved = crate::store::quarantine_and_notify(
                app,
                &path,
                crate::store::QuarantineArea::SchedulerHistory,
            )
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "壊れたスケジュール実行履歴を退避できません。上書きを中止しました: {error}"
                )
            })?;
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "スケジュール実行履歴を読めないため空で起動します（{error}）。破損ファイルは {} へ退避しました",
                    saved.display()
                ),
            );
            Ok(Vec::new())
        }
    }
}

fn save_run_history(app: &AppHandle, records: &[ScheduledRunRecord]) -> anyhow::Result<()> {
    let path = run_history_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::store::write_json_atomic(&path, &records)?;
    Ok(())
}

fn task_title(task: &ScheduledTask) -> String {
    if !task.label.trim().is_empty() {
        return task.label.clone();
    }
    if task.kind.trim().eq_ignore_ascii_case("tool") {
        format!("スケジュール: {}", task.tool_name)
    } else {
        format!("スケジュール: {}", task.program)
    }
}

fn task_target(task: &ScheduledTask) -> String {
    if task.kind.trim().eq_ignore_ascii_case("tool") {
        format!("{} {}", task.tool_name, task.tool_args)
    } else {
        format!("{} {}", task.program, task.args.join(" "))
    }
}

fn new_run_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("sr{nanos}")
}

fn append_run_history(
    app: &AppHandle,
    task: &ScheduledTask,
    result: &str,
    at: &str,
) -> ScheduledRunRecord {
    let record = ScheduledRunRecord {
        id: new_run_id(),
        task_id: task.id.clone(),
        title: task_title(task),
        kind: task.kind.clone(),
        target: task_target(task),
        ok: !result.starts_with("エラー:"),
        at: at.to_string(),
        body: result.to_string(),
    };
    let _guard = crate::store::lock_state("schedule-runs");
    let mut records = match load_run_history_unlocked(app) {
        Ok(records) => records,
        Err(error) => {
            crate::log_paths::append_app_log(
                app,
                &format!("スケジュール実行履歴を更新できません: {error}"),
            );
            notify_scheduler_history_state(app, Some(&error.to_string()));
            return record;
        }
    };
    records.insert(0, record.clone());
    records.truncate(MAX_RUN_HISTORY);
    if let Err(error) = save_run_history(app, &records) {
        crate::log_paths::append_app_log(
            app,
            &format!("スケジュール実行履歴の保存に失敗しました: {error}"),
        );
        notify_scheduler_history_state(app, Some(&error.to_string()));
    } else {
        notify_scheduler_history_state(app, None);
    }
    record
}

fn emit_run_notice(app: &AppHandle, record: &ScheduledRunRecord) {
    crate::notice::emit(
        app,
        crate::notice::Notice::info("schedule", &record.title, &record.body, record.ok)
            .detail("taskId", json!(record.task_id))
            .detail("runId", json!(record.id)),
    );
}

fn bounded_cwd(cfg: &Config, cwd: &str) -> Result<PathBuf, String> {
    crate::permissions::resolve(cfg, if cwd.trim().is_empty() { "." } else { cwd })
        .map(|path| path.as_path().to_path_buf())
}

fn is_protected_edit_path(path: &Path) -> bool {
    crate::permissions::is_system_protected(path)
}

pub fn validate_task(cfg: &Config, task: &ScheduledTask) -> Result<(), String> {
    if !task.cron.trim().is_empty() {
        validate_cron(&task.cron)?;
    } else if normalize_time(&task.time).is_none() {
        return Err("time は HH:MM（24時間表記）で指定してください。".to_string());
    }
    if !task.date.trim().is_empty()
        && chrono::NaiveDate::parse_from_str(&task.date, "%Y-%m-%d").is_err()
    {
        return Err("date は YYYY-MM-DD で指定してください。".to_string());
    }
    match task.kind.trim().to_lowercase().as_str() {
        "" | "command" => {
            if task.program.trim().is_empty() {
                return Err("program が指定されていません。".to_string());
            }
            crate::tools::process::command_policy::check(cfg, &task.program, &task.args)?;
        }
        "tool" => {
            if task.tool_name.trim().is_empty() {
                return Err("tool_name が指定されていません。".to_string());
            }
            if task.tool_name == "task_scheduler" {
                return Err(
                    "task_scheduler 自身をスケジュール実行することはできません。".to_string(),
                );
            }
            let allowed = cfg
                .get(TOOL_ALLOWLIST_KEY, DEFAULT_TOOL_ALLOWLIST)
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .any(|name| {
                    name == task.tool_name
                        || (name == "genspark_suite:light"
                            && task.tool_name == "genspark_suite"
                            && task.tool_args.get("mode").and_then(Value::as_str) == Some("light"))
                });
            if !allowed {
                return Err(format!(
                    "スケジュール実行を許可されていないスキルです: {}。設定の {} で明示的に許可してください。",
                    task.tool_name, TOOL_ALLOWLIST_KEY
                ));
            }
            if !task.tool_args.is_object() {
                return Err("tool_args は JSON オブジェクトで指定してください。".to_string());
            }
        }
        other => {
            return Err(format!(
                "kind は command または tool を指定してください: {other}"
            ))
        }
    }
    Ok(())
}

fn trim_output(label: &str, bytes: &[u8]) -> String {
    format!(
        "{label}:\n{}",
        crate::text::command_output_within(bytes, MAX_OUTPUT_BYTES)
    )
}

async fn execute_task(app: &AppHandle, cfg: &Config, task: &ScheduledTask) -> String {
    if task.kind.trim().eq_ignore_ascii_case("tool") {
        if let Err(error) = validate_task(cfg, task) {
            return format!("エラー: {error}");
        }
        crate::log_paths::append_app_log(
            app,
            &format!(
                "スケジュールスキル実行開始: id={} tool={}",
                task.id, task.tool_name
            ),
        );
        let result = crate::tools::dispatch(cfg, &task.tool_name, task.tool_args.clone()).await;
        return format!(
            "スケジュールスキル完了: id={} tool={}\n{}",
            task.id, task.tool_name, result
        );
    }
    if !cfg.get_bool("PICOAGENT_COMMAND_ENABLED", false) {
        return "エラー: PICOAGENT_COMMAND_ENABLED が true ではないため実行しません。".to_string();
    }
    if let Err(error) = validate_task(cfg, task) {
        return format!("エラー: {error}");
    }
    let cwd = match bounded_cwd(cfg, &task.cwd) {
        Ok(cwd) => cwd,
        Err(error) => return format!("エラー: {error}"),
    };
    if is_protected_edit_path(&cwd) {
        return format!(
            "エラー: Windows / Program Files 配下は編集不可のため、コマンド実行先にできません: {}",
            cwd.display()
        );
    }
    let default_timeout = match cfg.int(&crate::config::ints::COMMAND_TIMEOUT_SECS) {
        Ok(value) => value as u64,
        Err(error) => return format!("エラー: {error}"),
    };
    let timeout_secs = match task.timeout_secs {
        Some(value) if !(1..=120).contains(&value) => {
            return "エラー: タスクの待ち時間は1〜120秒にしてください".to_string();
        }
        Some(value) => value,
        None => default_timeout,
    };

    let mut command = crate::tools::process::command(&task.program);
    command.args(&task.args).current_dir(&cwd);

    crate::log_paths::append_app_log(
        app,
        &format!(
            "スケジュールタスク実行開始: id={} command={} {} cwd={}",
            task.id,
            task.program,
            task.args.join(" "),
            cwd.display()
        ),
    );
    let output = match crate::tools::process::output(command, timeout_secs, "スケジュールコマンド")
        .await
    {
        Ok(output) => output,
        Err(error) => return format!("エラー: {error}"),
    };
    let mut result = format!(
        "スケジュールタスク完了: id={} 終了コード: {}\n作業ディレクトリ: {}\n",
        task.id,
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "シグナル終了".to_string()),
        cwd.display()
    );
    result.push_str(&trim_output("stdout", &output.stdout));
    result.push('\n');
    result.push_str(&trim_output("stderr", &output.stderr));
    result
}

fn weekday_code(at: DateTime<Local>) -> &'static str {
    match at.weekday().num_days_from_monday() {
        0 => "mon",
        1 => "tue",
        2 => "wed",
        3 => "thu",
        4 => "fri",
        5 => "sat",
        _ => "sun",
    }
}

fn matches_fixed_time(task: &ScheduledTask, at: DateTime<Local>) -> bool {
    if !task.date.trim().is_empty() && task.date != at.format("%Y-%m-%d").to_string() {
        return false;
    }
    if !task.days.is_empty() && !task.days.iter().any(|day| day == weekday_code(at)) {
        return false;
    }
    task.time == at.format("%H:%M").to_string()
}

fn parse_cron_value(part: &str, value: u32, min: u32, max: u32) -> bool {
    part.split(',').any(|raw| {
        let token = raw.trim();
        if token == "*" {
            return true;
        }
        if let Some(step) = token.strip_prefix("*/").and_then(|s| s.parse::<u32>().ok()) {
            return step > 0 && value >= min && value <= max && (value - min).is_multiple_of(step);
        }
        token
            .parse::<u32>()
            .map(|want| want == value && want >= min && want <= max)
            .unwrap_or(false)
    })
}

fn cron_weekday_value(at: DateTime<Local>) -> u32 {
    // cron 互換で日曜=0、月曜=1。
    at.weekday().num_days_from_sunday()
}

/// `parse_cron_value` が理解できる字句かどうか。検証と照合で同じ文法を使う。
fn cron_token_valid(part: &str, min: u32, max: u32) -> bool {
    !part.trim().is_empty()
        && part.split(',').all(|raw| {
            let token = raw.trim();
            if token == "*" {
                return true;
            }
            if let Some(step) = token.strip_prefix("*/") {
                return step
                    .parse::<u32>()
                    .is_ok_and(|step| step > 0 && step <= max);
            }
            token
                .parse::<u32>()
                .is_ok_and(|value| value >= min && value <= max)
        })
}

/// 保存前に、照合器が実際に解釈できる書式かまで確かめる。
///
/// 以前はフィールド数しか見ておらず、`1-5`（本物のcronにはある範囲指定）や `99`、
/// 綴り誤りをそのまま保存できた。`matches_cron` はそれらに一致しないため、
/// **登録は成功したように見えて予約が二度と発火しない**。無言で動かないのは、
/// エラーで断られるより悪い。
fn validate_cron(cron: &str) -> Result<(), String> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return Err("cron は「分 時 日 月 曜日」の5フィールドで指定してください。".to_string());
    }
    for (part, (label, min, max)) in parts.iter().zip([
        ("分", 0, 59),
        ("時", 0, 23),
        ("日", 1, 31),
        ("月", 1, 12),
        ("曜日", 0, 6),
    ]) {
        if !cron_token_valid(part, min, max) {
            return Err(format!(
                "cron の「{label}」が解釈できません: {part}\n                 使えるのは `*`、数値（{min}〜{max}）、カンマ区切り、`*/N` です。\
                 `1-5` のような範囲指定には対応していないので `1,2,3,4,5` と書いてください。"
            ));
        }
    }
    Ok(())
}

fn matches_cron(task: &ScheduledTask, at: DateTime<Local>) -> bool {
    let parts: Vec<&str> = task.cron.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }
    let day_matches = parse_cron_value(parts[2], at.day(), 1, 31);
    let weekday_matches = parse_cron_value(parts[4], cron_weekday_value(at), 0, 6);
    // Vixie cron互換: 日と曜日を両方制限した場合だけORで結ぶ。
    // 片方が`*`なら、そのフィールドは常に真なので、もう片方だけで決まる。
    let day_or_weekday_matches = if parts[2].trim() != "*" && parts[4].trim() != "*" {
        day_matches || weekday_matches
    } else {
        day_matches && weekday_matches
    };
    parse_cron_value(parts[0], at.minute(), 0, 59)
        && parse_cron_value(parts[1], at.hour(), 0, 23)
        && parse_cron_value(parts[3], at.month(), 1, 12)
        && day_or_weekday_matches
}

fn should_run_at(task: &ScheduledTask, at: DateTime<Local>) -> bool {
    let minute_stamp = at.format("%Y-%m-%d %H:%M").to_string();
    task.enabled
        && task.last_run_minute != minute_stamp
        && if task.cron.trim().is_empty() {
            matches_fixed_time(task, at)
        } else {
            matches_cron(task, at)
        }
}

fn checked_minute(value: &str) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").ok()?;
    Local.from_local_datetime(&naive).single()
}

fn minutes_to_check(task: &ScheduledTask, now: DateTime<Local>) -> Vec<DateTime<Local>> {
    let Some(previous) = checked_minute(&task.last_checked_minute) else {
        return vec![now];
    };
    if previous > now {
        return Vec::new();
    }
    let start = std::cmp::max(
        previous + chrono::Duration::minutes(1),
        now - chrono::Duration::minutes(MAX_CATCH_UP_MINUTES),
    );
    std::iter::successors(Some(start), |minute| {
        Some(*minute + chrono::Duration::minutes(1))
    })
    .take_while(|minute| *minute <= now)
    .collect()
}

/// 実行すべきタスクを取り出し、二重起動しないよう `last_run_minute` を先に保存する。
///
/// 実行そのものはここで待たない。文字起こしのように 30 分かかりうるタスクを
/// 検出ループの中で `await` すると、その間の tick が進まず、当該時間帯に予定
/// されていた他タスクが（次の分に繰り越されないため）恒久的にスキップされる。
fn enqueue_due_tasks(
    tx: &tokio::sync::mpsc::Sender<Vec<ScheduledTask>>,
    cfg: &Config,
    now: DateTime<Local>,
) -> anyhow::Result<()> {
    // 容量を確保できない間は発火状態を進めず、復旧後の追いつき検出へ残す。
    let permit = tx.try_reserve().map_err(|error| {
        anyhow::anyhow!("スケジュール実行キューを確保できません。予約の検出を再試行します: {error}")
    })?;
    let tasks = take_due_tasks_at(cfg, now)?;
    if !tasks.is_empty() {
        permit.send(tasks);
    }
    Ok(())
}

fn notify_scheduler_state(app: &AppHandle, error: Option<&str>) {
    static FAILED: crate::notice::FailureLatch = crate::notice::FailureLatch::new();
    notify_scheduler_background_state(app, "scheduler-state", error, &FAILED);
}

fn notify_scheduler_history_state(app: &AppHandle, error: Option<&str>) {
    static FAILED: crate::notice::FailureLatch = crate::notice::FailureLatch::new();
    notify_scheduler_background_state(app, "scheduler-history", error, &FAILED);
}

fn notify_scheduler_background_state(
    app: &AppHandle,
    key: &str,
    error: Option<&str>,
    previous: &crate::notice::FailureLatch,
) {
    previous.report(
        app,
        key,
        "スケジュール実行",
        error.map_or(Ok(()), |error| Err(error.to_string())),
        "スケジュールの状態保存が復旧しました",
    );
}

fn take_due_tasks_at(cfg: &Config, now: DateTime<Local>) -> anyhow::Result<Vec<ScheduledTask>> {
    let minute_stamp = now.format("%Y-%m-%d %H:%M").to_string();
    // 読む→直す→書き戻すの間、task_scheduler スキルの更新と競合しないようにする。
    let _guard = crate::store::lock_state(CONFIG_KEY);
    // 読めなかった場合は空一覧で続けない（そのまま書き戻すと予約が全部消える）。
    let mut tasks = load(cfg).map_err(anyhow::Error::msg)?;
    let mut changed = false;
    let mut due = Vec::new();
    for task in &mut tasks {
        // 時計が戻った間は確認済み時刻を維持し、処理済み区間を再走査しない。
        if checked_minute(&task.last_checked_minute).is_some_and(|previous| previous > now) {
            continue;
        }
        let minutes = minutes_to_check(task, now);
        for minute in minutes {
            if should_run_at(task, minute) {
                task.last_run_minute = minute.format("%Y-%m-%d %H:%M").to_string();
                due.push(task.clone());
                if !task.repeat {
                    task.enabled = false;
                    break;
                }
            }
        }
        if task.last_checked_minute != minute_stamp {
            task.last_checked_minute = minute_stamp.clone();
            changed = true;
        }
    }
    if changed || !due.is_empty() {
        // 発火済み状態を保存できなければ実行せず、次の検出で再試行する。
        save(cfg, &tasks)?;
    }
    Ok(due)
}

/// 1タスクを実行し、ログ・実行履歴・通知へ反映する。
async fn run_and_record(app: &AppHandle, cfg: &Config, task: ScheduledTask) {
    let started = Local::now().to_rfc3339();
    let result = execute_task(app, cfg, &task).await;
    crate::log_paths::append_app_log(app, &result);
    let record = append_run_history(app, &task, &result, &started);
    emit_run_notice(app, &record);
}

pub async fn rerun_task(
    app: &AppHandle,
    cfg: &Config,
    task_id: &str,
) -> Result<ScheduledRunRecord, String> {
    let task = load(cfg)?
        .into_iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| format!("指定されたスケジュールタスクは見つかりません: {task_id}"))?;
    let now = Local::now().to_rfc3339();
    let result = execute_task(app, cfg, &task).await;
    crate::log_paths::append_app_log(app, &result);
    let record = append_run_history(app, &task, &result, &now);
    emit_run_notice(app, &record);
    Ok(record)
}

fn migration_gate(ready: &mut bool, prepare: impl FnOnce() -> bool) -> bool {
    if !*ready {
        *ready = prepare();
    }
    *ready
}

pub fn start(app: AppHandle, mut migration_ready: bool) {
    // 検出（tick）と実行（ワーカー）を分ける。実行は 1 本のワーカーで直列に処理するので、
    // 長時間タスクがあっても検出は止まらず、実行履歴の書き込みも競合しない。
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<ScheduledTask>>(64);

    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(tasks) = rx.recv().await {
            let Some(state) = worker_app.try_state::<crate::commands::AppState>() else {
                continue;
            };
            for task in tasks {
                run_and_record(&worker_app, &state.config, task).await;
            }
        }
    });

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let Some(state) = app.try_state::<crate::commands::AppState>() else {
                continue;
            };
            // 失敗中は発火状態やキューを変更しない。設定修正後は同じ検出処理で復旧する。
            if !migration_gate(&mut migration_ready, || {
                crate::workflow_migration::prepare(&app, &state.config)
            }) {
                continue;
            }
            // 1回の検出結果をまとめて送る。容量を先に確保してから発火状態を更新する。
            match enqueue_due_tasks(&tx, &state.config, Local::now()) {
                Ok(()) => notify_scheduler_state(&app, None),
                Err(error) => notify_scheduler_state(&app, Some(&error.to_string())),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        load, minutes_to_check, normalize_days, normalize_time, parse_cron_value,
        reconcile_settings_patch, save, should_run_at, validate_task, ScheduledTask, CONFIG_KEY,
        STATE_KEY,
    };
    use crate::config::Config;
    use chrono::TimeZone;

    #[test]
    fn 移行失敗中は発火へ進まず修正後は一度だけ復旧する() {
        let mut ready = false;
        assert!(!super::migration_gate(&mut ready, || false));
        assert!(!ready);
        assert!(super::migration_gate(&mut ready, || true));
        assert!(super::migration_gate(&mut ready, || panic!(
            "復旧済みの移行を繰り返さない"
        )));
    }

    fn 固定時刻タスク(time: &str) -> ScheduledTask {
        ScheduledTask {
            id: "t1".to_string(),
            kind: "command".to_string(),
            time: time.to_string(),
            cron: String::new(),
            days: Vec::new(),
            date: String::new(),
            repeat: true,
            label: String::new(),
            enabled: true,
            program: "git".to_string(),
            args: vec!["status".to_string()],
            cwd: String::new(),
            timeout_secs: None,
            tool_name: String::new(),
            tool_args: serde_json::Value::Null,
            last_run_minute: String::new(),
            last_checked_minute: String::new(),
        }
    }

    #[test]
    fn 軽量資料の移行許可を全機能へ広げない() {
        let cfg = Config::new();
        let mut task = 固定時刻タスク("07:05");
        task.kind = "tool".into();
        task.tool_name = "genspark_suite".into();
        cfg.set(super::TOOL_ALLOWLIST_KEY, "genspark_suite:light");
        for args in [
            serde_json::json!({}),
            serde_json::json!({"mode":"full"}),
            serde_json::json!({"mode":true}),
            serde_json::json!({"mode":"light-extra"}),
        ] {
            task.tool_args = args;
            assert!(validate_task(&cfg, &task).is_err());
        }
        task.tool_args = serde_json::json!({"mode":"light", "topic":"資料"});
        assert!(validate_task(&cfg, &task).is_ok());
        cfg.set(super::TOOL_ALLOWLIST_KEY, "genspark_suite");
        task.tool_args = serde_json::json!({"mode":"full"});
        assert!(validate_task(&cfg, &task).is_ok());
        cfg.set(super::TOOL_ALLOWLIST_KEY, " , ");
        assert!(validate_task(&cfg, &task).is_err());
    }

    fn test_config(label: &str) -> (Config, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "picoagent-scheduler-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = Config::new();
        cfg.load(dir.join("config.json")).unwrap();
        (cfg, dir)
    }

    #[test]
    fn 予約定義と発火状態を別キーへ保存して再結合する() {
        let (cfg, dir) = test_config("state-split");
        let mut task = 固定時刻タスク("07:05");
        task.last_run_minute = "2026-07-28 07:05".into();
        task.last_checked_minute = "2026-07-28 07:06".into();
        save(&cfg, &[task.clone()]).unwrap();

        let definitions = cfg.get(CONFIG_KEY, "");
        assert!(!definitions.contains("2026-07-28"));
        assert!(cfg.get(STATE_KEY, "").contains("2026-07-28 07:05"));
        let loaded = load(&cfg).unwrap();
        assert_eq!(loaded[0].last_run_minute, task.last_run_minute);
        assert_eq!(loaded[0].last_checked_minute, task.last_checked_minute);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn 発火状態を保存できないと予約を実行キューへ渡さない() {
        let cfg = Config::new();
        cfg.set(
            CONFIG_KEY,
            &serde_json::to_string(&vec![固定時刻タスク("09:00")]).unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        assert!(super::enqueue_due_tasks(&tx, &cfg, minute(1, 9, 0)).is_err());
        assert!(rx.try_recv().is_err());
        assert!(load(&cfg).unwrap()[0].last_run_minute.is_empty());
    }

    #[test]
    fn 満杯のキューは発火状態を進めず復旧後に一括実行する() {
        let (cfg, dir) = test_config("queue-full");
        let tasks = (0..65)
            .map(|index| {
                let mut task = 固定時刻タスク("09:00");
                task.id = format!("task{index}");
                task.repeat = false;
                task.last_checked_minute = "2026-07-01 08:59".into();
                task
            })
            .collect::<Vec<_>>();
        save(&cfg, &tasks).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(Vec::new()).unwrap();
        assert!(super::enqueue_due_tasks(&tx, &cfg, minute(1, 9, 0)).is_err());
        assert!(load(&cfg)
            .unwrap()
            .iter()
            .all(|task| task.enabled && task.last_run_minute.is_empty()));
        rx.try_recv().unwrap();
        super::enqueue_due_tasks(&tx, &cfg, minute(1, 9, 1)).unwrap();
        assert_eq!(rx.try_recv().unwrap().len(), 65);
        super::enqueue_due_tasks(&tx, &cfg, minute(1, 9, 2)).unwrap();
        assert!(rx.try_recv().is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 時計が戻っても確認済み時刻を後退させず同じ予定を再実行しない() {
        let (cfg, dir) = test_config("clock-backward");
        let mut task = cronタスク("* * * * *");
        task.last_checked_minute = "2026-07-01 08:59".into();
        save(&cfg, &[task]).unwrap();
        assert_eq!(
            super::take_due_tasks_at(&cfg, minute(1, 9, 0))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            super::take_due_tasks_at(&cfg, minute(1, 9, 1))
                .unwrap()
                .len(),
            1
        );
        assert!(super::take_due_tasks_at(&cfg, minute(1, 8, 59))
            .unwrap()
            .is_empty());
        assert_eq!(
            load(&cfg).unwrap()[0].last_checked_minute,
            "2026-07-01 09:01"
        );
        assert!(super::take_due_tasks_at(&cfg, minute(1, 9, 0))
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 設定画面の予約更新は旧状態を移行して削除済みidを除く() {
        let old = serde_json::json!([{
            "id":"keep", "time":"09:00", "last_run_minute":"2026-07-28 09:00",
            "last_checked_minute":"2026-07-28 09:01"
        }, {"id":"remove", "time":"10:00", "last_run_minute":"2026-07-28 10:00"}]);
        let mut current = serde_json::Map::new();
        current.insert(CONFIG_KEY.into(), serde_json::json!(old.to_string()));
        let mut patch = serde_json::Map::new();
        patch.insert(
            CONFIG_KEY.into(),
            serde_json::Value::String(String::from(r#"[{"id":"keep","time":"11:00"}]"#)),
        );
        reconcile_settings_patch(&current, &mut patch).unwrap();
        let state = patch[STATE_KEY].as_str().unwrap();
        assert!(state.contains("keep"));
        assert!(!state.contains("remove"));
        assert!(!patch[CONFIG_KEY]
            .as_str()
            .unwrap()
            .contains("last_run_minute\":\"2026"));
    }

    fn cronタスク(cron: &str) -> ScheduledTask {
        let mut task = 固定時刻タスク("");
        task.cron = cron.to_string();
        task
    }

    fn minute(day: u32, hour: u32, minute: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(2026, 7, day, hour, minute, 0)
            .single()
            .expect("テスト日時")
    }

    #[test]
    fn 同じ分では二重に実行しない() {
        let mut task = 固定時刻タスク("07:05");
        assert!(should_run_at(&task, minute(28, 7, 5)));
        // 検出時に last_run_minute を刻む＝同じ分の再検出では拾わない。
        task.last_run_minute = "2026-07-28 07:05".to_string();
        assert!(!should_run_at(&task, minute(28, 7, 5)));
        // 分が変われば次の予定は通常どおり拾える。
        assert!(!should_run_at(&task, minute(28, 8, 5)));
        task.time = "08:05".to_string();
        assert!(should_run_at(&task, minute(28, 8, 5)));
    }

    #[test]
    fn 無効タスクは検出しない() {
        let mut task = 固定時刻タスク("07:05");
        task.enabled = false;
        assert!(!should_run_at(&task, minute(28, 7, 5)));
    }

    #[test]
    fn スリープ中の未処理分を上限付きで拾う() {
        let mut task = 固定時刻タスク("07:05");
        task.last_checked_minute = "2026-07-28 07:00".to_string();
        let minutes = minutes_to_check(&task, minute(28, 7, 10));
        assert_eq!(minutes.len(), 10);
        assert!(minutes.into_iter().any(|at| should_run_at(&task, at)));

        task.last_checked_minute = "2026-07-20 00:00".to_string();
        assert_eq!(minutes_to_check(&task, minute(28, 7, 10)).len(), 1441);
    }

    #[test]
    fn 時計巻き戻り時は過去予定を二重実行しない() {
        let mut task = 固定時刻タスク("07:05");
        task.last_checked_minute = "2026-07-28 08:00".to_string();
        assert!(minutes_to_check(&task, minute(28, 7, 10)).is_empty());
    }

    /// 物理削除は allowlist を通っても実行させない（run_command と同じ規則）。
    ///
    /// 以前この判定は run_command だけが持っていて、同じ allowlist で動くこちらには
    /// 無かった。許可コマンドに powershell があれば予約実行から Remove-Item を通せた。
    #[test]
    fn 許可済みでも物理削除コマンドは拒否する() {
        let cfg = crate::config::Config::new();
        // わざと「許可されている」状態にしてから、それでも拒否されることを確かめる。
        cfg.set("PICOAGENT_COMMAND_ALLOWLIST", "rm,powershell,git clean");

        let mut task = 固定時刻タスク("07:05");
        task.program = "rm".to_string();
        task.args = vec!["-rf".to_string(), "important".to_string()];
        let error = validate_task(&cfg, &task).unwrap_err();
        assert!(error.contains("ゴミ箱"), "error: {error}");

        task.program = "powershell".to_string();
        task.args = vec!["-Command".to_string(), "Remove-Item important".to_string()];
        assert!(validate_task(&cfg, &task).unwrap_err().contains("ゴミ箱"));

        task.program = "git".to_string();
        task.args = vec!["clean".to_string(), "-fd".to_string()];
        assert!(validate_task(&cfg, &task).unwrap_err().contains("ゴミ箱"));

        // 通常のコマンドはこれまでどおり通る。
        task.program = "git".to_string();
        task.args = vec!["status".to_string()];
        cfg.set("PICOAGENT_COMMAND_ALLOWLIST", "git status");
        assert!(validate_task(&cfg, &task).is_ok());
    }

    #[test]
    fn 無人実行するスキルは専用allowlistへ完全一致が必要() {
        let cfg = crate::config::Config::new();
        let mut task = 固定時刻タスク("07:05");
        task.kind = "tool".to_string();
        task.tool_name = "gmail_message".to_string();
        task.tool_args = serde_json::json!({"action": "send"});
        assert!(validate_task(&cfg, &task)
            .unwrap_err()
            .contains("許可されていないスキル"));

        cfg.set(super::TOOL_ALLOWLIST_KEY, "weather,gmail_message_preview");
        assert!(
            validate_task(&cfg, &task).is_err(),
            "prefix一致では許可しない"
        );
        cfg.set(super::TOOL_ALLOWLIST_KEY, "weather,gmail_message");
        assert!(validate_task(&cfg, &task).is_ok());
    }

    /// 検出は同期関数に固定する。ここを async にすると、長時間タスクの実行を
    /// 検出ループの中で `await` してしまう設計へ簡単に戻れてしまい、実行中は
    /// 他タスクの検出が止まる（＝その分の予定が恒久的にスキップされる）。
    #[test]
    fn 検出は実行を待たない構造である() {
        let source = include_str!("scheduler.rs");
        // 探索語をリテラルで書くと、この判定自身がソースに現れて空振りする。
        let async_marker = concat!("async ", "fn take_due_tasks_at(");
        assert!(
            source.contains("fn take_due_tasks_at(") && !source.contains(async_marker),
            "take_due_tasks は同期関数のままにしてください（実行は別ワーカーへ渡す）"
        );
        assert!(
            source.contains("tokio::sync::mpsc::channel::<Vec<ScheduledTask>>"),
            "実行は検出と切り離したキュー経由にしてください"
        );
    }

    #[test]
    fn 時刻と曜日を正規化する() {
        assert_eq!(normalize_time("7:05").as_deref(), Some("07:05"));
        assert_eq!(normalize_time("24:00"), None);
        assert_eq!(
            normalize_days(&["平日".to_string()]).unwrap(),
            vec!["mon", "tue", "wed", "thu", "fri"]
        );
        assert!(normalize_days(&["毎日".to_string()]).unwrap().is_empty());
    }

    /// 保存できたのに一度も発火しない予約を作らせないこと。
    #[test]
    fn 照合できないcronは保存前に断る() {
        assert!(super::validate_cron("*/5 * * * *").is_ok());
        assert!(super::validate_cron("0,30 9 * * 1").is_ok());
        assert!(super::validate_cron("* * * * *").is_ok());
        // 本物のcronにはある範囲指定。照合器は解釈しないので保存させない。
        let error = super::validate_cron("1-5 * * * *").expect_err("範囲指定を通した");
        assert!(error.contains("範囲指定"), "error: {error}");
        // 値域外・綴り誤り・空フィールドも、無言で不発になる前に断る。
        assert!(super::validate_cron("99 * * * *").is_err());
        assert!(super::validate_cron("* 25 * * *").is_err());
        assert!(super::validate_cron("* * * * 9").is_err());
        assert!(super::validate_cron("abc * * * *").is_err());
        assert!(super::validate_cron("*/0 * * * *").is_err());
        assert!(super::validate_cron("* * * *").is_err());
    }

    #[test]
    fn cron値を判定する() {
        assert!(parse_cron_value("*", 12, 0, 59));
        assert!(parse_cron_value("*/5", 10, 0, 59));
        assert!(!parse_cron_value("*/5", 11, 0, 59));
        assert!(parse_cron_value("1,3,5", 3, 0, 59));
    }

    #[test]
    fn cronの日と曜日は標準どおり両方指定時だけorで結ぶ() {
        // 2026-07-01は水曜、07-06は月曜、07-07は火曜。
        let both = cronタスク("0 9 1 * 1");
        assert!(should_run_at(&both, minute(1, 9, 0)), "毎月1日で発火");
        assert!(should_run_at(&both, minute(6, 9, 0)), "毎週月曜で発火");
        assert!(!should_run_at(&both, minute(7, 9, 0)));

        let day_only = cronタスク("0 9 1 * *");
        assert!(should_run_at(&day_only, minute(1, 9, 0)));
        assert!(!should_run_at(&day_only, minute(6, 9, 0)));

        let weekday_only = cronタスク("0 9 * * 1");
        assert!(!should_run_at(&weekday_only, minute(1, 9, 0)));
        assert!(should_run_at(&weekday_only, minute(6, 9, 0)));
    }
}
