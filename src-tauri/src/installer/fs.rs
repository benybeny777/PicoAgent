// fs - 管理先への複製と有限探索。リンク・特殊ファイルを辿らない。
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::Limits;

pub(super) fn normal_metadata(path: &Path) -> Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    validate_normal_metadata(path, &metadata)?;
    Ok(metadata)
}

/// 呼出元が取得した情報を使い、リンク・特殊ファイルの判定を一箇所に保つ。
pub(super) fn validate_normal_metadata(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        anyhow::ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "リンクやジャンクションは扱えません: {}",
            path.display()
        );
    }
    anyhow::ensure!(
        !metadata.file_type().is_symlink(),
        "シンボリックリンクは扱えません: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.is_file() || metadata.is_dir(),
        "通常のファイル・フォルダではありません: {}",
        path.display()
    );
    Ok(())
}

/// 既存の祖先も検査し、途中のリンクを経由した書込みを拒否する。
pub(crate) fn checked_directory(path: &Path) -> Result<PathBuf> {
    directory_location(path, true)
}

fn complete_directory_step(part: Component<'_>, current: &Path) -> bool {
    // Windowsの拡張ドライブ・UNC接頭辞は根を結合する前でもabsoluteになる。
    // 接頭辞だけをOSへ渡さず、RootDirを結合した完全な根から検査する。
    !matches!(part, Component::Prefix(_)) && current.is_absolute()
}

fn directory_location(path: &Path, create: bool) -> Result<PathBuf> {
    let mut pending = None::<PathBuf>;
    let absolute = std::path::absolute(path)?;
    let mut current = PathBuf::new();
    for part in absolute.components() {
        anyhow::ensure!(
            !matches!(part, Component::ParentDir),
            "保存先に .. は使えません"
        );
        current.push(part);
        if let Some(ref mut pending) = pending {
            pending.push(part);
            continue;
        }
        if !complete_directory_step(part, &current) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                validate_normal_metadata(&current, &metadata)?;
                anyhow::ensure!(
                    metadata.is_dir(),
                    "保存先がフォルダではありません: {}",
                    current.display()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if create {
                    std::fs::create_dir(&current)?;
                } else {
                    let parent = current.parent().context("保存先の祖先がありません")?;
                    pending = Some(
                        std::fs::canonicalize(parent)?
                            .join(current.file_name().context("保存先の名前がありません")?),
                    );
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    match pending {
        Some(path) => Ok(path),
        None => Ok(std::fs::canonicalize(absolute)?),
    }
}

pub(super) fn destination(root: &Path, relative: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        !relative.is_absolute()
            && relative
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "安全でない相対パスです: {}",
        relative.display()
    );
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        let resolved = checked_directory(parent)?;
        anyhow::ensure!(resolved.starts_with(root), "保存先が管理範囲の外です");
    }
    check_existing_target(&path)?;
    Ok(path)
}

fn check_existing_target(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_normal_metadata(path, &metadata)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// 単一ファイルの公開前に、祖先と既存の宛先を調べる。管理範囲は呼出元が確定する。
pub(super) fn checked_target(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().context("保存先の親がありません")?;
    let parent = checked_directory(parent)?;
    let target = parent.join(path.file_name().context("保存先の名前がありません")?);
    check_existing_target(&target)?;
    Ok(target)
}

pub(super) fn file_limit(limits: Limits, path: &Path) -> u64 {
    limits
        .file_limit
        .map_or(limits.max_file_bytes, |limit| limit(path))
}

/// 途中失敗で旧ファイルを破壊しない。同名許可時も実内容が一致することを調べる。
pub(super) fn write_file(
    reader: &mut impl Read,
    target: &Path,
    limit: u64,
    expected_size: u64,
    same_content: bool,
) -> Result<u64> {
    let parent = target.parent().context("保存先の親がありません")?;
    let mut pending = tempfile::NamedTempFile::new_in(parent)?;
    let copied = std::io::copy(&mut reader.take(limit.saturating_add(1)), &mut pending)?;
    anyhow::ensure!(
        copied <= limit,
        "展開・複製サイズが用途の上限を超えています"
    );
    anyhow::ensure!(copied == expected_size, "読込サイズと宣言が一致しません");
    pending.flush()?;
    if target.try_exists()? {
        anyhow::ensure!(
            normal_metadata(target)?.is_file() && same_content,
            "ファイル名が既存の配置と重複しています: {}",
            target.display()
        );
        anyhow::ensure!(
            same_files(pending.path(), target)?,
            "同名ファイルの内容が一致しません: {}",
            target.display()
        );
    } else {
        pending
            .persist_noclobber(target)
            .map_err(|error| error.error)?;
    }
    Ok(copied)
}

fn same_files(left: &Path, right: &Path) -> Result<bool> {
    if std::fs::metadata(left)?.len() != std::fs::metadata(right)?.len() {
        return Ok(false);
    }
    let mut left = std::fs::File::open(left)?;
    let mut right = std::fs::File::open(right)?;
    let mut a = [0; 65536];
    let mut b = [0; 65536];
    loop {
        let count = left.read(&mut a)?;
        if count == 0 {
            return Ok(true);
        }
        right.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
    }
}

pub(super) fn set_safe_permissions(path: &Path, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
        )?;
    }
    #[cfg(not(unix))]
    let _ = (path, executable);
    Ok(())
}

