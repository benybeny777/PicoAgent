use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SidecarError {
    #[error("サイドカーを起動できません: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("サイドカーI/Oに失敗しました: {0}")]
    Io(#[from] std::io::Error),
    #[error("サイドカーが不正なJSON Linesを返しました: {0}")]
    Json(#[from] serde_json::Error),
    #[error("サイドカーを中断しました")]
    Cancelled,
    /// 画面へは末尾の要点だけを出す。Pythonのトレースバックや生成ツールのログは数百行に
    /// なり、状態欄へ全文を流すと原因の1行が埋もれる。全文は `stderr` に残す。
    #[error("サイドカーが失敗しました ({status}): {}", stderr_summary(stderr))]
    Failed {
        status: String,
        /// 終了コード。GPUメモリ不足など、呼び出し元が分類したい失敗を文言ではなく数値で受ける。
        code: Option<i32>,
        stderr: String,
    },
}

const SUMMARY_LINES: usize = 6;
const SUMMARY_CHARS: usize = 600;

/// 標準エラー出力の末尾（例外名と原因が出る位置）から空行を除いた数行だけを返す。
pub fn stderr_summary(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(SUMMARY_LINES);
    let mut summary = lines[start..].join("\n");
    let total = summary.chars().count();
    if total > SUMMARY_CHARS {
        summary = summary.chars().skip(total - SUMMARY_CHARS).collect();
    }
    if start > 0 || total > SUMMARY_CHARS {
        summary.insert_str(0, "…\n");
    }
    if summary.is_empty() {
        "出力なし".into()
    } else {
        summary
    }
}

pub struct SidecarProcess {
    child: Option<Child>,
    #[cfg(windows)]
    job: Option<std::os::windows::io::OwnedHandle>,
}

impl SidecarProcess {
    pub fn spawn(
        program: &Path,
        arguments: &[OsString],
        working_directory: &Path,
    ) -> Result<Self, SidecarError> {
        let mut command = Command::new(program);
        command
            .args(arguments)
            .current_dir(working_directory)
            // JSON Lines はOSのコードページに依存させず、常にUTF-8で受け渡す。
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
            command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        }
        #[cfg(unix)]
        {
            // macOSにはJob Objectが無い。サイドカーを独立したプロセスグループの
            // リーダーにして、ComfyUIなどの孫まで`killpg`でまとめて回収できるようにする。
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn().map_err(SidecarError::Spawn)?;
        // Job Objectへ所属させるのはWindowsだけなので、他OSでは`mut`が不要になる。
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut process = Self {
            child: Some(child),
            #[cfg(windows)]
            job: None,
        };
        #[cfg(windows)]
        {
            // 停止状態で所属させ、孫プロセスがJobの外へ先行することを防ぐ。
            process.job = Some(attach_job(
                process.child.as_ref().expect("起動済みプロセスが必要です"),
            )?);
            resume_owned_thread(
                process
                    .child
                    .as_ref()
                    .expect("起動済みプロセスが必要です")
                    .id(),
            )?;
        }
        Ok(process)
    }

    pub fn relay_json_lines(
        mut self,
        cancelled: Arc<AtomicBool>,
        mut relay: impl FnMut(Value),
    ) -> Result<(), SidecarError> {
        let child = self.child.as_mut().expect("起動済みプロセスが必要です");
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SidecarError::Io(std::io::Error::other("stdoutを取得できません")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SidecarError::Io(std::io::Error::other("stderrを取得できません")))?;
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let error_reader = thread::spawn(move || {
            let mut error_text = String::new();
            let mut stderr = stderr;
            stderr.read_to_string(&mut error_text).map(|_| error_text)
        });
        let read_result = (|| -> Result<(), SidecarError> {
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(SidecarError::Cancelled);
                }
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(line) => relay(serde_json::from_str(&line?)?),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if child.try_wait()?.is_some() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(())
        })();
        if read_result.is_err() {
            #[cfg(windows)]
            self.job.take();
            // 既に終了した場合のkill失敗でもwaitと読取スレッド回収を省略しない。
            #[cfg(unix)]
            terminate_process_group(child.id());
            let _ = child.kill();
        }
        #[cfg(unix)]
        let group = child.id();
        let status = child.wait();
        // 親が正常終了しても、pipeを継承した孫を先に回収して読取待ちを解く。
        #[cfg(windows)]
        self.job.take();
        #[cfg(unix)]
        terminate_process_group(group);
        let _ = reader.join();
        let error_result = error_reader.join().map_err(|_| {
            SidecarError::Io(std::io::Error::other("stderr読取スレッドが停止しました"))
        });
        self.child = None;
        read_result?;
        let status = status?;
        let error_text = error_result??;
        if status.success() {
            Ok(())
        } else {
            Err(SidecarError::Failed {
                status: status.to_string(),
                code: status.code(),
                stderr: error_text,
            })
        }
    }
}

impl Drop for SidecarProcess {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.job.take();
        if let Some(child) = &mut self.child {
            #[cfg(unix)]
            terminate_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 自分が作ったプロセスグループだけをまとめて終了させる（macOS・Linux）。
///
/// `spawn`時に`process_group(0)`でサイドカーをグループリーダーにしているため、
/// グループIDは子のPIDと一致する。名前による列挙・終了は行わない。
#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    let group = pid as i32;
    if group <= 0 {
        return;
    }
    // 安全性: 自分が起動したプロセスグループのIDだけを渡す。
    unsafe {
        libc::killpg(group, libc::SIGTERM);
    }
}

#[cfg(windows)]
fn attach_job(child: &Child) -> std::io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    // 無名・非継承のハンドルを唯一保持する。失敗経路もOwnedHandleで閉じる。
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = OwnedHandle::from_raw_handle(raw);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&info) as u32,
        ) == 0
            || AssignProcessToJobObject(raw, child.as_raw_handle()) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }
}

