// durable_jobs.rs - アプリ再起動後も再開できる、ツール単位の永続ジョブキュー。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

const STORE_NAME: &str = "durable-jobs.json";
const STORE_LOCK: &str = "durable-jobs";
const MAX_JOBS: usize = 200;
const MAX_RESULT_CHARS: usize = 12_000;
const MAX_ATTEMPTS: u32 = 5;
const JOB_TIMEOUT_SECS: u64 = 30 * 60;
const WORKER_KEY: &str = "__picoagent_durable_worker";
/// 同時に走らせるジョブの上限。
///
/// 1件ずつ順に処理すると、`JOB_TIMEOUT_SECS`（30分）いっぱい掛かりうる重いジョブ
/// （Web調査＋Office生成＋PDF変換）の後ろに並んだ短い転送が、その間ずっと始まらない。
/// 利用者には「ジョブへ登録しました」と返っているので、何も起きていないように見える。
///
/// 状態ファイルの更新は `update()` / `list()` がどちらも `store::lock_state` を取るため、
/// 同時に走らせても一覧の書き戻しや `unique_key` の重複判定は壊れない。上限を設けるのは、
/// 大きな転送が何本も同時にメモリと回線を取り合わないようにするため。
const MAX_CONCURRENT_JOBS: usize = 3;

/// 進捗・状態を画面へ送るためのハンドル。`start()` で一度だけ入る。
static APP: OnceLock<AppHandle> = OnceLock::new();
static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();
static SENDER: OnceLock<tokio::sync::mpsc::Sender<String>> = OnceLock::new();
static WORKER_TOKEN: OnceLock<String> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    WaitingAuth,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DurableJob {
    pub id: String,
    pub label: String,
    pub tool_name: String,
    #[serde(default)]
    pub unique_key: String,
    pub args: Value,
    pub state: JobState,
    pub progress: u8,
    /// いま何をしているか（「3/5 送信中」など）。旧データには無いので既定は空。
    #[serde(default)]
    pub step: String,
    pub attempts: u32,
    pub created_at: String,
    pub updated_at: String,
    pub result: String,
    pub error: String,
}

fn now() -> String {
    Local::now().to_rfc3339()
}

fn new_id() -> String {
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("job-{}-{sequence}", Local::now().format("%Y%m%d%H%M%S%3f"))
}

fn store_path() -> Result<&'static PathBuf, String> {
    STORE_PATH.get().ok_or_else(|| {
        "永続ジョブ機能がまだ起動していません。アプリを再起動してください。".to_string()
    })
}

fn load_from(path: &std::path::Path) -> Result<Vec<DurableJob>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| format!("永続ジョブ一覧を読み取れません: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("永続ジョブ一覧を読み取れません: {error}")),
    }
}

#[rustfmt::skip]
fn quarantine_job_store(path: &std::path::Path) -> Result<PathBuf, String> {
    crate::store::quarantine_corrupt(path).ok_or_else(|| "破損した永続ジョブ一覧を退避できません。".to_string())
}

pub fn list() -> Result<Vec<DurableJob>, String> {
    let _guard = crate::store::lock_state(STORE_LOCK);
    load_from(store_path()?)
}

