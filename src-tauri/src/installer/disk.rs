// disk - 保存先と同じボリュームで、利用者が使用できる空き容量を調べる。
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn existing_directory(path: &Path) -> Result<PathBuf> {
    let mut ancestor = std::path::absolute(path)?;
    loop {
        match std::fs::metadata(&ancestor) {
            Ok(metadata) => {
                if metadata.is_file() {
                    ancestor.pop();
                }
                return Ok(std::fs::canonicalize(ancestor)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                anyhow::ensure!(ancestor.pop(), "保存先のボリュームが見つかりません");
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub(crate) fn available_bytes(path: &Path) -> Result<u64> {
    let directory = existing_directory(path)
        .with_context(|| format!("保存先の空き容量を確認できません: {}", path.display()))?;
    platform_available(&directory)
}

#[cfg(windows)]
fn platform_available(path: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut available = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("保存先の空き容量を確認できません");
    }
    Ok(available)
}

#[cfg(unix)]
fn platform_available(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .context("保存先のパスにNULが含まれています")?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("保存先の空き容量を確認できません");
    }
    let info = unsafe { info.assume_init() };
    let bytes = u128::from(info.f_bavail) * u128::from(info.f_frsize);
    u64::try_from(bytes).context("空き容量が扱える整数範囲を超えています")
}

#[cfg(not(any(windows, unix)))]
fn platform_available(_path: &Path) -> Result<u64> {
    anyhow::bail!("このOSの空き容量検査に対応していません")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 未作成の保存先は同じ既存祖先の容量を調べる() {
        let root = super::super::fs::test_directory();
        let nested = root.path().join("未作成/保存先");
        assert_eq!(
            existing_directory(&nested).unwrap(),
            std::fs::canonicalize(root.path()).unwrap()
        );
        assert!(available_bytes(&nested).unwrap() > 0);
        assert!(!nested.exists());
    }
}
