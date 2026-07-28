// scheduler.rs - 指定時刻に許可済みコマンドを実行する常駐タスク。
//
// タスク一覧は設定キー `PICOAGENT_SCHEDULED_TASKS` に JSON 文字列で保存する。実行は
// run_command と同じ許可ルート・allowlist・有効化フラグを使い、shell は使わない。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Command;

use crate::config::Config;

pub const CONFIG_KEY: &str = "PICOAGENT_SCHEDULED_TASKS";
const DEFAULT_ALLOWLIST: &str = "git status,git diff,npm test,cargo test";
const MAX_ARGS: usize = 32;
const MAX_ARG_CHARS: usize = 500;
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_RUN_HISTORY: usize = 100;

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

const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

// 実行の許可ルートは全スキル共通の permissions を参照する。
fn command_root(cfg: &Config) -> PathBuf {
    crate::permissions::primary_root(cfg)
}

fn command_roots(cfg: &Config) -> Vec<PathBuf> {
    crate::permissions::allowed_roots(cfg)
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

fn day_codes(token: &str) -> Vec<&'static str> {
    match token.trim().to_lowercase().as_str() {
        "everyday" | "daily" | "all" | "毎日" | "まいにち" => WEEKDAYS.to_vec(),
        "weekday" | "weekdays" | "平日" => vec!["mon", "tue", "wed", "thu", "fri"],
        "weekend" | "weekends" | "週末" => vec!["sat", "sun"],
        "mon" | "monday" | "月" | "月曜" | "月曜日" => vec!["mon"],
        "tue" | "tuesday" | "火" | "火曜" | "火曜日" => vec!["tue"],
        "wed" | "wednesday" | "水" | "水曜" | "水曜日" => vec!["wed"],
        "thu" | "thursday" | "木" | "木曜" | "木曜日" => vec!["thu"],
        "fri" | "friday" | "金" | "金曜" | "金曜日" => vec!["fri"],
        "sat" | "saturday" | "土" | "土曜" | "土曜日" => vec!["sat"],
        "sun" | "sunday" | "日" | "日曜" | "日曜日" => vec!["sun"],
        _ => vec![],
    }
}

pub fn normalize_days(input: &[String]) -> Vec<String> {
    let mut set: Vec<&'static str> = Vec::new();
    let mut everyday = false;
    for raw in input {
        let codes = day_codes(raw);
        if codes.len() == 7 {
            everyday = true;
        }
        for code in codes {
            if !set.contains(&code) {
                set.push(code);
            }
        }
    }
    if everyday {
        return Vec::new();
    }
    WEEKDAYS
        .iter()
        .filter(|d| set.contains(d))
        .map(|d| d.to_string())
        .collect()
}

pub fn normalize_time(input: &str) -> Option<String> {
    let (h, m) = input.trim().split_once(':')?;
    let hour: u32 = h.trim().parse().ok()?;
    let minute: u32 = m.trim().parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{hour:02}:{minute:02}"))
}

pub fn days_label(days: &[String]) -> String {
    if days.is_empty() {
        return "毎日".to_string();
    }
    let jp = |code: &str| match code {
        "mon" => "月",
        "tue" => "火",
        "wed" => "水",
        "thu" => "木",
        "fri" => "金",
        "sat" => "土",
        "sun" => "日",
        _ => "?",
    };
    WEEKDAYS
        .iter()
        .filter(|d| days.iter().any(|x| x == *d))
        .map(|d| jp(d))
        .collect::<Vec<_>>()
        .join("・")
}

pub fn load(cfg: &Config) -> Vec<ScheduledTask> {
    let raw = cfg.get(CONFIG_KEY, "");
    if raw.trim().is_empty() {
        return Vec::new();
    }
    // フォールバック許可: 設定 JSON が壊れていてもアプリ起動を優先し、空一覧で扱う。
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save(cfg: &Config, tasks: &[ScheduledTask]) -> anyhow::Result<()> {
    let mut patch = Map::new();
    patch.insert(
        CONFIG_KEY.to_string(),
        Value::String(serde_json::to_string(tasks)?),
    );
    cfg.set_all(patch)
}

fn run_history_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("schedule-runs.json"))
}

