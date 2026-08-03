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
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

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
    // フォールバック許可: 番号採番のロックが毒されても採番は続ける（中身は数値のみ）。
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
                // フォールバック許可: 読み取りエラーはパイプ終了として扱い、スレッドを畳む。
                Err(_) => break,
            }
        }
    });
}

/// 終了済みジョブの状態を取り込み、古い履歴を捨てる。
fn refresh(jobs: &mut Vec<Job>) {
    for job in jobs.iter_mut() {
        if job.status != Status::Running {
            continue;
        }
        let Some(child) = job.child.as_mut() else {
            continue;
        };
        match child.try_wait() {
            Ok(Some(exit)) => {
                job.status = Status::Exited(exit.code());
                // 終了を確認できた時点でハンドルを解放する（ゾンビを残さない）。
                job.child = None;
            }
            Ok(None) => {}
            Err(error) => {
                job.status = Status::Exited(None);
                job.child = None;
                if let Ok(mut buffer) = job.output.lock() {
                    buffer.append(&format!("\n[状態を取得できませんでした: {error}]"));
                }
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
        for index in finished
            .into_iter()
            .take(jobs.len() - MAX_RETAINED)
            .rev()
        {
            jobs.remove(index);
        }
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
    // future の終了後も動かし続けるため、同期版のコマンドを使う（窓抑止だけ共通化される）。
    let mut command = crate::tools::process::command_sync(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("起動できませんでした（{error}）: {program}"))?;

    let output = Arc::new(Mutex::new(Buffer::default()));
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, Arc::clone(&output));
    }
    if let Some(stderr) = child.stderr.take() {
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
        out.push_str(&format!("\n（古い出力 {dropped} バイトは上限超過で破棄しました）"));
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
    match job.child.take() {
        Some(mut child) => {
            let killed = child.kill();
            // kill したら wait でハンドルまで回収する（ゾンビを残さない）。
            let _ = child.wait();
            job.status = Status::Stopped;
            match killed {
                Ok(()) => format!("OK: {id} を停止しました。"),
                Err(error) => format!("{id} は既に終了していました（{error}）。"),
            }
        }
        None => format!("{id} は既に{}です。", job.status.label()),
    }
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
    // フォールバック許可: 終了処理なのでロックが毒されていても停止を試みる。
    let mut jobs = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for job in jobs.iter_mut() {
        if let Some(mut child) = job.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            job.status = Status::Stopped;
        }
    }
    jobs.clear();
}

#[cfg(test)]
mod tests {
    use super::{list, output, spawn, stop, Buffer, MAX_BUFFER_BYTES};

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
        assert!(buffer.text.len() <= MAX_BUFFER_BYTES, "len: {}", buffer.text.len());
        assert!(buffer.dropped > 0);
        // 文字境界で切れていれば、そのまま文字列として扱える。
        assert!(buffer.text.chars().all(|c| c == 'あ'));
    }
}
