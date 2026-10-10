// xtask/src/util.rs - 取得・展開・コピーの共通処理。
//
// 旧 `scripts/lib/fetch-util.mjs` の置き換え。取得は必ず
// 「一時ファイルへ書く → 完了してから rename」で行う。各取得処理は
// 「ファイルがあれば取得済み」と判断するため、中断で途中まで書けたファイルが残ると
// 壊れたモデルや実行ファイルが永久に「取得済み」として扱われてしまう。

use std::fs::{self, FileTimes, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};

const USER_AGENT: &str = "picoagent-setup";

/// 進捗や結果の1行ログ（旧スクリプトの console.log 相当）。
pub fn log(tag: &str, message: impl AsRef<str>) {
    println!("[{tag}] {}", message.as_ref());
}

pub fn warn(tag: &str, message: impl AsRef<str>) {
    eprintln!("[{tag}] 警告: {}", message.as_ref());
}

/// PicoAgent管理下の一時・旧ディレクトリを、存在する場合だけ確実に削除する。
pub fn cleanup_dir(path: &Path, label: &str) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .with_context(|| format!("{label}を掃除できません: {}", path.display()))?;
    }
    Ok(())
}

/// PicoAgent管理下の一時・旧ファイルを、存在する場合だけ確実に削除する。
pub fn cleanup_file(path: &Path, label: &str) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("{label}を掃除できません: {}", path.display()))?;
    }
    Ok(())
}

/// 同じフォルダの一時ファイルへ書き切ってから保存先を置き換える。
///
/// Windowsでは既存ファイルへのrenameができないため、先に退避してから差し替える。
/// 失敗時は元ファイルを戻し、成功時は一時・退避ファイルを残さない。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic_with(path, bytes, |from, to| fs::rename(from, to))
}

fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("保存先の親フォルダがありません"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    fs::create_dir_all(parent)?;
    let nonce = std::process::id();
    let temporary = parent.join(format!(".{name}.{nonce}.tmp"));
    let backup = parent.join(format!(".{name}.{nonce}.backup"));
    let result = (|| -> Result<()> {
        cleanup_file(&temporary, "前回の一時ファイル")?;
        anyhow::ensure!(
            !backup.exists(),
            "前回の退避ファイルが残っています。元ファイルを確認して復元してください: {}",
            backup.display()
        );
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            rename(path, &backup)?;
        }
        if let Err(error) = rename(&temporary, path) {
            if backup.exists() {
                rename(&backup, path).with_context(|| format!("置き換えに失敗しました: {error}。元ファイルの復元にも失敗しました。退避先: {}", backup.display()))?;
            }
            return Err(error.into());
        }
        cleanup_file(&backup, "置換済みファイルの退避")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// 別ボリュームだけ複製を許す。権限・使用中などは元の理由で停止する。
pub(crate) fn rename_or_copy(
    from: &Path,
    to: &Path,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    copy: impl FnOnce() -> Result<()>,
) -> Result<()> {
    match rename(from, to) {
        Ok(()) => Ok(()),
        // フォールバック許可: PR #470の別ボリューム間配置に限る複製。（根拠: PR #470 別ボリューム間の配置）
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => copy(),
        Err(error) => Err(error)
            .with_context(|| format!("配置できません: {} → {}", from.display(), to.display())),
    }
}

/// リポジトリのルート（`xtask/` の親）。
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask の親")
        .to_path_buf()
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .build()
        .context("HTTP クライアントを作れません")
}

/// URL の内容を `dest` へ取得する。**完了するまで `dest` は作られない。**
///
/// 途中で失敗した場合は一時ファイルを片付けるので、次回は最初から取り直せる。
pub fn download_to(url: &str, dest: &Path) -> Result<u64> {
    log("download", url);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("保存先を作れません: {}", parent.display()))?;
    }
    let temp = dest.with_extension(format!(
        "{}part",
        dest.extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));

    let result = (|| -> Result<u64> {
        // 数GBのモデルを低速回線でも取得できる余裕を持たせつつ、無期限には待たない。
        let mut response = client(Duration::from_secs(2 * 60 * 60))?
            .get(url)
            .send()
            .with_context(|| format!("取得に失敗しました: {url}"))?;
        if !response.status().is_success() {
            bail!("取得に失敗しました（{}）: {url}", response.status());
        }
        let mut file = fs::File::create(&temp)
            .with_context(|| format!("一時ファイルを作れません: {}", temp.display()))?;
        // ストリームでコピーするので、巨大なモデルでも全体をメモリへ載せない。
        let bytes = response.copy_to(&mut file)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, dest)
            .with_context(|| format!("保存先へ移動できません: {}", dest.display()))?;
        Ok(bytes)
    })();

    match result {
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            cleanup_file(&temp, "失敗した取得の一時ファイル")
                .with_context(|| format!("元の取得エラー: {error:#}"))?;
            Err(error)
        }
    }
}

