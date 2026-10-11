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
            let _ = owned.child.kill();
            return Err(error);
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

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
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
        if !self.scope.root_attached() && self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
        if self.scope.is_owned() {
            if let Err(error) = self.terminate("所有プロセスの回収") {
                // Dropでは通知先がないため、通常経路は必ずterminateのResultを呼出元へ返す。
                crate::log_paths::log_detached(&error);
            }
        }
        // 登録前の失敗ではWindowsのJobに入っていない停止中の根も回収する。
        #[cfg(windows)]
        if self.child.try_wait().is_ok_and(|exit| exit.is_none()) {
            if self.child.kill().is_ok() {
                let _ = self.child.wait();
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
            let _ = owned.child.start_kill();
            return Err(SpawnFailure::Ownership(error));
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
            let _ = self.child.start_kill();
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
                        } else {
                            let _ = self.child.try_wait();
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