pub fn load_run_history(app: &AppHandle) -> Vec<ScheduledRunRecord> {
    let Some(path) = run_history_path(app) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    // フォールバック許可: 実行履歴の破損でアプリ本体やスケジューラを止めない。
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_run_history(app: &AppHandle, records: &[ScheduledRunRecord]) -> anyhow::Result<()> {
    let Some(path) = run_history_path(app) else {
        anyhow::bail!("app_data_dir を取得できませんでした");
    };
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
    let mut records = load_run_history(app);
    records.insert(0, record.clone());
    records.truncate(MAX_RUN_HISTORY);
    if let Err(error) = save_run_history(app, &records) {
        crate::log_paths::append_app_log(
            app,
            &format!("スケジュール実行履歴の保存に失敗しました: {error}"),
        );
    }
    record
}

fn emit_run_notice(app: &AppHandle, record: &ScheduledRunRecord) {
    let _ = app.emit(
        "pet:notice",
        json!({
            "kind": "schedule",
            "title": record.title,
            "body": record.body,
            "ok": record.ok,
            "taskId": record.task_id,
            "runId": record.id,
            "at": record.at,
        }),
    );
}

fn normalize(base: &Path, input: &str) -> PathBuf {
    let joined = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        base.join(input)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => {
                out.pop();
            }
            CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn bounded_cwd(base: &Path, roots: Vec<PathBuf>, cwd: &str) -> Result<PathBuf, String> {
    let target = normalize(base, if cwd.trim().is_empty() { "." } else { cwd });
    for root in roots {
        let root = normalize(&root, ".");
        if target == root || target.starts_with(&root) {
            return Ok(target);
        }
    }
    Err(format!(
        "許可ルートの外では実行できません: {}",
        target.display()
    ))
}

fn is_protected_edit_path(path: &Path) -> bool {
    crate::permissions::is_system_protected(path)
}

fn command_tokens(program: &str, args: &[String]) -> Vec<String> {
    let mut tokens = Vec::with_capacity(args.len() + 1);
    tokens.push(program.to_string());
    tokens.extend(args.iter().cloned());
    tokens
}

fn allowlist_entries(cfg: &Config) -> Vec<Vec<String>> {
    cfg.get("PICOAGENT_COMMAND_ALLOWLIST", DEFAULT_ALLOWLIST)
        .split(',')
        .filter_map(|entry| {
            let tokens: Vec<String> = entry
                .split_whitespace()
                .filter(|token| !token.is_empty())
                .map(ToString::to_string)
                .collect();
            if tokens.is_empty() {
                None
            } else {
                Some(tokens)
            }
        })
        .collect()
}

fn allowed(cfg: &Config, program: &str, args: &[String]) -> bool {
    let tokens = command_tokens(program, args);
    allowlist_entries(cfg).iter().any(|entry| {
        entry.len() <= tokens.len()
            && entry
                .iter()
                .zip(tokens.iter())
                .all(|(allowed, actual)| allowed == actual)
    })
}

fn validate_args(args: &[String]) -> Result<(), String> {
    if args.len() > MAX_ARGS {
        return Err(format!("args は {} 個以内で指定してください。", MAX_ARGS));
    }
    for arg in args {
        if arg.chars().count() > MAX_ARG_CHARS {
            return Err(format!(
                "各 arg は {} 文字以内で指定してください。",
                MAX_ARG_CHARS
            ));
        }
    }
    Ok(())
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
            validate_args(&task.args)?;
            if !allowed(cfg, &task.program, &task.args) {
                return Err(format!(
                    "許可されていないコマンドです: {} {}",
                    task.program,
                    task.args.join(" ")
                ));
            }
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
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= MAX_OUTPUT_BYTES {
        return format!("{}:\n{}", label, text);
    }
    format!(
        "{}:\n{}\n…（{} バイトで打ち切り）",
        label,
        crate::text::truncate_bytes(&text, MAX_OUTPUT_BYTES),
        MAX_OUTPUT_BYTES
    )
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

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
    let base = command_root(cfg);
    let cwd = match bounded_cwd(&base, command_roots(cfg), &task.cwd) {
        Ok(cwd) => cwd,
        Err(error) => return format!("エラー: {error}"),
    };
    if is_protected_edit_path(&cwd) {
        return format!(
            "エラー: Windows / Program Files 配下は編集不可のため、コマンド実行先にできません: {}",
            cwd.display()
        );
    }
    let default_timeout = cfg
        .get("PICOAGENT_COMMAND_TIMEOUT_SECS", "30")
        .parse::<u64>()
        .ok()
        .filter(|value| (1..=120).contains(value))
        .unwrap_or(30);
    let timeout_secs = task
        .timeout_secs
        .filter(|value| (1..=120).contains(value))
        .unwrap_or(default_timeout);

    let mut command = Command::new(&task.program);
    command
        .args(&task.args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut command);

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
    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), command.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => return format!("エラー: コマンド起動に失敗しました（{}）", error),
            Err(_) => {
                return format!(
                    "エラー: コマンドが {} 秒でタイムアウトしました。",
                    timeout_secs
                )
            }
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

fn current_weekday_code() -> &'static str {
    match Local::now().weekday().num_days_from_monday() {
        0 => "mon",
        1 => "tue",
        2 => "wed",
        3 => "thu",
        4 => "fri",
        5 => "sat",
        _ => "sun",
    }
}

fn matches_fixed_time(task: &ScheduledTask, minute_key: &str) -> bool {
    let now = Local::now();
    if !task.date.trim().is_empty() && task.date != now.format("%Y-%m-%d").to_string() {
        return false;
    }
    if !task.days.is_empty() && !task.days.iter().any(|day| day == current_weekday_code()) {
        return false;
    }
    task.time == minute_key
}

fn parse_cron_value(part: &str, value: u32, min: u32, max: u32) -> bool {
    part.split(',').any(|raw| {
        let token = raw.trim();
        if token == "*" {
            return true;
        }
        if let Some(step) = token.strip_prefix("*/").and_then(|s| s.parse::<u32>().ok()) {
            return step > 0 && value >= min && value <= max && (value - min) % step == 0;
        }
        token
            .parse::<u32>()
            .map(|want| want == value && want >= min && want <= max)
            .unwrap_or(false)
    })
}

fn cron_weekday_value() -> u32 {
    // cron 互換で日曜=0、月曜=1。
    Local::now().weekday().num_days_from_sunday()
}

fn validate_cron(cron: &str) -> Result<(), String> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return Err("cron は「分 時 日 月 曜日」の5フィールドで指定してください。".to_string());
    }
    Ok(())
}