pub(crate) fn copy_dir(source: &Path, destination_path: &Path, limits: Limits) -> Result<()> {
    copy_directory(source, destination_path, limits, true)
}

pub(crate) fn copy_dir_plain(source: &Path, destination_path: &Path, limits: Limits) -> Result<()> {
    copy_directory(source, destination_path, limits, false)
}

fn copy_directory(
    source: &Path,
    destination_path: &Path,
    limits: Limits,
    preserve_execution: bool,
) -> Result<()> {
    anyhow::ensure!(
        normal_metadata(source)?.is_dir(),
        "コピー元がフォルダではありません"
    );
    let source = std::fs::canonicalize(source)?;
    let target = directory_location(destination_path, false)?;
    anyhow::ensure!(
        !source.starts_with(&target) && !target.starts_with(&source),
        "コピー元とコピー先が重なっています。別のフォルダを選んでください。"
    );
    #[cfg(not(unix))]
    let _ = preserve_execution;
    let target = checked_directory(&target)?;
    let mut visited = HashSet::from([source.clone()]);
    let mut queue = vec![(source.clone(), PathBuf::new())];
    let mut count = 0usize;
    let mut total = 0u64;
    while let Some((directory, relative)) = queue.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            count = count.checked_add(1).context("複製項目数が多すぎます")?;
            anyhow::ensure!(
                count <= limits.max_entries,
                "フォルダの項目が用途の上限を超えています"
            );
            let from = entry.path();
            let metadata = normal_metadata(&from)?;
            let resolved = std::fs::canonicalize(&from)?;
            anyhow::ensure!(
                resolved.starts_with(&source),
                "パック外を指すリンクやジャンクションは置けません: {}",
                from.display()
            );
            let relative = relative.join(entry.file_name());
            let to = destination(&target, &relative)?;
            if metadata.is_dir() {
                anyhow::ensure!(
                    visited.insert(resolved),
                    "同じフォルダを再び参照しています: {}",
                    from.display()
                );
                checked_directory(&to)?;
                queue.push((from, relative));
            } else {
                let limit = limits
                    .max_total_bytes
                    .saturating_sub(total)
                    .min(file_limit(limits, &to));
                anyhow::ensure!(
                    metadata.len() <= limit,
                    "フォルダの中身が用途の上限を超えています"
                );
                let copied = write_file(
                    &mut std::fs::File::open(&from)?,
                    &to,
                    limit,
                    metadata.len(),
                    false,
                )?;
                total = total
                    .checked_add(copied)
                    .context("複製サイズが多すぎます")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    set_safe_permissions(
                        &to,
                        preserve_execution && metadata.permissions().mode() & 0o111 != 0,
                    )?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn find_file(root: &Path, name: &str, max_depth: usize) -> Result<Option<PathBuf>> {
    anyhow::ensure!(
        normal_metadata(root)?.is_dir(),
        "探索元がフォルダではありません"
    );
    let root = std::fs::canonicalize(root)?;
    let mut queue = vec![(root.clone(), 0usize)];
    let mut visited = HashSet::from([root.clone()]);
    let mut found = Vec::new();
    while let Some((directory, depth)) = queue.pop() {
        let mut subdirectories = Vec::new();
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = normal_metadata(&path)?;
            let resolved = std::fs::canonicalize(&path)?;
            anyhow::ensure!(resolved.starts_with(&root), "探索先が管理範囲の外です");
            if metadata.is_file() && entry.file_name() == std::ffi::OsStr::new(name) {
                found.push(path.clone());
            }
            if metadata.is_dir() && depth < max_depth {
                anyhow::ensure!(visited.insert(resolved), "探索先のフォルダが循環しています");
                subdirectories.push((path, depth + 1));
            }
        }
        queue.extend(subdirectories.into_iter().rev());
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        count => anyhow::bail!("{name} の候補が{count}個あります。配置を一意に確定できません"),
    }
}

/// ファイルを開く責務だけを持ち、内容のハッシュ計算は中核の読取ストリームへ渡す。
pub(crate) fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    picoagent_core::hash::sha256_reader(&mut file)
}