#[cfg(windows)]
fn resume_owned_thread(pid: u32) -> std::io::Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    // 停止中の自分のChildのPIDだけを対象にする。名前による列挙・終了は行わない。
    unsafe {
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if raw == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let snapshot = OwnedHandle::from_raw_handle(raw);
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut found = Thread32First(snapshot.as_raw_handle(), &mut entry);
        while found != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if thread.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let owned = OwnedHandle::from_raw_handle(thread);
                if ResumeThread(owned.as_raw_handle()) == u32::MAX {
                    return Err(std::io::Error::last_os_error());
                }
                return Ok(());
            }
            found = Thread32Next(snapshot.as_raw_handle(), &mut entry);
        }
        Err(std::io::Error::other(
            "起動したサイドカーの初期スレッドが見つかりません",
        ))
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    #[test]
    fn 失敗表示はトレースバック末尾の要点だけにする() {
        let mut stderr = String::from("Traceback (most recent call last):\n");
        for index in 0..200 {
            stderr.push_str(&format!("  File \"x.py\", line {index}, in f\n\n"));
        }
        stderr.push_str("RuntimeError: CUDA out of memory\n");
        let error = SidecarError::Failed {
            status: "exit code: 1".into(),
            code: Some(1),
            stderr,
        };
        let text = error.to_string();
        assert!(text.ends_with("RuntimeError: CUDA out of memory"), "{text}");
        assert!(text.contains('…'), "省略した印を付ける: {text}");
        assert!(text.lines().count() <= SUMMARY_LINES + 2, "{text}");
    }

    #[test]
    fn 短い出力と空の出力はそのまま分かる形で出す() {
        assert_eq!(stderr_summary("失敗しました\n"), "失敗しました");
        assert_eq!(stderr_summary("\n  \n"), "出力なし");
    }

    #[test]
    fn 一行が長すぎても文字数で末尾を残す() {
        let summary = stderr_summary(&"あ".repeat(5000));
        assert_eq!(summary.chars().count(), SUMMARY_CHARS + 2);
        assert!(summary.starts_with("…\n"));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };

    fn python() -> Option<std::path::PathBuf> {
        std::env::var_os("PICOAGENT_TEST_PYTHON")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_file())
    }

    fn open_owned_test_child(value: &Value) -> OwnedHandle {
        let pid = value["pid"].as_u64().expect("テストの子PID") as u32;
        // テストが起動したPythonから返された子だけを待機し、操作対象を広げない。
        unsafe {
            let raw = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            assert!(!raw.is_null(), "{}", std::io::Error::last_os_error());
            OwnedHandle::from_raw_handle(raw)
        }
    }

    fn assert_terminated(handle: &OwnedHandle) {
        unsafe {
            assert_eq!(
                WaitForSingleObject(handle.as_raw_handle(), 5_000),
                WAIT_OBJECT_0
            );
        }
    }

    fn spawn_tree(python: &Path, ending: &str) -> SidecarProcess {
        let script = format!(
            "import subprocess,sys,json,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); print(json.dumps({{'pid':p.pid,'text':'日本語'}}),flush=True); time.sleep(.2); {ending}"
        );
        SidecarProcess::spawn(
            python,
            &["-c".into(), script.into()],
            Path::new(env!("CARGO_MANIFEST_DIR")),
        )
        .unwrap()
    }

    #[test]
    fn job_reclaims_descendants_on_normal_exit_cancel_and_bad_json() {
        let Some(python) = python() else { return };
        for ending in [
            "sys.exit(0)",
            "time.sleep(30)",
            "print('invalid-json',flush=True); time.sleep(30)",
        ] {
            let cancelled = Arc::new(AtomicBool::new(false));
            let mut descendant = None;
            let result = spawn_tree(&python, ending).relay_json_lines(cancelled.clone(), |value| {
                assert_eq!(value["text"], "日本語");
                descendant = Some(open_owned_test_child(&value));
                if ending == "time.sleep(30)" {
                    cancelled.store(true, Ordering::Relaxed);
                }
            });
            if ending == "sys.exit(0)" {
                assert!(result.is_ok(), "{result:?}");
            } else if ending == "time.sleep(30)" {
                assert!(matches!(result, Err(SidecarError::Cancelled)));
            } else {
                assert!(matches!(result, Err(SidecarError::Json(_))));
            }
            assert_terminated(&descendant.expect("子生成の通知"));
        }
    }

    #[test]
    fn job_reclaims_descendants_when_guard_is_dropped() {
        let Some(python) = python() else { return };
        let mut process = spawn_tree(&python, "time.sleep(30)");
        let stdout = process.child.as_mut().unwrap().stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = sender.send(result);
        });
        let line = receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let descendant = open_owned_test_child(&serde_json::from_str(&line).unwrap());
        drop(process);
        assert_terminated(&descendant);
        reader.join().unwrap();
    }
}
