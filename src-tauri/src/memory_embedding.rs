// memory_embedding.rs - 長期メモリ専用の埋め込みプロセスを再利用する。
// Pythonが所有するモデルと索引、Rustが所有する原本・通信・終了処理を分ける。

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

use crate::config::Config;

static WORKER: tokio::sync::Mutex<Option<Worker>> = tokio::sync::Mutex::const_new(None);
static CANCEL: tokio::sync::Notify = tokio::sync::Notify::const_new();
static EXITING: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static FAILURE: Mutex<String> = Mutex::new(String::new());
static STATUS: Mutex<Option<Value>> = Mutex::new(None);
static CLEANUP: Mutex<Vec<tauri::async_runtime::JoinHandle<Result<()>>>> = Mutex::new(Vec::new());

struct Worker {
    child: Option<Child>,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    requested: String,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // 通信futureが中断された場合も、応答行を次の質問へ混ぜずプロセスを回収する。
        if let Some(mut child) = self.child.take() {
            let task = tauri::async_runtime::spawn(async move {
                if child.try_wait()?.is_none() {
                    crate::tools::process::terminate_process_tree(&mut child, "記憶検索")
                        .await
                        .map_err(anyhow::Error::msg)?;
                }
                Ok(())
            });
            CLEANUP.lock().unwrap().push(task);
        }
    }
}

impl Worker {
    async fn close(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            if child.try_wait()?.is_none() {
                crate::tools::process::terminate_process_tree(&mut child, "記憶検索")
                    .await
                    .map_err(anyhow::Error::msg)?;
            }
            child
                .wait()
                .await
                .context("記憶検索プロセスを回収できません")?;
        }
        Ok(())
    }

    async fn response(&mut self) -> Result<Value> {
        // 原本上限200件×768次元のスコアではなく、順位とスコアだけを受け取る。
        let line = self
            .output
            .next_line()
            .await?
            .context("記憶検索プロセスが応答前に終了しました")?;
        anyhow::ensure!(line.len() <= 64 * 1024, "記憶検索の応答が大きすぎます");
        let result: Value = serde_json::from_str(&line).context("記憶検索の応答形式が不正です")?;
        if let Some(error) = result.get("error").and_then(Value::as_str) {
            anyhow::bail!("{error}");
        }
        Ok(result)
    }
}

pub fn status() -> Value {
    STATUS
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| json!({"phase":"idle"}))
}

fn set_status(phase: &str, message: &str, device: &str) {
    *STATUS.lock().unwrap() = Some(json!({"phase":phase,"message":message,"device":device}));
}

fn notify(app: &AppHandle, error: Option<&str>) {
    let mut previous = FAILURE.lock().unwrap();
    let next = error.unwrap_or_default();
    if previous.as_str() == next {
        return;
    }
    *previous = next.to_string();
    let payload = json!({
        "kind":"background_failure", "key":"memory-embedding", "title":"長期メモリ検索",
        "body":error.unwrap_or("長期メモリ検索が復旧しました"), "ok":error.is_none(),
        "at":chrono::Local::now().to_rfc3339(),
    });
    if let Err(failure) = app.emit("pet:notice", payload) {
        crate::log_paths::append_app_log(
            app,
            &format!("記憶検索の状態を通知できません: {failure}"),
        );
    }
}

fn device(cfg: &Config) -> Result<String> {
    let value = cfg.get("PICOAGENT_MEMORY_DEVICE", "gpu");
    anyhow::ensure!(
        matches!(value.as_str(), "gpu" | "cpu"),
        "記憶検索の実行先はgpuまたはcpuを選んでください"
    );
    Ok(value)
}

