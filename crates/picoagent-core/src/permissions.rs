// 実行属性の変更と、呼出用途ごとの失敗判定を一か所で扱う。
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutablePolicy {
    Strict,
    CheckAtSpawn,
}

#[derive(Debug)]
pub enum ExecutableOutcome {
    Ready,
    PermissionDenied(io::Error),
}

/// 導入時は失敗を返し、アプリの既存配置では権限拒否だけ実起動の判定へ渡す。
pub fn ensure_executable(path: &Path, policy: ExecutablePolicy) -> io::Result<ExecutableOutcome> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        if policy == ExecutablePolicy::CheckAtSpawn && permissions.mode() & 0o111 != 0 {
            return Ok(ExecutableOutcome::Ready);
        }
        permissions.set_mode(permissions.mode() | 0o755);
        permission_result(policy, std::fs::set_permissions(path, permissions))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, policy);
        Ok(ExecutableOutcome::Ready)
    }
}

#[cfg(any(unix, test))]
fn permission_result(
    policy: ExecutablePolicy,
    result: io::Result<()>,
) -> io::Result<ExecutableOutcome> {
    match result {
        Ok(()) => Ok(ExecutableOutcome::Ready),
        Err(error)
            if policy == ExecutablePolicy::CheckAtSpawn
                && error.kind() == io::ErrorKind::PermissionDenied =>
        {
            Ok(ExecutableOutcome::PermissionDenied(error))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 権限拒否だけアプリの実起動へ渡し導入と他の失敗は止める() {
        for policy in [ExecutablePolicy::Strict, ExecutablePolicy::CheckAtSpawn] {
            assert!(matches!(
                permission_result(policy, Ok(())),
                Ok(ExecutableOutcome::Ready)
            ));
            for kind in [
                io::ErrorKind::PermissionDenied,
                io::ErrorKind::NotFound,
                io::ErrorKind::Other,
            ] {
                let result =
                    permission_result(policy, Err(io::Error::new(kind, "検査用権限変更失敗")));
                if policy == ExecutablePolicy::CheckAtSpawn
                    && kind == io::ErrorKind::PermissionDenied
                {
                    let Ok(ExecutableOutcome::PermissionDenied(error)) = result else {
                        panic!("権限拒否の理由を起動側へ渡していません");
                    };
                    assert_eq!(error.kind(), kind);
                    assert!(error.to_string().contains("検査用権限変更失敗"));
                } else {
                    assert_eq!(result.unwrap_err().kind(), kind);
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn unixでは既存属性とアプリの既存実行bitを保持し導入時は必要属性を付ける() {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("picoagent-permission-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        for (initial, policy, expected) in [
            (0o600, ExecutablePolicy::CheckAtSpawn, 0o755),
            (0o100, ExecutablePolicy::CheckAtSpawn, 0o100),
            (0o100, ExecutablePolicy::Strict, 0o755),
            (0o640, ExecutablePolicy::Strict, 0o755),
            (0o775, ExecutablePolicy::Strict, 0o775),
        ] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(initial)).unwrap();
            assert!(matches!(
                ensure_executable(&path, policy),
                Ok(ExecutableOutcome::Ready)
            ));
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                expected
            );
        }
        drop(file);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            ensure_executable(&path, ExecutablePolicy::Strict)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn 非unixの実行属性変更は既存どおり不要() {
        for policy in [ExecutablePolicy::Strict, ExecutablePolicy::CheckAtSpawn] {
            assert!(matches!(
                ensure_executable(Path::new("存在しない検査用パス"), policy),
                Ok(ExecutableOutcome::Ready)
            ));
        }
    }
}
