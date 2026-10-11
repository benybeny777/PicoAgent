// skills/run_command/jobs.rs - バックグラウンド実行ジョブの管理。
//
// `run_command` は timeout 付きの同期実行なので、開発サーバ・長いビルド・テストのように
// 「走らせたまま様子を見たい」処理を扱えず、タイムアウトで打ち切るしかなかった。
// ここでは起動だけして job_id を返し、`command_job` スキルが途中出力の取得と停止を行う。
//
// 許可判定（allowlist・cwd・物理削除拒否）は呼び出し元の `run_command` が済ませてから
// `spawn()` を呼ぶ。このモジュールは「起動済みの子プロセスをどう飼うか」だけを持つ。

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};

use crate::tools::process::{OwnedChild as Child, ProcessSnapshot};

/// 同時に走らせられるジョブ数。
const MAX_RUNNING: usize = 4;
/// 1ジョブが保持する出力の上限（超えた分は古い側から捨て、捨てた量を伝える）。
const MAX_BUFFER_BYTES: usize = 64 * 1024;
/// 終了済みジョブを一覧に残す件数。
const MAX_RETAINED: usize = 20;

#[derive(Default)]
struct Buffer {
    text: String,
    /// 既に `output` で返した位置（バイト）。
    cursor: usize,
    /// 上限超過で捨てたバイト数。
    dropped: usize,
}

impl Buffer {
    fn append(&mut self, chunk: &str) {
        self.text.push_str(chunk);
        while self.text.len() > MAX_BUFFER_BYTES {
            // 先頭 1/4 を捨てる。文字の途中で切らないよう境界まで進める。
            let mut cut = self.text.len() / 4;
            while cut < self.text.len() && !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
            self.dropped += cut;
            self.cursor = self.cursor.saturating_sub(cut);
        }
    }

    /// 未読部分を返してカーソルを進める。`from_start` なら保持分すべてを返す。
    fn take(&mut self, from_start: bool) -> String {
        if from_start {
            self.cursor = self.text.len();
            return self.text.clone();
        }
        let mut start = self.cursor.min(self.text.len());
        while start < self.text.len() && !self.text.is_char_boundary(start) {
            start += 1;
        }
        let out = self.text[start..].to_string();
        self.cursor = self.text.len();
        out
    }
}

#[derive(Clone, PartialEq)]
enum Status {
    Running,
    Exited(Option<i32>),
    Stopped,
}

impl Status {
    fn label(&self) -> String {
        match self {
            Status::Running => "実行中".to_string(),
            Status::Exited(Some(code)) => format!("終了（コード {code}）"),
            Status::Exited(None) => "終了（コード不明）".to_string(),
            Status::Stopped => "停止済み".to_string(),
        }
    }
}

struct Job {
    id: String,
    command: String,
    cwd: String,
    child: Option<Child>,
    output: Arc<Mutex<Buffer>>,
    status: Status,
}

fn registry() -> &'static Mutex<Vec<Job>> {
    static JOBS: OnceLock<Mutex<Vec<Job>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(Vec::new()))
}

fn lock() -> Result<std::sync::MutexGuard<'static, Vec<Job>>, String> {
    registry()
        .lock()
        .map_err(|_| "バックグラウンドジョブの一覧を取得できません。".to_string())
}

fn next_id(jobs: &[Job]) -> String {
    static COUNTER: OnceLock<Mutex<usize>> = OnceLock::new();
    let counter = COUNTER.get_or_init(|| Mutex::new(0));
    // 番号採番のロックが毒されても採番は続ける（中身は数値のみ）。
    let mut value = counter
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *value += 1;
    let mut id = format!("job-{value}");
    while jobs.iter().any(|job| job.id == id) {
        *value += 1;
        id = format!("job-{value}");
    }
    id
}