async fn launch(app: &AppHandle, requested: &str) -> Result<Worker> {
    set_status(
        "prepare",
        "長期メモリ検索の実行環境とモデルを準備しています",
        requested,
    );
    let python = crate::memory_embedding_install::ensure(app, requested).await?;
    let root = crate::log_paths::bundled_dir(app).join("embedding-memory");
    let script = root.join("worker.py");
    crate::store::write_atomic(
        &script,
        include_str!("../resources/embedding-memory/worker.py").as_bytes(),
    )?;
    // 診断ログは日付付きでログフォルダへ置き、保持期間の自動削除と診断情報の対象にする。
    crate::log_paths::ensure_log_dir(app)?;
    let stderr = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::log_paths::log_file(app, "embedding-memory.log"))?;
    let mut command = crate::tools::process::command(python);
    command
        .arg("-u")
        .arg(&script)
        .arg(&root)
        .arg(requested)
        .arg(crate::log_paths::data_dir(app).join("memory-vectors.json"))
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stderr(Stdio::from(stderr));
    let mut child = command
        .spawn()
        .context("記憶検索プロセスを起動できません")?;
    let input = child.stdin.take().context("記憶検索の入力を開けません")?;
    let output = BufReader::new(child.stdout.take().context("記憶検索の出力を開けません")?).lines();
    let mut worker = Worker {
        child: Some(child),
        input,
        output,
        requested: requested.to_string(),
    };
    let ready = tokio::time::timeout(Duration::from_secs(300), worker.response())
        .await
        .context("記憶検索モデルの起動が時間切れになりました")??;
    anyhow::ensure!(
        ready.get("ready") == Some(&Value::Bool(true)),
        "記憶検索モデルの準備が完了しませんでした"
    );
    let actual = ready
        .get("device")
        .and_then(Value::as_str)
        .context("記憶検索の実行先が応答にありません")?;
    set_status("ready", "長期メモリ検索を利用できます", actual);
    Ok(worker)
}

async fn run(
    app: &AppHandle,
    cfg: &Config,
    query: Option<(&[String], &str, u64)>,
) -> Result<Vec<usize>> {
    anyhow::ensure!(
        !EXITING.load(Ordering::SeqCst),
        "終了中のため記憶検索を開始できません"
    );
    let generation = query
        .map(|(_, _, generation)| generation)
        .unwrap_or_else(|| GENERATION.load(Ordering::SeqCst));
    let mut slot = WORKER.lock().await;
    drain_cleanup().await?;
    let cancellation = CANCEL.notified();
    tokio::pin!(cancellation);
    // Notifyの登録を世代・終了の確認より先に済ませる。確認後に登録すると、その間の
    // pause()の通知を取りこぼし、導入・起動が終わるまで設定保存や終了を待たせる。
    cancellation.as_mut().enable();
    anyhow::ensure!(
        generation == GENERATION.load(Ordering::SeqCst),
        "記憶検索の設定または原本が変わったため中断しました"
    );
    anyhow::ensure!(
        !EXITING.load(Ordering::SeqCst),
        "終了中のため記憶検索を開始できません"
    );
    anyhow::ensure!(
        cfg.get_bool("PICOAGENT_MEMORY_ENABLED", true),
        "長期メモリが無効になったため検索を中断しました"
    );
    let requested = device(cfg)?;
    let operation = async {
        let mut worker = match slot.take() {
            Some(mut worker) if worker.requested != requested => {
                worker.close().await?;
                launch(app, &requested).await?
            }
            Some(worker) => worker,
            None => launch(app, &requested).await?,
        };
        let order = if let Some((documents, text, _)) = query {
            let payload = serde_json::to_vec(&json!({"documents":documents,"query":text}))?;
            let result = tokio::time::timeout(Duration::from_secs(60), async {
                worker.input.write_all(&payload).await?;
                worker.input.write_all(b"\n").await?;
                worker.input.flush().await?;
                worker.response().await
            })
            .await
            .context("記憶検索が時間切れになりました")??;
            validate_order(&result, documents.len())?
        } else {
            Vec::new()
        };
        *slot = Some(worker);
        anyhow::ensure!(
            generation == GENERATION.load(Ordering::SeqCst),
            "記憶検索の設定または原本が変わったため中断しました"
        );
        Ok(order)
    };
    tokio::select! {
        result = operation => result,
        _ = &mut cancellation => anyhow::bail!("記憶検索を中断しました"),
    }
}

fn validate_order(result: &Value, count: usize) -> Result<Vec<usize>> {
    let indices = result
        .get("indices")
        .and_then(Value::as_array)
        .context("記憶検索の順位がありません")?;
    let scores = result
        .get("scores")
        .and_then(Value::as_array)
        .context("記憶検索のスコアがありません")?;
    anyhow::ensure!(
        indices.len() == count && scores.len() == count,
        "記憶検索の件数が一致しません"
    );
    let mut seen = vec![false; count];
    let mut order = Vec::with_capacity(count);
    for index in indices {
        let index = index
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .context("記憶検索の順位が不正です")?;
        anyhow::ensure!(
            index < count && !seen[index],
            "記憶検索の順位が重複または範囲外です"
        );
        seen[index] = true;
        order.push(index);
    }
    anyhow::ensure!(
        scores.iter().all(|s| s
            .as_f64()
            .is_some_and(|s| s.is_finite() && (-1.01..=1.01).contains(&s))),
        "記憶検索のスコアが不正です"
    );
    for pair in order.windows(2) {
        let before = scores[pair[0]]
            .as_f64()
            .context("記憶検索のスコアが不正です")?;
        let after = scores[pair[1]]
            .as_f64()
            .context("記憶検索のスコアが不正です")?;
        anyhow::ensure!(
            before >= after && (before != after || pair[0] < pair[1]),
            "記憶検索の順位とスコアが一致しません"
        );
    }
    Ok(order)
}