/// GitHub API から JSON を取る（リリース情報の参照用）。
pub fn get_json(url: &str) -> Result<serde_json::Value> {
    let response = client(Duration::from_secs(60))?
        .get(url)
        .header("accept", "application/vnd.github+json")
        .send()
        .with_context(|| format!("取得に失敗しました: {url}"))?;
    if !response.status().is_success() {
        bail!("取得に失敗しました（{}）: {url}", response.status());
    }
    response.json().context("JSON として読めません")
}

/// アーカイブを `dir` へ展開する。
///
/// 展開は OS 標準のコマンドへ委譲する（旧スクリプトと同じ方針）。
/// Windows は zip を PowerShell の Expand-Archive、それ以外は tar / unzip。
pub fn extract(archive: &Path, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let name = archive.to_string_lossy().to_lowercase();
    let is_zip = name.ends_with(".zip");

    let status = if is_zip && cfg!(windows) {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", "Expand-Archive", "-LiteralPath"])
            .arg(archive)
            .args(["-DestinationPath"])
            .arg(dir)
            .arg("-Force")
            .status()
    } else if is_zip {
        Command::new("unzip")
            .arg("-q")
            .arg("-o")
            .arg(archive)
            .arg("-d")
            .arg(dir)
            .status()
    } else {
        Command::new("tar")
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(dir)
            .status()
    };

    let status =
        status.with_context(|| format!("展開コマンドを起動できません: {}", archive.display()))?;
    if !status.success() {
        bail!(
            "展開に失敗しました（終了コード {:?}）: {}",
            status.code(),
            archive.display()
        );
    }
    Ok(())
}

/// 指定コマンドが PATH にあるか。
pub fn command_exists(program: &str) -> bool {
    let probe = if cfg!(windows) { "where" } else { "which" };
    Command::new(probe)
        .arg(program)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 実行ファイル名（Windows は `.exe` を付ける）。
pub fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// ディレクトリを再帰的に探して、名前が一致する最初のファイルを返す。
pub fn find_file(dir: &Path, file_name: &str) -> Option<PathBuf> {
    find_file_inner(dir, file_name, 0)
}

fn find_file_inner(dir: &Path, file_name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 7 || !dir.exists() {
        return None;
    }
    let entries = fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.file_name().is_some_and(|name| name == file_name) {
            return Some(path);
        }
    }
    dirs.into_iter()
        .find_map(|sub| find_file_inner(&sub, file_name, depth + 1))
}

/// ディレクトリを丸ごとコピーする。`skip` が true を返すパスは除外する。
pub fn copy_dir(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        if skip(&source) {
            continue;
        }
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_dir(&source, &target, skip)?;
        } else {
            fs::copy(&source, &target)
                .with_context(|| format!("コピーできません: {}", source.display()))?;
        }
    }
    Ok(())
}