fn update(mutator: impl FnOnce(&mut Vec<DurableJob>) -> Result<(), String>) -> Result<(), String> {
    let _guard = crate::store::lock_state(STORE_LOCK);
    let path = store_path()?;
    let mut jobs = load_from(path)?;
    mutator(&mut jobs)?;
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    while jobs.len() > MAX_JOBS {
        let Some(index) = jobs.iter().rposition(|job| {
            matches!(
                job.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        }) else {
            break;
        };
        jobs.remove(index);
    }
    crate::store::write_json_atomic(path, &jobs)
        .map_err(|error| format!("永続ジョブ一覧を保存できません: {error:#}"))
}

fn allowed_tool(name: &str) -> bool {
    matches!(name, "ai_drive" | "artifact_pipeline")
}

fn enqueue_inner(
    tool_name: &str,
    args: Value,
    label: String,
    unique_key: String,
) -> Result<String, String> {
    if !allowed_tool(tool_name) {
        return Err(format!("永続実行に対応していないツールです: {tool_name}"));
    }
    let id = new_id();
    let stamp = now();
    let job = DurableJob {
        id: id.clone(),
        label,
        tool_name: tool_name.to_string(),
        unique_key,
        args,
        state: JobState::Queued,
        progress: 0,
        step: String::new(),
        attempts: 0,
        created_at: stamp.clone(),
        updated_at: stamp,
        result: String::new(),
        error: String::new(),
    };
    let mut existing_id = None;
    update(|jobs| {
        if !job.unique_key.is_empty() {
            if let Some(existing) = jobs
                .iter()
                .find(|existing| existing.unique_key == job.unique_key)
            {
                existing_id = Some(existing.id.clone());
                return Ok(());
            }
        }
        jobs.push(job);
        Ok(())
    })?;
    if let Some(existing) = existing_id {
        return Ok(existing);
    }
    if let Err(error) = SENDER
        .get()
        .ok_or_else(|| "永続ジョブの実行ワーカーが起動していません。".to_string())?
        .try_send(id.clone())
    {
        let message = format!("永続ジョブを実行キューへ送れません: {error}");
        let _ = fail_without_execution(&id, &message);
        return Err(message);
    }
    Ok(id)
}

pub fn enqueue(tool_name: &str, args: Value, label: impl Into<String>) -> Result<String, String> {
    enqueue_inner(tool_name, args, label.into(), String::new())
}

pub fn enqueue_unique(
    tool_name: &str,
    args: Value,
    label: impl Into<String>,
    unique_key: String,
) -> Result<String, String> {
    enqueue_inner(tool_name, args, label.into(), unique_key)
}

fn fail_without_execution(id: &str, message: &str) -> Result<(), String> {
    update(|jobs| {
        let job = jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| format!("ジョブが見つかりません: {id}"))?;
        job.state = JobState::Failed;
        job.error = message.to_string();
        job.updated_at = now();
        Ok(())
    })
}

/// 利用者が明示した再試行で、終了済みジョブを待機中へ戻す。
fn requeue_for_retry(job: &mut DurableJob) -> Result<(), String> {
    if matches!(job.state, JobState::Running | JobState::Queued) {
        return Err("実行中または待機中のジョブは再試行できません。".to_string());
    }
    job.state = JobState::Queued;
    job.progress = 0;
    // 試行回数は「起動のたびに復旧して延々と再実行する」暴走を止めるための上限であって、
    // 利用者が原因を直してから明示的に押した再試行まで塞ぐためのものではない。
    // 数え直さないと、上限に達したジョブは再試行のたびに mark_started で即失敗し、
    // ジョブセンターから二度と実行できなくなる。
    job.attempts = 0;
    job.error.clear();
    job.result.clear();
    job.step.clear();
    job.updated_at = now();
    Ok(())
}

pub fn retry(id: &str) -> Result<(), String> {
    let mut requeued = None;
    update(|jobs| {
        let job = jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| format!("ジョブが見つかりません: {id}"))?;
        requeue_for_retry(job)?;
        requeued = Some(job.clone());
        Ok(())
    })?;
    if let Some(job) = requeued {
        notify(&job);
    }
    if let Err(error) = SENDER
        .get()
        .ok_or_else(|| "永続ジョブの実行ワーカーが起動していません。".to_string())?
        .try_send(id.to_string())
    {
        let message = format!("再試行を開始できません: {error}");
        let _ = fail_without_execution(id, &message);
        return Err(message);
    }
    Ok(())
}

pub fn cancel(id: &str) -> Result<(), String> {
    let mut changed = None;
    update(|jobs| {
        let job = jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| format!("ジョブが見つかりません: {id}"))?;
        if matches!(
            job.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return Err("完了済みのジョブはキャンセルできません。".to_string());
        }
        job.state = JobState::Cancelled;
        job.step = String::new();
        job.updated_at = now();
        changed = Some(job.clone());
        Ok(())
    })?;
    if let Some(job) = changed {
        notify(&job);
    }
    Ok(())
}

pub fn is_cancelled(id: &str) -> bool {
    list()
        .ok()
        .and_then(|jobs| jobs.into_iter().find(|job| job.id == id))
        .is_some_and(|job| job.state == JobState::Cancelled)
}

pub fn worker_job_id(args: &Value) -> Option<&str> {
    let expected = WORKER_TOKEN.get()?;
    if args.get(WORKER_KEY).and_then(Value::as_str) != Some(expected.as_str()) {
        return None;
    }
    args.get("__job_id").and_then(Value::as_str)
}

fn bounded(text: String) -> String {
    // 上限までの文字境界を1回の走査で求める。文字数を数えてから作り直すと、
    // 打ち切りの要らない普通の結果でも全文を2度走査することになる。
    match text.char_indices().nth(MAX_RESULT_CHARS) {
        Some((boundary, _)) => text[..boundary].to_string() + "\n…（保存上限で打ち切り）",
        None => text,
    }
}