/// 子プロセスの出力を別スレッドで読み続け、共有バッファへ溜める。
///
/// チャンク境界で日本語が壊れないよう `text::Utf8Stream` で逐次デコードする。
fn pump<R: Read + Send + 'static>(mut reader: R, buffer: Arc<Mutex<Buffer>>) {
    std::thread::spawn(move || {
        let mut stream = crate::text::Utf8Stream::default();
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    let text = stream.push(&chunk[..read]);
                    if text.is_empty() {
                        continue;
                    }
                    let Ok(mut buffer) = buffer.lock() else {
                        break;
                    };
                    buffer.append(&text);
                }
                // 読み取りエラーはパイプ終了として扱い、スレッドを畳む。
                Err(_) => break,
            }
        }
    });
}

/// 終了済みジョブの状態を取り込み、古い履歴を捨てる。
fn refresh(jobs: &mut Vec<Job>) {
    let mut exited = Vec::new();
    // 一覧取得より先に、対象となる根すべての終了を観測する。取得後に初めて
    // 終了した根へ古い一覧を使うと、直前に起動した孫を取りこぼす。
    for (index, job) in jobs.iter_mut().enumerate() {
        if job.status != Status::Running {
            continue;
        }
        let Some(child) = job.child.as_mut() else {
            continue;
        };
        match child.try_root_exit() {
            Ok(Some(_)) => exited.push(index),
            Ok(None) => {}
            Err(error) => append_refresh_error(job, &error),
        }
    }
    let mut snapshot = ProcessSnapshot::default();
    for index in exited {
        let job = &mut jobs[index];
        let Some(child) = job.child.as_mut() else {
            continue;
        };
        match child.try_complete(&mut snapshot) {
            Ok(Some(exit)) => {
                job.status = Status::Exited(exit.code());
                // 根の終了と所有する子孫の終了がそろった時点だけ、履歴へ移す。
                job.child = None;
            }
            Ok(None) => {}
            Err(error) => {
                // 状態不明を終了扱いにせず、停止・再確認に必要な所有ハンドルを残す。
                append_refresh_error(job, &error);
            }
        }
    }
    let finished: Vec<usize> = jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.status != Status::Running)
        .map(|(index, _)| index)
        .collect();
    if finished.len() > MAX_RETAINED {
        let excess = finished.len() - MAX_RETAINED;
        for index in finished.into_iter().take(excess).rev() {
            jobs.remove(index);
        }
    }
}

fn append_refresh_error(job: &Job, error: &str) {
    if let Ok(mut buffer) = job.output.lock() {
        buffer.append(&format!("\n[状態を取得できませんでした: {error}]"));
    }
}

/// バックグラウンドで起動し、job_id を返す。
///
/// 許可判定は呼び出し元（`run_command`）で済ませてから呼ぶこと。
pub fn spawn(program: &str, args: &[String], cwd: &Path) -> Result<String, String> {
    let mut jobs = lock()?;
    refresh(&mut jobs);
    let running = jobs
        .iter()
        .filter(|job| job.status == Status::Running)
        .count();
    if running >= MAX_RUNNING {
        return Err(format!(
            "同時に実行できるバックグラウンドジョブは {MAX_RUNNING} 件までです。command_job の stop で止めてください。"
        ));
    }
    // futureの終了後も動かし続ける。根が先に終わっても所有範囲を保持する。
    let mut command = crate::tools::process::command_sync(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Child::spawn(&mut command)
        .map_err(|error| format!("起動できませんでした（{error}）: {program}"))?;

    let output = Arc::new(Mutex::new(Buffer::default()));
    if let Some(stdout) = child.take_stdout() {
        pump(stdout, Arc::clone(&output));
    }
    if let Some(stderr) = child.take_stderr() {
        pump(stderr, Arc::clone(&output));
    }
    let id = next_id(&jobs);
    jobs.push(Job {
        id: id.clone(),
        command: crate::text::preview(
            &std::iter::once(program.to_string())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" "),
            200,
        ),
        cwd: cwd.display().to_string(),
        child: Some(child),
        output,
        status: Status::Running,
    });
    Ok(id)
}