fn matches_cron(task: &ScheduledTask) -> bool {
    let parts: Vec<&str> = task.cron.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }
    let now = Local::now();
    parse_cron_value(parts[0], now.minute(), 0, 59)
        && parse_cron_value(parts[1], now.hour(), 0, 23)
        && parse_cron_value(parts[2], now.day(), 1, 31)
        && parse_cron_value(parts[3], now.month(), 1, 12)
        && parse_cron_value(parts[4], cron_weekday_value(), 0, 6)
}

fn should_run(task: &ScheduledTask, minute_stamp: &str, hhmm: &str) -> bool {
    task.enabled
        && task.last_run_minute != minute_stamp
        && if task.cron.trim().is_empty() {
            matches_fixed_time(task, hhmm)
        } else {
            matches_cron(task)
        }
}

/// 実行すべきタスクを取り出し、二重起動しないよう `last_run_minute` を先に保存する。
///
/// 実行そのものはここで待たない。文字起こしのように 30 分かかりうるタスクを
/// 検出ループの中で `await` すると、その間の tick が進まず、当該時間帯に予定
/// されていた他タスクが（次の分に繰り越されないため）恒久的にスキップされる。
fn take_due_tasks(app: &AppHandle, cfg: &Config) -> Vec<ScheduledTask> {
    let now = Local::now();
    let minute_stamp = now.format("%Y-%m-%d %H:%M").to_string();
    let hhmm = now.format("%H:%M").to_string();
    let mut tasks = load(cfg);
    let mut changed = false;
    let due: Vec<ScheduledTask> = tasks
        .iter_mut()
        .filter_map(|task| {
            if should_run(task, &minute_stamp, &hhmm) {
                task.last_run_minute = minute_stamp.clone();
                if !task.repeat {
                    task.enabled = false;
                }
                changed = true;
                Some(task.clone())
            } else {
                None
            }
        })
        .collect();
    if changed {
        if let Err(error) = save(cfg, &tasks) {
            crate::log_paths::append_app_log(
                app,
                &format!("スケジュールタスク状態の保存に失敗しました: {error}"),
            );
        }
    }
    due
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
    let task = load(cfg)
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

pub fn start(app: AppHandle) {
    // 検出（tick）と実行（ワーカー）を分ける。実行は 1 本のワーカーで直列に処理するので、
    // 長時間タスクがあっても検出は止まらず、実行履歴の書き込みも競合しない。
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ScheduledTask>(64);

    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(task) = rx.recv().await {
            let Some(state) = worker_app.try_state::<crate::commands::AppState>() else {
                continue;
            };
            run_and_record(&worker_app, &state.config, task).await;
        }
    });

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let Some(state) = app.try_state::<crate::commands::AppState>() else {
                continue;
            };
            for task in take_due_tasks(&app, &state.config) {
                // キューが詰まっている＝前のタスクが長引いている。取りこぼしを黙って
                // 捨てず、ログに残してから次の機会に委ねる。
                if let Err(error) = tx.try_send(task) {
                    crate::log_paths::append_app_log(
                        &app,
                        &format!("スケジュール実行キューが埋まっているため見送りました: {error}"),
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{normalize_days, normalize_time, parse_cron_value, should_run, ScheduledTask};

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
        }
    }

    #[test]
    fn 同じ分では二重に実行しない() {
        let mut task = 固定時刻タスク("07:05");
        assert!(should_run(&task, "2026-07-28 07:05", "07:05"));
        // 検出時に last_run_minute を刻む＝同じ分の再検出では拾わない。
        task.last_run_minute = "2026-07-28 07:05".to_string();
        assert!(!should_run(&task, "2026-07-28 07:05", "07:05"));
        // 分が変われば次の予定は通常どおり拾える。
        assert!(!should_run(&task, "2026-07-28 08:05", "08:05"));
        task.time = "08:05".to_string();
        assert!(should_run(&task, "2026-07-28 08:05", "08:05"));
    }

    #[test]
    fn 無効タスクは検出しない() {
        let mut task = 固定時刻タスク("07:05");
        task.enabled = false;
        assert!(!should_run(&task, "2026-07-28 07:05", "07:05"));
    }

    /// 検出は同期関数に固定する。ここを async にすると、長時間タスクの実行を
    /// 検出ループの中で `await` してしまう設計へ簡単に戻れてしまい、実行中は
    /// 他タスクの検出が止まる（＝その分の予定が恒久的にスキップされる）。
    #[test]
    fn 検出は実行を待たない構造である() {
        let source = include_str!("scheduler.rs");
        // 探索語をリテラルで書くと、この判定自身がソースに現れて空振りする。
        let async_marker = concat!("async ", "fn take_due_tasks(");
        assert!(
            source.contains("fn take_due_tasks(") && !source.contains(async_marker),
            "take_due_tasks は同期関数のままにしてください（実行は別ワーカーへ渡す）"
        );
        assert!(
            source.contains("tokio::sync::mpsc::channel::<ScheduledTask>"),
            "実行は検出と切り離したキュー経由にしてください"
        );
    }

    #[test]
    fn 時刻と曜日を正規化する() {
        assert_eq!(normalize_time("7:05").as_deref(), Some("07:05"));
        assert_eq!(normalize_time("24:00"), None);
        assert_eq!(
            normalize_days(&["平日".to_string()]),
            vec!["mon", "tue", "wed", "thu", "fri"]
        );
        assert!(normalize_days(&["毎日".to_string()]).is_empty());
    }

    #[test]
    fn cron値を判定する() {
        assert!(parse_cron_value("*", 12, 0, 59));
        assert!(parse_cron_value("*/5", 10, 0, 59));
        assert!(!parse_cron_value("*/5", 11, 0, 59));
        assert!(parse_cron_value("1,3,5", 3, 0, 59));
    }
}
