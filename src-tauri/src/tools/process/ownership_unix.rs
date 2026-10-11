//! 専用process groupを、根のPIDをreapせず保持して所有する。

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::ExitStatus;

/// 1回のジョブ更新ではOSの一覧を一度だけ取得する。読取失敗も共有して空扱いしない。
#[derive(Default)]
pub(crate) struct ProcessSnapshot(Option<Result<Vec<(u32, u32)>, String>>);

pub(super) struct ProcessScope {
    id: Option<u32>,
}

impl ProcessScope {
    pub(super) fn prepare(command: &mut std::process::Command) -> Result<Self, String> {
        // exec前に専用グループを確立する。設定失敗はCommand::spawnから返る。
        command.process_group(0);
        Ok(Self { id: None })
    }

    pub(super) fn set_id(&mut self, id: u32) {
        self.id = Some(id);
    }

    pub(super) fn is_owned(&self) -> bool {
        self.id.is_some()
    }

    pub(super) fn peek_exit(&self) -> Result<Option<ExitStatus>, String> {
        let id = self.id.ok_or("プロセスの所有権を既に解放しています")?;
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    id as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                // ECHILDを「終了済み」にしない。他のwaitが所有権を消費した状態で
                // 保存した数値だけを使うと再利用された別groupへsignalしてしまう。
                return Err(format!("所有する根プロセスの終了を確認できません: {error}"));
            }
            if unsafe { info.si_pid() } == 0 {
                return Ok(None);
            }
            let code = unsafe { info.si_status() };
            let raw = match info.si_code {
                libc::CLD_EXITED => code << 8,
                libc::CLD_KILLED => code,
                libc::CLD_DUMPED => code | 0x80,
                other => return Err(format!("根プロセスの終了種別が不正です: {other}")),
            };
            return Ok(Some(ExitStatus::from_raw(raw)));
        }
    }

    pub(super) fn has_descendants(&self, snapshot: &mut ProcessSnapshot) -> Result<bool, String> {
        let id = self.id.ok_or("プロセスの所有権を既に解放しています")?;
        let processes = snapshot.0.get_or_insert_with(process_groups);
        match processes {
            Ok(processes) => {
                for (pid, group) in processes {
                    if *pid != id && *group == id && member_running(*pid, id)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Err(error) => Err(error.clone()),
        }
    }

    pub(super) fn terminate(&mut self) -> Result<(), String> {
        let Some(id) = self.id else {
            return Ok(());
        };
        // WNOWAITで根の所有を再確認する。根が未回収なのでこのPGIDは再利用されない。
        self.peek_exit()?;
        if unsafe { libc::kill(-(id as libc::pid_t), libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("所有するプロセスグループを停止できません: {error}"));
            }
        }
        // 停止要求だけでは根をreapしない。stoppedで子孫の停止を確認するまで番号を保持する。
        Ok(())
    }

    pub(super) fn stopped(&self) -> Result<bool, String> {
        if self.id.is_none() {
            return Ok(true);
        }
        Ok(self.peek_exit()?.is_some() && !self.has_descendants(&mut ProcessSnapshot::default())?)
    }

    pub(super) fn release(&mut self) -> Result<(), String> {
        self.id = None;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn member_running(pid: u32, expected_group: u32) -> Result<bool, String> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("プロセス {pid} の状態を取得できません: {error}")),
    };
    // commは括弧や空白を含められるため、最後の閉じ括弧より後の固定フィールドを読む。
    let tail = stat
        .rsplit_once(')')
        .ok_or("プロセス状態の形式が不正です")?
        .1;
    let mut fields = tail.split_whitespace();
    let state = fields.next().ok_or("プロセス状態がありません")?;
    let group = fields
        .nth(1)
        .ok_or("プロセスグループがありません")?
        .parse::<u32>()
        .map_err(|_| "プロセスグループが不正です")?;
    // 再照合時に他groupへ移ったPIDは停止対象としない。ゾンビのreapはその親の責務。
    Ok(group == expected_group && state != "Z" && state != "X")
}

#[cfg(target_os = "macos")]
fn member_running(pid: u32, expected_group: u32) -> Result<bool, String> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as _,
            size,
        )
    };
    if read != size {
        let error = std::io::Error::last_os_error();
        if read == 0 && error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err(format!("プロセス {pid} の状態を取得できません: {error}"));
    }
    Ok(info.pbi_pgid == expected_group && info.pbi_status != libc::SZOMB)
}

fn process_groups() -> Result<Vec<(u32, u32)>, String> {
    let mut groups = Vec::new();
    for pid in process_ids()? {
        let group = unsafe { libc::getpgid(pid as libc::pid_t) };
        if group < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(format!("プロセス {pid} の所属を確認できません: {error}"));
        }
        groups.push((pid, group as u32));
    }
    // 根はWNOWAITで番号を保持するため、走査で別groupを自分のものと誤認しない。
    // ただし全体の走査は原子的ではなく、走査中のfork/終了やsetsidでの離脱まで
    // Windows Job Objectと同じ強さで封じ込めるものではない。
    Ok(groups)
}

#[cfg(target_os = "linux")]
fn process_ids() -> Result<Vec<u32>, String> {
    let entries = std::fs::read_dir("/proc")
        .map_err(|error| format!("プロセス一覧を取得できません: {error}"))?;
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("プロセス一覧を読み取れません: {error}"))?;
        if let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        {
            ids.push(pid);
        }
    }
    Ok(ids)
}

#[cfg(target_os = "macos")]
fn process_ids() -> Result<Vec<u32>, String> {
    let mut capacity = 256usize;
    for _ in 0..8 {
        let mut ids = vec![0i32; capacity];
        let size = i32::try_from(std::mem::size_of_val(ids.as_slice()))
            .map_err(|_| "プロセス一覧が大きすぎます")?;
        let count = unsafe { libc::proc_listallpids(ids.as_mut_ptr().cast(), size) };
        if count <= 0 {
            return Err(format!(
                "プロセス一覧を取得できません: {}",
                std::io::Error::last_os_error()
            ));
        }
        if (count as usize) < capacity {
            ids.truncate(count as usize);
            return Ok(ids
                .into_iter()
                .filter(|id| *id > 0)
                .map(|id| id as u32)
                .collect());
        }
        capacity *= 2;
    }
    Err("プロセス一覧の大きさを確定できません".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 根の終了は再観測でき所有解放後と回収済み番号には停止要求しない() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "exit 7"]);
        let mut scope = ProcessScope::prepare(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        scope.set_id(child.id());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(status) = scope.peek_exit().unwrap() {
                assert_eq!(status.code(), Some(7));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(scope.peek_exit().unwrap().unwrap().code(), Some(7));
        child.wait().unwrap();
        // ECHILDなら保存番号へkillしない。失敗時も所有情報を捨てず診断を返す。
        assert!(scope
            .terminate()
            .unwrap_err()
            .contains("終了を確認できません"));
        scope.release().unwrap();
        scope.terminate().unwrap();
        scope.terminate().unwrap();
        assert!(!scope.is_owned());
    }
}