/// ジョブ1件の現在地を画面へ送る。
///
/// 通知は「完了時に1回」では足りない。同時に3件走るようになったので、いま何が
/// どこまで進んでいるかが見えないと、利用者には無音のまま止まっているのと区別が付かない。
/// 受け側（`renderer.js`）は `jobId` で既存の項目を上書きするため、何度送ってよい。
fn notify(job: &DurableJob) {
    let Some(app) = APP.get() else {
        return;
    };
    let payload = json!({
        "kind": "durable_job",
        "jobId": job.id,
        "title": job.label,
        "body": job_body(job),
        "ok": !matches!(job.state, JobState::Failed | JobState::WaitingAuth),
        "state": job.state,
        "progress": job.progress,
        "at": job.updated_at,
    });
    if let Err(error) = app.emit("pet:notice", payload) {
        crate::log_paths::append_app_log(
            app,
            &format!("永続ジョブの進捗を画面へ通知できません: {error}"),
        );
    }
}

/// 通知へ出す1行。エラーや結果があればその先頭行、無ければ工程名。
fn job_body(job: &DurableJob) -> String {
    for text in [&job.error, &job.result] {
        if let Some(line) = text.lines().next().filter(|line| !line.is_empty()) {
            return line.to_string();
        }
    }
    match job.state {
        JobState::Queued => "順番待ちです。".to_string(),
        JobState::Cancelled => "キャンセルしました。".to_string(),
        _ if !job.step.is_empty() => job.step.clone(),
        _ => "処理中です。".to_string(),
    }
}

/// 画面へ通知するためのアプリハンドル。`start()` より前は `None`。
///
/// 永続ジョブ以外のスキルも、`AppHandle` を引数で受け取れない位置から
/// 利用者へ知らせたいことがある（生成のlowvram起動・OOM後の再試行など）。
pub(crate) fn app_handle() -> Option<&'static AppHandle> {
    APP.get()
}

/// スキルの実行中から現在地を知らせる。`percent` は 1〜99 に収める
/// （0 は未着手、100 は完了の意味を持たせているため、途中経過で使わせない）。
pub fn report_progress(job_id: &str, percent: u8, step: &str) {
    let mut updated = None;
    let outcome = update(|jobs| {
        if let Some(job) = jobs.iter_mut().find(|job| job.id == job_id) {
            if apply_progress(job, percent, step) {
                updated = Some(job.clone());
            }
        }
        Ok(())
    });
    match outcome {
        Ok(()) => {
            if let Some(job) = updated {
                notify(&job);
            }
        }
        // 進捗は補助情報。保存に失敗しても本処理は続ける（次の保存で書き直る）。
        // フォールバック許可: 進捗の書き込み失敗で転送・生成そのものを止めない。
        Err(error) => {
            if let Some(app) = APP.get() {
                crate::log_paths::append_app_log(
                    app,
                    &format!("永続ジョブの進捗を保存できません: {error}"),
                );
            }
        }
    }
}

/// 実行中のジョブにだけ進捗を反映する。反映したら true。
///
/// 終了済み・キャンセル済みを進捗で生き返らせない。転送ループは区切りごとに報告するので、
/// 中止した直後の1回が遅れて届くことがある。
fn apply_progress(job: &mut DurableJob, percent: u8, step: &str) -> bool {
    if job.state != JobState::Running {
        return false;
    }
    job.progress = percent.clamp(1, 99);
    job.step = step.to_string();
    job.updated_at = now();
    true
}

fn mark_started(id: &str) -> Result<Option<DurableJob>, String> {
    let mut selected = None;
    update(|jobs| {
        let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
            return Ok(());
        };
        if job.state != JobState::Queued {
            return Ok(());
        }
        if job.attempts >= MAX_ATTEMPTS {
            job.state = JobState::Failed;
            job.error = format!("再試行上限（{MAX_ATTEMPTS}回）に達しました。");
            job.updated_at = now();
            return Ok(());
        }
        job.state = JobState::Running;
        job.progress = 1;
        job.step = "開始しました".to_string();
        job.attempts += 1;
        job.updated_at = now();
        selected = Some(job.clone());
        Ok(())
    })?;
    Ok(selected)
}

fn mark_finished(id: &str, result: String) -> Result<DurableJob, String> {
    let auth_error = result.starts_with("エラー:")
        && (result.contains("認証更新が必要") || result.contains("HTTP 401"));
    let failed = result.starts_with("エラー:");
    let next = if auth_error {
        JobState::WaitingAuth
    } else if failed {
        JobState::Failed
    } else {
        JobState::Completed
    };
    let mut finished = None;
    update(|jobs| {
        let job = jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| format!("ジョブが見つかりません: {id}"))?;
        if job.state != JobState::Cancelled {
            job.state = next.clone();
            job.progress = if next == JobState::Completed {
                100
            } else {
                job.progress
            };
            job.step = String::new();
            if failed {
                job.error = bounded(result);
            } else {
                job.result = bounded(result);
            }
            job.updated_at = now();
        }
        finished = Some(job.clone());
        Ok(())
    })?;
    finished.ok_or_else(|| format!("ジョブが見つかりません: {id}"))
}