/// 差分同期で行った処理の件数と、内容が変わった相対パス。
#[derive(Debug, Default)]
pub struct SyncStats {
    pub copied_files: usize,
    pub unchanged_files: usize,
    pub removed_entries: usize,
    pub changed_paths: Vec<PathBuf>,
}

/// ディレクトリをサイズと更新日時で差分同期する。
///
/// `skip` が真の正本パスは同期対象から外し、配信先に残っていれば削除する。
/// `force` は正本の検証が済んだあとに呼び出し側から指定し、配信先を作り直す。
pub fn sync_dir(
    from: &Path,
    to: &Path,
    skip: &dyn Fn(&Path) -> bool,
    force: bool,
) -> Result<SyncStats> {
    sync_dir_with_equivalence(from, to, skip, force, &|_, _| Ok(false))
}

/// 通常のサイズ・更新日時比較に加え、用途固有の同値判定を使って差分同期する。
///
/// `equivalent` はメタデータが異なるファイルだけに呼ぶ。自動生成する版だけが違う
/// manifest のように、配信先をそのまま保つ必要がある場合に使う。
pub fn sync_dir_with_equivalence(
    from: &Path,
    to: &Path,
    skip: &dyn Fn(&Path) -> bool,
    force: bool,
    equivalent: &dyn Fn(&Path, &Path) -> Result<bool>,
) -> Result<SyncStats> {
    if !from.is_dir() {
        bail!("同期元ディレクトリが見つかりません: {}", from.display());
    }
    if force && to.exists() {
        fs::remove_dir_all(to)
            .with_context(|| format!("同期先を作り直せません: {}", to.display()))?;
    }
    let mut stats = SyncStats::default();
    sync_dir_inner(from, to, from, to, skip, equivalent, &mut stats)?;
    Ok(stats)
}

fn sync_dir_inner(
    from: &Path,
    to: &Path,
    from_root: &Path,
    to_root: &Path,
    skip: &dyn Fn(&Path) -> bool,
    equivalent: &dyn Fn(&Path, &Path) -> Result<bool>,
    stats: &mut SyncStats,
) -> Result<()> {
    if to.exists() && !to.is_dir() {
        fs::remove_file(to)?;
    }
    fs::create_dir_all(to)?;

    let mut expected = std::collections::HashSet::new();
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        if skip(&source) {
            continue;
        }
        expected.insert(entry.file_name());
        let target = to.join(entry.file_name());
        if source.is_dir() {
            if target.exists() && !target.is_dir() {
                fs::remove_file(&target)?;
                stats.removed_entries += 1;
                stats.changed_paths.push(relative_to(&target, to_root));
            }
            sync_dir_inner(
                &source, &target, from_root, to_root, skip, equivalent, stats,
            )?;
            continue;
        }

        let source_meta = fs::metadata(&source)?;
        let unchanged = if target.is_file() {
            let target_meta = fs::metadata(&target)?;
            (source_meta.len() == target_meta.len()
                && source_meta.modified()? == target_meta.modified()?)
                || equivalent(&source, &target)?
        } else {
            false
        };
        if unchanged {
            stats.unchanged_files += 1;
            continue;
        }
        if target.is_dir() {
            fs::remove_dir_all(&target)?;
            stats.removed_entries += 1;
        }
        fs::copy(&source, &target)
            .with_context(|| format!("コピーできません: {}", source.display()))?;
        // fs::copy の更新日時保持はOSごとに異なる。明示的に合わせ、次回比較を安定させる。
        let times = FileTimes::new().set_modified(source_meta.modified()?);
        OpenOptions::new()
            .write(true)
            .open(&target)?
            .set_times(times)?;
        stats.copied_files += 1;
        stats.changed_paths.push(relative_to(&source, from_root));
    }

    for entry in fs::read_dir(to)?.collect::<std::io::Result<Vec<_>>>()? {
        if expected.contains(&entry.file_name()) {
            continue;
        }
        let target = entry.path();
        if target.is_dir() {
            fs::remove_dir_all(&target)?;
        } else {
            fs::remove_file(&target)?;
        }
        stats.removed_entries += 1;
        stats.changed_paths.push(relative_to(&target, to_root));
    }
    Ok(())
}