/// chmod拒否を明示してから、既存配置の起動結果で実行可否を判定する。
pub(crate) fn prepare_executable(app: &tauri::AppHandle, path: &Path) -> anyhow::Result<()> {
    use picoagent_core::permissions::{ExecutableOutcome, ExecutablePolicy};
    static PERMISSIONS: crate::notice::FailureLatch = crate::notice::FailureLatch::new();
    let notification =
        match picoagent_core::permissions::ensure_executable(path, ExecutablePolicy::CheckAtSpawn)?
        {
            ExecutableOutcome::Ready => Ok(()),
            ExecutableOutcome::PermissionDenied(error) => {
                let reason = format!(
                "実行権限を変更できません（{}）: {error}。既存配置の実際の起動結果で判定します。",
                path.display()
            );
                crate::log_paths::append_app_log(app, &reason);
                Err(reason)
            }
        };
    PERMISSIONS.report(
        app,
        "executable-permissions",
        "実行権限の確認",
        notification,
        "実行権限を確認できました",
    );
    Ok(())
}

// OS標準tempがリンク経由でも、検査へ渡す入口は確定した実体パスにする。
#[cfg(test)]
pub(crate) fn test_directory() -> tempfile::TempDir {
    tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    const LIMITS: Limits = Limits {
        max_entries: 5,
        max_total_bytes: 16,
        max_file_bytes: 8,
        file_limit: None,
    };

    #[cfg(windows)]
    #[test]
    fn 通常と拡張ドライブの根と未作成保存先を同じ実体へ解決する() {
        let temp = test_directory();
        let extended = std::fs::canonicalize(temp.path()).unwrap();
        let text = extended.to_str().unwrap();
        let ordinary = match text.strip_prefix(r"\\?\UNC\") {
            Some(relative) => PathBuf::from(format!(r"\\{relative}")),
            None => PathBuf::from(text.strip_prefix(r"\\?\").unwrap()),
        };
        for path in [&ordinary, &extended] {
            assert_eq!(checked_directory(path).unwrap(), extended);
            let root = path.ancestors().last().unwrap();
            assert_eq!(
                checked_directory(root).unwrap(),
                std::fs::canonicalize(root).unwrap()
            );
            let target = path.join("new/child");
            assert_eq!(
                directory_location(&target, false).unwrap(),
                extended.join("new/child")
            );
            assert!(!target.exists());
        }
        assert_eq!(
            checked_directory(&extended.join("new/child")).unwrap(),
            extended.join("new/child")
        );
    }

    #[cfg(windows)]
    #[test]
    fn ドライブとuncの祖先検査は未完成接頭辞を渡さず根を保持する() {
        for (path, root) in [
            (r"C:\folder", r"C:\"),
            (r"\\?\C:\folder", r"\\?\C:\"),
            (r"\\server\share\folder", r"\\server\share\"),
            (r"\\?\UNC\server\share\folder", r"\\?\UNC\server\share\"),
        ] {
            let mut current = PathBuf::new();
            let mut checked = Vec::new();
            for part in Path::new(path).components() {
                current.push(part);
                if complete_directory_step(part, &current) {
                    checked.push(current.clone());
                }
            }
            assert_eq!(
                checked,
                [PathBuf::from(root), PathBuf::from(path)],
                "{path}"
            );
        }
    }

    #[test]
    fn 複製は用途上限と元先重なりを守り既存ファイルを壊さない() {
        let temp = test_directory();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("one"), b"original").unwrap();
        let target = temp.path().join("target");
        copy_dir(&source, &target, LIMITS).unwrap();
        assert_eq!(std::fs::read(target.join("one")).unwrap(), b"original");
        std::fs::write(source.join("one"), b"changed").unwrap();
        assert!(copy_dir(&source, &target, LIMITS).is_err());
        assert_eq!(std::fs::read(target.join("one")).unwrap(), b"original");
        assert!(copy_dir(&source, &source.join("nested"), LIMITS).is_err());
        assert!(!source.join("nested").exists());
        for limits in [
            Limits {
                max_entries: 0,
                ..LIMITS
            },
            Limits {
                max_total_bytes: 1,
                ..LIMITS
            },
            Limits {
                max_file_bytes: 1,
                ..LIMITS
            },
        ] {
            let output = test_directory();
            assert!(copy_dir(&source, output.path(), limits).is_err());
            assert!(!output.path().join("one").exists());
        }
    }

    #[test]
    fn 探索は従来の深さ境界と大小文字を保ち読込不能を不存在扱いしない() {
        let temp = test_directory();
        let nested = temp.path().join("one/two");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("uv");
        std::fs::write(&file, b"tool").unwrap();
        assert_eq!(find_file(temp.path(), "uv", 2).unwrap(), Some(file));
        assert_eq!(find_file(temp.path(), "uv", 1).unwrap(), None);
        assert_eq!(find_file(temp.path(), "UV", 2).unwrap(), None);
        assert!(find_file(&temp.path().join("missing"), "uv", 2).is_err());
    }

    #[test]
    fn 探索は直下の候補が見つかっても奥の同名候補を見逃さない() {
        let temp = test_directory();
        std::fs::write(temp.path().join("uv"), b"first").unwrap();
        let nested = temp.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("uv"), b"second").unwrap();
        assert_eq!(
            find_file(temp.path(), "uv", 0).unwrap(),
            Some(temp.path().join("uv"))
        );
        let error = find_file(temp.path(), "uv", 1).unwrap_err().to_string();
        assert!(error.contains("候補が2個"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn 通常複製だけ実行属性を保持し追加キャラ複製は実行権を付与しない() {
        use std::os::unix::fs::PermissionsExt;
        let temp = test_directory();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("uv"), b"tool").unwrap();
        std::fs::set_permissions(source.join("uv"), std::fs::Permissions::from_mode(0o6755))
            .unwrap();
        for (plain, expected) in [(false, 0o755), (true, 0o644)] {
            let target = temp.path().join(if plain { "plain" } else { "exec" });
            if plain {
                copy_dir_plain(&source, &target, LIMITS).unwrap();
            } else {
                copy_dir(&source, &target, LIMITS).unwrap();
            }
            assert_eq!(
                std::fs::metadata(target.join("uv"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                expected
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn 外リンクと内部リンクと壊れた保存先リンクを辿らない() {
        use std::os::unix::fs::symlink;
        let temp = test_directory();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        symlink(temp.path(), source.join("outside")).unwrap();
        assert!(copy_dir(&source, &temp.path().join("target"), LIMITS).is_err());
        assert!(find_file(&source, "uv", 8).is_err());
        std::fs::remove_file(source.join("outside")).unwrap();
        symlink(&source, source.join("inside")).unwrap();
        assert!(copy_dir(&source, &temp.path().join("target"), LIMITS).is_err());
        let broken = temp.path().join("broken");
        symlink(temp.path().join("missing"), &broken).unwrap();
        assert!(checked_target(&broken).is_err());
        assert!(checked_target(&broken.join("file")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn ジャンクションを複製探索公開のいずれでも辿らない() {
        let temp = test_directory();
        let source = temp.path().join("source");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("uv"), b"external").unwrap();
        let link = source.join("junction");
        let result = crate::tools::process::command_sync("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "ジャンクション検査用の配置を作れません"
        );
        assert!(copy_dir(&source, &temp.path().join("target"), LIMITS).is_err());
        assert!(find_file(&source, "uv", 8).is_err());
        assert!(checked_target(&link.join("uv")).is_err());
        assert_eq!(std::fs::read(outside.join("uv")).unwrap(), b"external");
        std::fs::remove_dir(link).unwrap();
    }
}
