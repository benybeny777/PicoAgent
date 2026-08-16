// xtask/src/util.rs - 取得・展開・コピーの共通処理。
//
// 旧 `scripts/lib/fetch-util.mjs` の置き換え。取得は必ず
// 「一時ファイルへ書く → 完了してから rename」で行う。各取得処理は
// 「ファイルがあれば取得済み」と判断するため、中断で途中まで書けたファイルが残ると
// 壊れたモデルや実行ファイルが永久に「取得済み」として扱われてしまう。

use std::fs;
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
pub fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mut perm = meta.permissions();
            perm.set_mode(perm.mode() | 0o755);
            // フォールバック許可: .app 配下など chmod できない場所は起動結果で判定する。
            let _ = fs::set_permissions(path, perm);
        }
    }
    #[cfg(not(unix))]
    let _ = path;
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
