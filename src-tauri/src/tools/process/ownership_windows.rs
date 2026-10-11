//! Jobへの登録が終わるまで子のコードを実行しないWindowsの所有範囲。

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::os::windows::process::CommandExt;

use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
            GetProcessIdOfThread, OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED,
            THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
        },
    },
};

#[derive(Default)]
pub(crate) struct ProcessSnapshot {
    _private: (),
}

pub(super) struct ProcessScope {
    job: Option<OwnedHandle>,
    id: u32,
    attached: bool,
}

fn os_error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}

impl ProcessScope {
    pub(super) fn prepare(command: &mut std::process::Command) -> Result<Self, String> {
        // 共通コマンドの既定以外のcreation_flagsを持つ呼出元はない。
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(os_error("プロセスの所有領域を作れません"));
        }
        let scope = Self {
            job: Some(unsafe { OwnedHandle::from_raw_handle(raw) }),
            id: 0,
            attached: false,
        };
        scope.set_kill_on_close(true)?;
        Ok(scope)
    }

    pub(super) fn set_id(&mut self, id: u32) {
        self.id = id;
    }

    pub(super) fn attach(&mut self, process: RawHandle) -> Result<(), String> {
        self.attach_with_resume(process, |thread| {
            let previous = unsafe { ResumeThread(thread) };
            if previous != 1 {
                return Err(if previous == u32::MAX {
                    os_error("所有領域へ登録したプロセスを開始できません")
                } else {
                    format!("起動時のスレッド停止数が不正です: {previous}")
                });
            }
            Ok(())
        })
    }

    fn attach_with_resume(
        &mut self,
        process: RawHandle,
        resume: impl FnOnce(RawHandle) -> Result<(), String>,
    ) -> Result<(), String> {
        let job = self.job.as_ref().ok_or("プロセスの所有領域がありません")?;
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } == 0 {
            return Err(os_error("起動したプロセスを所有領域へ登録できません"));
        }
        self.attached = true;
        // stable Rustは主スレッドのハンドルを公開しない。まだ実行していない根に限定し、
        // 列挙したIDを開いた後も所有PIDを再照合する。根のハンドルはこの間ずっと保持する。
        let thread = suspended_thread(self.id)?;
        resume(thread.as_raw_handle())
    }

    fn set_kill_on_close(&self, enabled: bool) -> Result<(), String> {
        let Some(job) = &self.job else {
            return Ok(());
        };
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        if enabled {
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        }
        if unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(os_error("プロセス所有領域の終了条件を設定できません"));
        }
        Ok(())
    }

    pub(super) fn is_owned(&self) -> bool {
        self.job.is_some()
    }

    pub(super) fn root_attached(&self) -> bool {
        self.attached
    }

    pub(super) fn has_descendants(&self, _snapshot: &mut ProcessSnapshot) -> Result<bool, String> {
        Ok(!self.stopped()?)
    }

    pub(super) fn stopped(&self) -> Result<bool, String> {
        let Some(job) = &self.job else {
            return Ok(true);
        };
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as _,
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(os_error("所有するプロセスの状態を確認できません"));
        }
        Ok(info.ActiveProcesses == 0)
    }

    pub(super) fn terminate(&mut self) -> Result<(), String> {
        if let Some(job) = &self.job {
            if unsafe { TerminateJobObject(job.as_raw_handle(), 1) } == 0 {
                return Err(os_error("所有するプロセスを停止できません"));
            }
        }
        Ok(())
    }

    pub(super) fn release(&mut self) -> Result<(), String> {
        // 成功時に起動アプリを終了させない。解除失敗ではJobを保持してエラーを返す。
        self.set_kill_on_close(false)?;
        self.job = None;
        Ok(())
    }
}

fn suspended_thread(pid: u32) -> Result<OwnedHandle, String> {
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(os_error("起動スレッドを列挙できません"));
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut present = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
    let mut found = None;
    while present != 0 {
        if entry.th32OwnerProcessID == pid {
            let raw = unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                    0,
                    entry.th32ThreadID,
                )
            };
            if raw.is_null() {
                return Err(os_error("起動スレッドを取得できません"));
            }
            let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
            if unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } != pid {
                return Err("起動スレッドの所有プロセスが一致しません".into());
            }
            if found.replace(thread).is_some() {
                return Err("停止中の起動スレッドを一意に確認できません".into());
            }
        }
        present = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
    }
    if std::io::Error::last_os_error().raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
        return Err(os_error("起動スレッドの列挙を完了できません"));
    }
    found.ok_or_else(|| "停止中の起動スレッドがありません".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 登録前には実行せず登録失敗と再開失敗でも停止中の根を回収する() {
        use windows_sys::Win32::System::Threading::SuspendThread;

        for mode in 0..3 {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("started.txt");
            let mut command = crate::tools::process::tests::fixture_command(&marker, "marker");
            command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let mut scope = ProcessScope::prepare(&mut command).unwrap();
            let child = command.spawn().unwrap();
            scope.set_id(child.id());
            let mut owned = super::super::OwnedChild { child, scope };
            let pid = owned.id();
            // 実行が遅かっただけで通らないよう、起動スレッドの停止回数も確認する。
            let thread = suspended_thread(pid).unwrap();
            let previous = unsafe { SuspendThread(thread.as_raw_handle()) };
            assert_ne!(previous, u32::MAX);
            assert_eq!(
                unsafe { ResumeThread(thread.as_raw_handle()) },
                previous + 1
            );
            assert_eq!(previous, 1, "所有登録前の根が停止状態ではありません");
            assert!(!marker.exists(), "所有登録前に子のコードが実行されました");
            if mode == 0 {
                owned.scope.attach(owned.child.as_raw_handle()).unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                loop {
                    if let Some(status) = owned.child.try_wait().unwrap() {
                        assert!(status.success());
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "登録後のfixtureが終了しません"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                assert_eq!(std::fs::read_to_string(&marker).unwrap(), "started");
                continue;
            }
            let result = if mode == 2 {
                owned
                    .scope
                    .attach_with_resume(owned.child.as_raw_handle(), |_| {
                        Err("検査用の再開失敗".into())
                    })
            } else {
                owned.scope.attach(std::ptr::null_mut())
            };
            assert!(result.is_err());
            // 通常のspawnと同じく、Jobへ入る前の失敗でも根を明示停止してからDropする。
            owned.child.kill().unwrap();
            drop(owned);
            assert!(
                !marker.exists(),
                "登録・再開失敗後に子のコードが実行されました"
            );
            assert!(!crate::tools::process::process_alive(pid).unwrap());
        }
    }
}