pub fn list() -> String {
    let mut jobs = match lock() {
        Ok(jobs) => jobs,
        Err(error) => return format!("エラー: {error}"),
    };
    refresh(&mut jobs);
    if jobs.is_empty() {
        return "バックグラウンドジョブはありません。".to_string();
    }
    let mut out = format!("バックグラウンドジョブ: {} 件", jobs.len());
    for job in jobs.iter() {
        out.push_str(&format!(
            "\n{}\t{}\t{}（cwd: {}）",
            job.id,
            job.status.label(),
            job.command,
            job.cwd
        ));
    }
    out
}

pub fn output(id: &str, from_start: bool) -> String {
    let mut jobs = match lock() {
        Ok(jobs) => jobs,
        Err(error) => return format!("エラー: {error}"),
    };
    refresh(&mut jobs);
    let Some(job) = jobs.iter().find(|job| job.id == id) else {
        return format!("エラー: そのジョブはありません: {id}\n{}", summary(&jobs));
    };
    let (text, dropped) = match job.output.lock() {
        Ok(mut buffer) => (buffer.take(from_start), buffer.dropped),
        Err(_) => return format!("エラー: {id} の出力を取得できません。"),
    };
    let mut out = format!("{} [{}] {}", job.id, job.status.label(), job.command);
    if dropped > 0 {
        out.push_str(&format!(
            "\n（古い出力 {dropped} バイトは上限超過で破棄しました）"
        ));
    }
    if text.is_empty() {
        out.push_str("\n（新しい出力はありません）");
    } else {
        out.push('\n');
        out.push_str(&text);
    }
    out
}

pub fn stop(id: &str) -> String {
    let mut jobs = match lock() {
        Ok(jobs) => jobs,
        Err(error) => return format!("エラー: {error}"),
    };
    refresh(&mut jobs);
    let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
        return format!("エラー: そのジョブはありません: {id}\n{}", summary(&jobs));
    };
    match stop_job(job, Child::terminate) {
        Ok(true) => format!("OK: {id} を停止しました。"),
        Ok(false) => format!("{id} は既に{}です。", job.status.label()),
        Err(error) => format!("エラー: {id} を停止できませんでした: {error}"),
    }
}

fn stop_job(
    job: &mut Job,
    terminate: impl FnOnce(&mut Child, &str) -> Result<(), String>,
) -> Result<bool, String> {
    let Some(child) = job.child.as_mut() else {
        return Ok(false);
    };
    terminate(child, "バックグラウンドジョブ")?;
    // ツリー停止と終了ハンドルの回収に成功してから、状態と所有権を同時に確定する。
    job.child = None;
    job.status = Status::Stopped;
    Ok(true)
}