pub async fn search(
    app: &AppHandle,
    cfg: &Config,
    docs: &[String],
    query: &str,
    generation: u64,
) -> Result<Vec<usize>> {
    let result = run(app, cfg, Some((docs, query, generation))).await;
    match &result {
        Ok(_) => notify(app, None),
        Err(error) => {
            if generation != GENERATION.load(Ordering::SeqCst) || EXITING.load(Ordering::SeqCst) {
                return result;
            }
            let message = format!("長期メモリ検索に失敗しました: {error:#}");
            set_status("error", &message, "");
            notify(app, Some(&message));
            crate::log_paths::append_app_log(app, &message);
        }
    }
    result
}

pub fn prepare(app: AppHandle) {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        let generation = GENERATION.load(Ordering::SeqCst);
        let state = app.state::<crate::commands::AppState>();
        if state.config.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
            let result = run(&app, &state.config, None).await;
            if result.is_ok() {
                // 以前の準備失敗を知らせた後なら復旧を伝え、次の同じ失敗も再通知できるようにする。
                notify(&app, None);
            }
            if let Err(error) = result {
                if generation != GENERATION.load(Ordering::SeqCst) || EXITING.load(Ordering::SeqCst)
                {
                    return;
                }
                let message = format!("長期メモリ検索を準備できません: {error:#}");
                set_status("error", &message, "");
                notify(&app, Some(&message));
                crate::log_paths::append_app_log(&app, &message);
            }
        }
    });
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

pub struct PauseGuard {
    _slot: tokio::sync::MutexGuard<'static, Option<Worker>>,
}

impl Drop for PauseGuard {
    fn drop(&mut self) {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        set_status("idle", "", "");
    }
}

pub async fn pause() -> Result<PauseGuard> {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    CANCEL.notify_waiters();
    let mut slot = WORKER.lock().await;
    drain_cleanup().await?;
    if let Some(mut worker) = slot.take() {
        worker.close().await?;
    }
    Ok(PauseGuard { _slot: slot })
}

pub async fn reset_for_edit<T>(edit: impl FnOnce() -> Result<T>) -> Result<T> {
    let _pause = pause().await?;
    edit()
}

async fn drain_cleanup() -> Result<()> {
    let tasks = std::mem::take(&mut *CLEANUP.lock().unwrap());
    for task in tasks {
        task.await.context("記憶検索の回収処理が中断しました")??;
    }
    Ok(())
}

pub async fn reset() -> Result<()> {
    reset_for_edit(|| Ok(())).await
}

pub fn stop(app: &AppHandle) {
    EXITING.store(true, Ordering::SeqCst);
    if let Err(error) = tauri::async_runtime::block_on(reset()) {
        crate::log_paths::append_app_log(app, &format!("記憶検索を終了できません: {error:#}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 記憶原本と一致する順位だけを受け付ける() {
        assert_eq!(
            validate_order(&json!({"indices":[1,0],"scores":[0.3,0.7]}), 2).unwrap(),
            vec![1, 0]
        );
        for value in [
            json!({"indices":[0,0],"scores":[0.3,0.7]}),
            json!({"indices":[2,0],"scores":[0.3,0.7]}),
            json!({"indices":[0],"scores":[0.3]}),
            json!({"indices":[0,1],"scores":[0.3,"NaN"]}),
            json!({"indices":[0,1],"scores":[0.3,0.7]}),
            json!({"indices":[1,0],"scores":[0.7,0.7]}),
        ] {
            assert!(validate_order(&value, 2).is_err());
        }
    }

    #[tokio::test]
    async fn 記憶編集の排他中は次の検索へ進ませず世代を両端で更新する() {
        let initial = generation();
        let guard = pause().await.unwrap();
        assert_ne!(generation(), initial);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), WORKER.lock())
                .await
                .is_err()
        );
        let editing = generation();
        drop(guard);
        assert_ne!(generation(), editing);
        assert!(tokio::time::timeout(Duration::from_secs(1), WORKER.lock())
            .await
            .is_ok());
    }
}