fn recover_jobs(jobs: &mut [DurableJob]) -> Vec<String> {
    let mut queued = Vec::new();
    for job in jobs {
        if job.state == JobState::Running {
            job.state = JobState::Queued;
            job.progress = 0;
            job.updated_at = now();
        }
        if job.state == JobState::Queued {
            queued.push(job.id.clone());
        }
    }
    queued
}

fn recover() -> Result<Vec<String>, String> {
    let mut queued = Vec::new();
    update(|jobs| {
        queued = recover_jobs(jobs);
        Ok(())
    })?;
    Ok(queued)
}

pub fn start(app: AppHandle) {
    let path = match app.path().app_data_dir() {
        Ok(dir) => dir.join(STORE_NAME),
        Err(error) => {
            crate::log_paths::append_app_log(
                &app,
                &format!("永続ジョブの保存先を取得できません: {error}"),
            );
            return;
        }
    };
    let _ = APP.set(app.clone());
    let _ = STORE_PATH.set(path.clone());
    let token = format!(
        "{}-{}",
        std::process::id(),
        Local::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let _ = WORKER_TOKEN.set(token);
    // フォールバック許可: ジョブ履歴の破損でアプリ全体を止めない。退避できた場合だけ空で継続する。
    if let Err(error) = load_from(&path) {
        let saved = match quarantine_job_store(&path) {
            Ok(saved) => saved,
            Err(quarantine_error) => {
                crate::log_paths::append_app_log(
                    &app,
                    &format!("{error}。{quarantine_error} 永続ジョブを開始しません。"),
                );
                return;
            }
        };
        crate::log_paths::append_app_log(
            &app,
            &format!(
                "{error}。破損ファイルを {} へ退避しました。",
                saved.display()
            ),
        );
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    if SENDER.set(tx.clone()).is_err() {
        return;
    }
    let pending = recover().unwrap_or_else(|error| {
        crate::log_paths::append_app_log(&app, &error);
        Vec::new()
    });
    for id in pending {
        if let Err(error) = tx.try_send(id.clone()) {
            let message = format!("復旧ジョブを実行キューへ送れません: {error}");
            let _ = fail_without_execution(&id, &message);
            crate::log_paths::append_app_log(&app, &message);
        }
    }
    // 取り出しは1本のループで行い、実行だけを `MAX_CONCURRENT_JOBS` まで並べる。
    // 上限に達している間は `acquire_owned()` で待つので、待ちのジョブはキューに残る。
    tauri::async_runtime::spawn(async move {
        let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_JOBS));
        while let Some(id) = rx.recv().await {
            let Ok(permit) = limit.clone().acquire_owned().await else {
                // セマフォが閉じるのは終了時だけ。取り出しを続ける意味が無いので抜ける。
                break;
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _permit = permit;
                run_job(&app, &id).await;
            });
        }
    });
}