fn summary(jobs: &[Job]) -> String {
    if jobs.is_empty() {
        return "現在ジョブはありません。".to_string();
    }
    format!(
        "現在のジョブ: {}",
        jobs.iter()
            .map(|job| job.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// アプリ終了時に、走らせたままの子プロセスを確実に止める。
pub fn stop_all() {
    // 終了処理なのでロックが毒されていても停止を試みる。
    let mut jobs = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for error in stop_jobs(&mut jobs, Child::terminate) {
        crate::log_paths::log_detached(&error);
    }
}

fn stop_jobs(
    jobs: &mut Vec<Job>,
    mut terminate: impl FnMut(&mut Child, &str) -> Result<(), String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    jobs.retain_mut(|job| match stop_job(job, &mut terminate) {
        Ok(_) => false,
        Err(error) => {
            errors.push(format!(
                "バックグラウンドジョブ {} の停止失敗: {error}",
                job.id
            ));
            true
        }
    });
    errors
}

#[cfg(test)]
mod tests {
    use super::{list, output, spawn, stop, Buffer, MAX_BUFFER_BYTES};

    // 回帰を注入した検査が途中で失敗しても、自分が起動したプロセスを回収する。
    struct TestJobs(Vec<super::Job>, Vec<u32>);

    impl TestJobs {
        fn new(job: super::Job) -> Self {
            let mut jobs = Self(Vec::new(), Vec::new());
            jobs.push(job);
            jobs
        }

        fn push(&mut self, job: super::Job) {
            if let Some(child) = job.child.as_ref() {
                self.1.push(child.id());
            }
            self.0.push(job);
        }
    }

    impl Drop for TestJobs {
        fn drop(&mut self) {
            for job in &mut self.0 {
                if let Some(child) = job.child.as_mut() {
                    if let Err(error) = child.terminate("ジョブ検査の後始末") {
                        eprintln!("ジョブ検査の後始末に失敗しました: {error}");
                    }
                }
            }
            // 所有ハンドルを失う退行を注入した場合にも、起動時に記録した検査PIDだけを止める。
            for pid in &self.1 {
                if crate::tools::process::process_alive(*pid).is_ok_and(|alive| alive) {
                    #[cfg(windows)]
                    let mut command = crate::tools::process::command_sync("taskkill.exe");
                    #[cfg(windows)]
                    command.args(["/PID", &pid.to_string(), "/T", "/F"]);
                    #[cfg(not(windows))]
                    let mut command = crate::tools::process::command_sync("kill");
                    #[cfg(not(windows))]
                    command.args(["-KILL", &pid.to_string()]);
                    if let Err(error) = command
                        .stdout(super::Stdio::null())
                        .stderr(super::Stdio::null())
                        .status()
                    {
                        eprintln!("検査PID {pid} の回収に失敗しました: {error}");
                    }
                }
            }
        }
    }

    fn test_job(id: &str, command: Option<std::process::Command>) -> super::Job {
        let child = command.map(|mut command| {
            command
                .stdin(super::Stdio::null())
                .stdout(super::Stdio::null())
                .stderr(super::Stdio::null());
            super::Child::spawn(&mut command).unwrap()
        });
        super::Job {
            id: id.to_owned(),
            command: "停止検査".into(),
            cwd: String::new(),
            child,
            output: std::sync::Arc::new(std::sync::Mutex::new(Buffer::default())),
            status: super::Status::Running,
        }
    }

    fn waiting_command() -> std::process::Command {
        #[cfg(windows)]
        {
            let mut command = crate::tools::process::command_sync("ping.exe");
            command.args(["-n", "31", "127.0.0.1"]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = crate::tools::process::command_sync("sleep");
            command.arg("30");
            command
        }
    }

    #[test]
    fn 停止失敗は状態と所有ハンドルを保持し一括停止も他のジョブを回収する() {
        for all in [false, true] {
            let mut jobs = TestJobs::new(test_job("失敗対象", Some(waiting_command())));
            let pid = jobs.0[0].child.as_ref().unwrap().id();
            if all {
                jobs.push(test_job("成功対象", Some(waiting_command())));
                let errors = super::stop_jobs(&mut jobs.0, |child, label| {
                    if child.id() == pid {
                        Err("停止できません".into())
                    } else {
                        child.terminate(label)
                    }
                });
                assert_eq!(errors.len(), 1);
                assert_eq!(jobs.0.len(), 1);
            } else {
                assert_eq!(
                    super::stop_job(&mut jobs.0[0], |_, _| Err("停止できません".into()))
                        .unwrap_err(),
                    "停止できません"
                );
            }
            assert!(matches!(jobs.0[0].status, super::Status::Running));
            assert_eq!(jobs.0[0].child.as_ref().map(super::Child::id), Some(pid));
            assert!(crate::tools::process::process_alive(pid).unwrap());
            assert!(super::stop_job(&mut jobs.0[0], super::Child::terminate).unwrap());
            assert!(jobs.0[0].child.is_none());
            assert!(matches!(jobs.0[0].status, super::Status::Stopped));
            assert!(!super::stop_job(&mut jobs.0[0], |_, _| panic!(
                "停止済みのプロセスへ再停止しました"
            ))
            .unwrap());
            assert!(super::stop_jobs(&mut jobs.0, |_, _| panic!(
                "回収済みのプロセスへ再停止しました"
            ))
            .is_empty());
            assert!(jobs.0.is_empty());
        }
    }

    #[test]
    fn 実行中のジョブが混在しても完了履歴は上限件数まで保持する() {
        let mut jobs = Vec::new();
        for index in 0..(super::MAX_RETAINED + 3) {
            let mut finished = test_job(&format!("完了{index}"), None);
            finished.status = super::Status::Exited(Some(0));
            jobs.push(finished);
            if index < super::MAX_RUNNING {
                jobs.push(test_job(&format!("実行中{index}"), None));
            }
        }
        super::refresh(&mut jobs);
        assert_eq!(jobs.len(), super::MAX_RETAINED + super::MAX_RUNNING);
        let completed: Vec<_> = jobs
            .iter()
            .filter(|job| job.status != super::Status::Running)
            .map(|job| job.id.as_str())
            .collect();
        assert_eq!(completed.len(), super::MAX_RETAINED);
        assert_eq!(completed[0], "完了3");
        for index in 0..super::MAX_RUNNING {
            assert!(jobs.iter().any(|job| job.id == format!("実行中{index}")));
        }
    }

    #[cfg(windows)]
    #[test]
    fn 個別停止と一括停止はwindowsで孫プロセスまで回収する() {
        use crate::tools::process::{
            process_alive,
            tests::{read_marker_ids, tree_test_args},
        };
        for all in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("owned-processes.txt");
            let mut command = crate::tools::process::command_sync("powershell.exe");
            command.args(tree_test_args(&marker));
            let mut jobs = TestJobs::new(test_job("ツリー停止対象", Some(command)));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let ids = loop {
                if let Ok(ids) = read_marker_ids(&marker) {
                    break ids;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "検査用の親子PIDがそろいません"
                );
                std::thread::sleep(std::time::Duration::from_millis(25));
            };
            jobs.1.extend(&ids);
            assert_eq!(ids[0], jobs.0[0].child.as_ref().unwrap().id());
            for pid in &ids {
                assert!(
                    process_alive(*pid).unwrap(),
                    "停止前に検査PID {pid} が終了しました"
                );
            }
            if all {
                assert!(super::stop_jobs(&mut jobs.0, super::Child::terminate).is_empty());
            } else {
                assert!(super::stop_job(&mut jobs.0[0], super::Child::terminate).unwrap());
            }
            for pid in ids {
                assert!(
                    !process_alive(pid).unwrap(),
                    "停止後も検査PID {pid} が残りました"
                );
            }
        }
    }

    #[test]
    fn 親が先に終了しても一覧更新と履歴圧縮後に孫を停止できる() {
        use crate::tools::process::tests::{
            test_process_running, tree_test_command, wait_marker_ids, TestProcessIds,
        };
        let independent_dir = tempfile::tempdir().unwrap();
        let independent_marker = independent_dir.path().join("independent.txt");
        let mut independent_command = tree_test_command(&independent_marker, true, false, 0);
        independent_command
            .stdout(super::Stdio::null())
            .stderr(super::Stdio::null());
        let mut independent = super::Child::spawn(&mut independent_command).unwrap();
        let independent_ids = TestProcessIds(wait_marker_ids(&independent_marker));
        for mode in 0..4 {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("owned.txt");
            let command = tree_test_command(&marker, false, false, 0);
            let mut jobs = TestJobs::new(test_job("親先行終了", Some(command)));
            let ids = TestProcessIds(wait_marker_ids(&marker));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while jobs.0[0]
                .child
                .as_mut()
                .unwrap()
                .try_root_exit()
                .unwrap()
                .is_none()
            {
                assert!(
                    std::time::Instant::now() < deadline,
                    "検査の根が終了しません"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if mode != 0 {
                // list/outputが共有する更新を通し、20件の履歴圧縮にも巻き込ませない。
                for index in 0..(super::MAX_RETAINED + 2) {
                    let mut finished = test_job(&format!("履歴{index}"), None);
                    finished.status = super::Status::Exited(Some(0));
                    jobs.push(finished);
                }
                super::refresh(&mut jobs.0);
                assert!(matches!(jobs.0[0].status, super::Status::Running));
                assert!(jobs.0[0].child.is_some());
                assert_eq!(jobs.0.len(), super::MAX_RETAINED + 1);
                assert!(
                    test_process_running(ids.0[1]),
                    "一覧更新だけで孫を停止しました"
                );
            }
            if mode == 3 {
                assert!(
                    super::stop_job(&mut jobs.0[0], |_, _| Err("検査用の停止失敗".into())).is_err()
                );
                assert!(jobs.0[0].child.is_some());
                assert!(test_process_running(ids.0[1]));
            }
            if mode == 2 {
                assert!(super::stop_jobs(&mut jobs.0, super::Child::terminate).is_empty());
            } else {
                assert!(super::stop_job(&mut jobs.0[0], super::Child::terminate).unwrap());
            }
            assert!(
                !test_process_running(ids.0[1]),
                "停止成功後も孫が実行中です"
            );
            assert!(
                test_process_running(independent.id()),
                "独立した根を停止しました"
            );
            assert!(
                test_process_running(independent_ids.0[1]),
                "独立した孫を停止しました"
            );
        }
        independent.terminate("独立ジョブの検査後始末").unwrap();
    }

    /// 起動 → 途中出力の取得 → 停止までを実プロセスで通す。
    #[test]
    fn バックグラウンド実行の出力を取得して停止できる() {
        let program = if cfg!(windows) { "cmd" } else { "sh" };
        let args: Vec<String> = if cfg!(windows) {
            vec!["/C".to_string(), "echo picoagent".to_string()]
        } else {
            vec!["-c".to_string(), "echo picoagent".to_string()]
        };
        let id = spawn(program, &args, &std::env::temp_dir()).expect("起動できない");

        // 出力を読むスレッドが追いつくまで少しだけ待つ（最大 2.5 秒）。
        let mut text = String::new();
        for _ in 0..50 {
            text = output(&id, true);
            if text.contains("picoagent") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(text.contains("picoagent"), "出力: {text}");
        assert!(list().contains(&id), "一覧に出ない: {}", list());

        // 未読分だけを返すので、続けて読むと空になる。
        assert!(output(&id, false).contains("新しい出力はありません"));
        stop(&id);
        assert!(stop("job-存在しない").starts_with("エラー:"));
    }

    #[test]
    fn 未読分だけを返してカーソルを進める() {
        let mut buffer = Buffer::default();
        buffer.append("あいう");
        assert_eq!(buffer.take(false), "あいう");
        assert_eq!(buffer.take(false), "");
        buffer.append("えお");
        assert_eq!(buffer.take(false), "えお");
        // from_start は保持分すべてを返す。
        assert_eq!(buffer.take(true), "あいうえお");
    }

    /// 上限超過で古い側を捨てても、日本語が文字の途中で切れないこと。
    #[test]
    fn 上限を超えたら古い出力を文字境界で捨てる() {
        let mut buffer = Buffer::default();
        for _ in 0..(MAX_BUFFER_BYTES / 3 + 100) {
            buffer.append("あ");
        }
        assert!(
            buffer.text.len() <= MAX_BUFFER_BYTES,
            "len: {}",
            buffer.text.len()
        );
        assert!(buffer.dropped > 0);
        // 文字境界で切れていれば、そのまま文字列として扱える。
        assert!(buffer.text.chars().all(|c| c == 'あ'));
    }
}
