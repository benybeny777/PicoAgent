//! 起動した範囲の所有。根の終了確認と、子孫の停止能力の解放を分ける。

use std::io;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "ownership_windows.rs"]
mod platform;
#[cfg(unix)]
#[path = "ownership_unix.rs"]
mod platform;

use platform::ProcessScope;
pub(crate) use platform::ProcessSnapshot;

const STOP_WAIT: Duration = Duration::from_secs(3);

fn with_root_stop_error(error: String, stop: io::Result<()>) -> String {
    match stop {
        Ok(()) => error,
        Err(cleanup) => format!("{error}。起動した根の停止にも失敗しました: {cleanup}"),
    }
}

#[cfg(windows)]
fn stop_unattached_root<C>(
    child: &mut C,
    stop: fn(&mut C) -> io::Result<()>,
    try_wait: fn(&mut C) -> io::Result<Option<ExitStatus>>,
) -> Result<(), String> {
    let observe = |child: &mut C| {
        try_wait(child).map_err(|error| format!("未登録の根の終了を確認できません: {error}"))
    };
    if observe(child)?.is_some() {
        return Ok(());
    }
    // 停止要求に失敗した根をwaitすると、停止中のまま永久に戻らなくなる。
    stop(child).map_err(|error| format!("未登録の根を停止できません: {error}"))?;
    let deadline = Instant::now() + STOP_WAIT;
    loop {
        if observe(child)?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("未登録の根の終了を3秒以内に確認できません".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) enum SpawnFailure {
    Spawn(io::Error),
    Ownership(String),
}

/// 生のChildを公開しない。Unixのwait/reapが所有者を飛び越えるとPGIDを保護できない。
pub(crate) struct OwnedChild {
    child: std::process::Child,
    scope: ProcessScope,
}

impl OwnedChild {
    pub(crate) fn spawn(command: &mut std::process::Command) -> Result<Self, String> {
        let mut scope = ProcessScope::prepare(command)?;
        let child = command.spawn().map_err(|error| error.to_string())?;
        scope.set_id(child.id());
        let mut owned = Self { child, scope };
        if let Err(error) = owned.attach() {
            // Job登録前の失敗でも、停止中の根を先に終了させてから所有者を破棄する。
            return Err(with_root_stop_error(error, owned.child.kill()));
        }
        Ok(owned)
    }

    fn attach(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            self.scope.attach(self.child.as_raw_handle())?;
        }
        Ok(())
    }

    pub(crate) fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// 根だけの終了は観測に留め、所有する子孫が残る間は完了にしない。
    pub(crate) fn try_complete(
        &mut self,
        snapshot: &mut ProcessSnapshot,
    ) -> Result<Option<ExitStatus>, String> {
        let exited = self.try_root_exit()?;
        let Some(status) = exited else {
            return Ok(None);
        };
        if self.scope.has_descendants(snapshot)? {
            return Ok(None);
        }
        self.scope.release()?;
        self.child.wait().map_err(|error| error.to_string())?;
        Ok(Some(status))
    }

    pub(crate) fn try_root_exit(&mut self) -> Result<Option<ExitStatus>, String> {
        #[cfg(windows)]
        return self.child.try_wait().map_err(|error| error.to_string());
        #[cfg(unix)]
        self.scope.peek_exit()
    }

    pub(crate) fn terminate(&mut self, label: &str) -> Result<(), String> {
        self.scope.terminate()?;
        let deadline = Instant::now() + STOP_WAIT;
        loop {
            // 全体の停止確認まで根の番号を保持し、reapより先にsignal用の所有権を解放する。
            if self.scope.stopped()? {
                self.scope.release()?;
                self.child.wait().map_err(|error| error.to_string())?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("{label}の停止完了を3秒以内に確認できません"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.scope.root_attached() {
            if let Err(error) = stop_unattached_root(
                &mut self.child,
                std::process::Child::kill,
                std::process::Child::try_wait,
            ) {
                crate::log_paths::log_detached(&error);
            }
            // 空のJobの停止確認を根の停止確認として扱わない。Jobの解放はscopeへ任せる。
            return;
        }
        if self.scope.is_owned() {
            if let Err(error) = self.terminate("所有プロセスの回収") {
                // Dropでは通知先がないため、通常経路は必ずterminateのResultを呼出元へ返す。
                crate::log_paths::log_detached(&error);
            }
        }
    }
}

pub(crate) struct OwnedAsyncChild {
    child: tokio::process::Child,
    scope: ProcessScope,
}

impl OwnedAsyncChild {
    pub(crate) fn spawn(command: &mut tokio::process::Command) -> Result<Self, SpawnFailure> {
        // 根の所有確認に失敗した後、Tokioが保存PIDだけでkillする経路を作らない。
        command.kill_on_drop(false);
        let mut scope =
            ProcessScope::prepare(command.as_std_mut()).map_err(SpawnFailure::Ownership)?;
        let child = command.spawn().map_err(SpawnFailure::Spawn)?;
        let id = child.id().ok_or_else(|| {
            SpawnFailure::Ownership("起動したプロセスのIDを取得できません".into())
        })?;
        scope.set_id(id);
        let mut owned = Self { child, scope };
        if let Err(error) = owned.attach() {
            return Err(SpawnFailure::Ownership(with_root_stop_error(
                error,
                owned.child.start_kill(),
            )));
        }
        Ok(owned)
    }

    fn attach(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        self.scope.attach(
            self.child
                .raw_handle()
                .ok_or("起動したプロセスのハンドルを取得できません")?,
        )?;
        Ok(())
    }

    pub(crate) fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    pub(crate) async fn wait(&mut self) -> Result<ExitStatus, String> {
        #[cfg(windows)]
        return self.child.wait().await.map_err(|error| error.to_string());
        #[cfg(unix)]
        {
            // 通知を先に登録してから観測する。Child::waitは呼ばず、PGIDをreapで解放しない。
            let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::child())
                .map_err(|error| error.to_string())?;
            loop {
                if let Some(status) = self.scope.peek_exit()? {
                    return Ok(status);
                }
                signal
                    .recv()
                    .await
                    .ok_or("子プロセスの終了通知が閉じました")?;
            }
        }
    }

    /// 正常EOFでは、コマンドが意図して起動したアプリ等の継続を維持する。
    pub(crate) async fn release(&mut self) -> Result<(), String> {
        self.scope.release()?;
        self.child.wait().await.map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) async fn terminate(&mut self, label: &str) -> Result<(), String> {
        self.scope.terminate()?;
        tokio::time::timeout(STOP_WAIT, async {
            while !self.scope.stopped()? {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.scope.release()?;
            self.child.wait().await.map_err(|error| error.to_string())?;
            Ok(())
        })
        .await
        .map_err(|_| format!("{label}の停止完了を3秒以内に確認できません"))?
    }
}

impl Drop for OwnedAsyncChild {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.scope.root_attached() {
            if let Err(error) = stop_unattached_root(
                &mut self.child,
                tokio::process::Child::start_kill,
                tokio::process::Child::try_wait,
            ) {
                crate::log_paths::log_detached(&error);
            }
            return;
        }
        if self.scope.is_owned() {
            if let Err(error) = self.scope.terminate() {
                crate::log_paths::log_detached(&error);
                return;
            }
            // 通常はworkerが非同期に回収済み。panicやruntime終了でも、終了確認前に
            // ChildのDropが根をreapしてPGIDを再利用させないよう最後の回収を行う。
            let deadline = Instant::now() + STOP_WAIT;
            loop {
                match self.scope.stopped() {
                    Ok(true) => {
                        if let Err(error) = self.scope.release() {
                            crate::log_paths::log_detached(&error);
                        } else if let Err(error) = self.child.try_wait() {
                            crate::log_paths::log_detached(&format!(
                                "所有プロセスの終了状態を回収できません: {error}"
                            ));
                        }
                        break;
                    }
                    Err(error) => {
                        crate::log_paths::log_detached(&error);
                        break;
                    }
                    Ok(false) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Ok(false) => {
                        crate::log_paths::log_detached(
                            "所有プロセスの破棄時に停止完了を確認できません",
                        );
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
impl OwnedChild {
    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn 根の停止要求に失敗したら終了待機へ進まない() {
        let mut observations = 0;
        let error = super::stop_unattached_root(
            &mut observations,
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "検査用の停止拒否",
                ))
            },
            |observations| {
                *observations += 1;
                assert_eq!(*observations, 1, "停止要求に失敗した根を待機しました");
                Ok(None)
            },
        )
        .unwrap_err();
        assert!(error.contains("検査用の停止拒否"));
    }

    #[test]
    fn 根が終了済みなら停止要求を再送しない() {
        use std::os::windows::process::ExitStatusExt;
        super::stop_unattached_root(
            &mut (),
            |_| panic!("終了済みの根へ停止要求を再送しました"),
            |_| Ok(Some(std::process::ExitStatus::from_raw(0))),
        )
        .unwrap();
    }
}