/// 1件のジョブを実行し、結果を保存して画面へ知らせる。
async fn run_job(app: &AppHandle, id: &str) {
    let Ok(Some(job)) = mark_started(id) else {
        return;
    };
    // 完了まで無音にせず、走り始めた時点で画面へ出す。
    notify(&job);
    let mut args = job.args.clone();
    if let Some(object) = args.as_object_mut() {
        object.insert(
            WORKER_KEY.to_string(),
            Value::String(WORKER_TOKEN.get().cloned().unwrap_or_default()),
        );
        object.insert("__job_id".to_string(), Value::String(id.to_string()));
    }
    let child_app = app.clone();
    let tool_name = job.tool_name.clone();
    let mut execution = tauri::async_runtime::spawn(async move {
        let Some(state) = child_app.try_state::<crate::commands::AppState>() else {
            return "エラー: アプリ状態を取得できません。".to_string();
        };
        crate::tools::dispatch_final(&state.config, &tool_name, args).await
    });
    let result = tokio::select! {
        joined = &mut execution => match joined {
            Ok(result) => result,
            Err(error) => format!("エラー: 永続ジョブの実行タスクが異常終了しました: {error}"),
        },
        _ = tokio::time::sleep(std::time::Duration::from_secs(JOB_TIMEOUT_SECS)) => {
            execution.abort();
            format!("エラー: 永続ジョブが{}分でタイムアウトしました。", JOB_TIMEOUT_SECS / 60)
        }
    };
    match mark_finished(id, result) {
        Ok(finished) => notify(&finished),
        Err(error) => crate::log_paths::append_app_log(
            app,
            &format!("永続ジョブの結果を保存できません: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 実行中ジョブを再起動待ちへ戻す() {
        let job = DurableJob {
            id: "x".into(),
            label: "x".into(),
            tool_name: "ai_drive".into(),
            unique_key: String::new(),
            args: json!({}),
            state: JobState::Running,
            progress: 50,
            step: String::new(),
            attempts: 1,
            created_at: now(),
            updated_at: now(),
            result: String::new(),
            error: String::new(),
        };
        let completed = DurableJob {
            id: "done".into(),
            state: JobState::Completed,
            ..job.clone()
        };
        let mut jobs = vec![job, completed];
        assert_eq!(recover_jobs(&mut jobs), vec!["x"]);
        assert_eq!(jobs[0].state, JobState::Queued);
        assert_eq!(jobs[0].progress, 0);
        assert_eq!(jobs[1].state, JobState::Completed);
    }

    fn 見本(state: JobState, attempts: u32) -> DurableJob {
        DurableJob {
            id: "x".into(),
            label: "x".into(),
            tool_name: "ai_drive".into(),
            unique_key: String::new(),
            args: json!({}),
            state,
            progress: 40,
            step: "送信中".into(),
            attempts,
            created_at: now(),
            updated_at: now(),
            result: String::new(),
            error: "前回の失敗".into(),
        }
    }

    #[test]
    fn 明示した再試行は試行回数を数え直す() {
        let mut job = 見本(JobState::Failed, MAX_ATTEMPTS);
        requeue_for_retry(&mut job).unwrap();
        assert_eq!(job.state, JobState::Queued);
        // ここが 0 に戻らないと、mark_started が上限超過で即失敗させるため再試行できない。
        assert_eq!(job.attempts, 0);
        assert_eq!(job.progress, 0);
        assert!(job.error.is_empty());

        for state in [JobState::Running, JobState::Queued] {
            assert!(requeue_for_retry(&mut 見本(state, 0)).is_err());
        }
    }

    #[test]
    fn 進捗は実行中のジョブにだけ入り両端を避ける() {
        let mut running = 見本(JobState::Running, 1);
        assert!(apply_progress(&mut running, 55, "送信中 2/4"));
        assert_eq!(running.progress, 55);
        assert_eq!(running.step, "送信中 2/4");

        // 0 は未着手、100 は完了の意味なので途中経過では使わせない。
        assert!(apply_progress(&mut running, 0, "開始直後"));
        assert_eq!(running.progress, 1);
        assert!(apply_progress(&mut running, 100, "ほぼ完了"));
        assert_eq!(running.progress, 99);

        // 中止したジョブへ遅れて届いた進捗で実行中へ戻さない。
        for state in [
            JobState::Cancelled,
            JobState::Completed,
            JobState::Failed,
            JobState::Queued,
        ] {
            let mut other = 見本(state.clone(), 1);
            assert!(!apply_progress(&mut other, 50, "遅れて到着"), "{state:?}");
            assert_eq!(other.state, state);
        }
    }

    #[test]
    fn 通知本文は結果と工程を優先して1行で出す() {
        let mut job = 見本(JobState::Running, 1);
        job.error.clear();
        job.step = "PDFへ変換中".into();
        assert_eq!(job_body(&job), "PDFへ変換中");

        job.result = "成果物を生成しました: C:\\work\n- report.html".into();
        assert_eq!(job_body(&job), "成果物を生成しました: C:\\work");

        job.error = "エラー: 認証更新が必要です\n詳細".into();
        assert_eq!(job_body(&job), "エラー: 認証更新が必要です");

        let mut queued = 見本(JobState::Queued, 0);
        queued.error.clear();
        queued.step.clear();
        assert_eq!(job_body(&queued), "順番待ちです。");
    }

    #[test]
    fn 保存上限を超えた結果だけを文字境界で打ち切る() {
        let short = "あ".repeat(10);
        assert_eq!(bounded(short.clone()), short);
        let long = "あ".repeat(MAX_RESULT_CHARS + 5);
        let cut = bounded(long);
        assert!(cut.ends_with("\n…（保存上限で打ち切り）"));
        assert_eq!(
            cut.chars().filter(|ch| *ch == 'あ').count(),
            MAX_RESULT_CHARS
        );
    }
}