fn relative_to(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// 配下のファイルを再帰的に列挙する。
pub fn list_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_files(dir, &mut out);
    out
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// 非 Windows で実行権限を付ける（取得・コピー直後の実行ファイル向け）。
pub fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("実行権限を読めません: {}", path.display()))?
            .permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("実行権限を設定できません: {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// MB 表記（ログ用）。
pub fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// 環境変数が "1" か。
pub fn flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

/// 環境変数（空なら既定値）。
pub fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 別ボリュームだけ複製し他の配置失敗では複製しない() {
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::AlreadyExists,
            std::io::ErrorKind::Other,
            std::io::ErrorKind::NotFound,
        ] {
            let result = rename_or_copy(
                Path::new("staging"),
                Path::new("dest"),
                |_, _| Err(kind.into()),
                || panic!("非EXDEVで複製してはいけない"),
            );
            assert_eq!(
                result
                    .unwrap_err()
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .kind(),
                kind
            );
        }
        let mut copied = 0;
        rename_or_copy(
            Path::new("staging"),
            Path::new("dest"),
            |_, _| Err(std::io::ErrorKind::CrossesDevices.into()),
            || {
                copied += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(copied, 1);
        rename_or_copy(
            Path::new("staging"),
            Path::new("dest"),
            |_, _| Ok(()),
            || panic!("移動成功なら複製しない"),
        )
        .unwrap();
    }

    #[test]
    fn 保存置換と復元が失敗したら両理由と元の退避先を保持する() {
        let root = repo_root()
            .join("temp")
            .join(format!("atomic-rollback-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("latest.json");
        fs::write(&path, b"original").unwrap();
        let mut calls = 0;
        let error = write_atomic_with(&path, b"next", |from, to| {
            calls += 1;
            match calls {
                1 => fs::rename(from, to),
                2 => Err(std::io::Error::other("置換故障")),
                _ => Err(std::io::Error::other("復元故障")),
            }
        })
        .unwrap_err();
        let message = format!("{error:#}");
        let backup = root.join(format!(".latest.json.{}.backup", std::process::id()));
        assert!(message.contains("置換故障") && message.contains("復元故障"));
        assert!(message.contains(backup.to_str().unwrap()));
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        assert_eq!(calls, 3);
        assert!(write_atomic(&path, b"next")
            .unwrap_err()
            .to_string()
            .contains("退避ファイル"));
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 差分同期は無変更を飛ばし追加編集削除と強制作り直しを扱う() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-differential-sync-{}",
            std::process::id()
        ));
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/edited.txt"), "old").unwrap();

        let first = sync_dir(&source, &target, &|_| false, false).unwrap();
        assert_eq!(first.copied_files, 1);
        let second = sync_dir(&source, &target, &|_| false, false).unwrap();
        assert_eq!(second.copied_files, 0);
        assert_eq!(second.unchanged_files, 1);

        fs::write(source.join("nested/edited.txt"), "new content").unwrap();
        fs::write(source.join("added.txt"), "added").unwrap();
        fs::write(target.join("deleted.txt"), "obsolete").unwrap();
        let changed = sync_dir(&source, &target, &|_| false, false).unwrap();
        assert_eq!(changed.copied_files, 2);
        assert_eq!(changed.removed_entries, 1);
        assert_eq!(
            fs::read_to_string(target.join("nested/edited.txt")).unwrap(),
            "new content"
        );
        assert!(target.join("added.txt").is_file());
        assert!(!target.join("deleted.txt").exists());

        let forced = sync_dir(&source, &target, &|_| false, true).unwrap();
        assert_eq!(forced.copied_files, 2);
        assert_eq!(forced.unchanged_files, 0);
        fs::remove_dir_all(root).unwrap();
    }
}
