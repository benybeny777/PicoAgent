// component_packages.rs - 軽量配布版が初回取得する追加機能パッケージを作る。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use zip::write::FileOptions;

use crate::util::{log, repo_root};

const COMPONENTS: [&str; 5] = ["aivis", "whisper", "tha3", "office", "tools"];

pub fn package_current_platform() -> Result<()> {
    let root = repo_root();
    let release: Value =
        serde_json::from_str(&fs::read_to_string(root.join("distribution/release.json"))?)?;
    let base_url = release
        .pointer("/components/baseUrl")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .context("distribution/release.json の components.baseUrl を設定してください")?;
    let output = root.join("distribution/components");
    fs::create_dir_all(&output)?;
    let destination = root.join("src-tauri/resources/component-manifest.json");
    let existing: Value = serde_json::from_str(&fs::read_to_string(&destination)?)?;
    let mut packages = existing
        .get("packages")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);
    for id in COMPONENTS {
        let source = root.join("src-tauri/resources/bundled").join(id);
        if !source.is_dir() {
            bail!("{id} の同梱元がありません。先に cargo xtask setup {id} 相当を実行してください");
        }
        if id == "tha3" && !source.join("runtime").is_dir() {
            bail!("THA3の専用ランタイムがありません。完全なruntimeを準備してからパッケージ化してください");
        }
        let key = package_key(id);
        let file_name = format!("{key}.zip");
        let archive = output.join(&file_name);
        write_zip(&source, &archive)?;
        let size = fs::metadata(&archive)?.len();
        let sha256 = file_sha256(&archive)?;
        packages.insert(
            key,
            json!({ "file": file_name, "sha256": sha256, "size": size }),
        );
        log(
            "package-components",
            format!("{id}: {} MB", size / 1024 / 1024),
        );
    }
    let manifest = json!({ "version": 1, "baseUrl": base_url, "packages": packages });
    let stage = destination.with_extension("json.tmp");
    fs::write(
        &stage,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    replace_file(&stage, &destination)?;
    Ok(())
}

fn package_key(id: &str) -> String {
    let os = if cfg!(windows) { "windows" } else { "macos" };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    format!("{id}-{os}-{arch}")
}

fn write_zip(source: &Path, archive: &Path) -> Result<()> {
    let stage = archive.with_extension("zip.tmp");
    if stage.exists() {
        fs::remove_file(&stage)?;
    }
    let result = write_zip_inner(source, &stage).and_then(|_| replace_file(&stage, archive));
    if result.is_err() && stage.exists() {
        // フォールバック許可: パッケージ作成失敗時の一時ファイル掃除では元のエラーを優先する。
        let _ = fs::remove_file(&stage);
    }
    result
}

fn write_zip_inner(source: &Path, stage: &Path) -> Result<()> {
    let file = fs::File::create(stage)?;
    let mut zip = zip::ZipWriter::new(file);
    for path in files(source)? {
        let relative = path
            .strip_prefix(source)?
            .to_string_lossy()
            .replace('\\', "/");
        zip.start_file(relative, file_options(&path)?)?;
        let mut input = fs::File::open(&path)?;
        std::io::copy(&mut input, &mut zip)?;
    }
    zip.finish()?.sync_all()?;
    Ok(())
}

fn file_options(path: &Path) -> Result<FileOptions> {
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return Ok(options.unix_permissions(fs::metadata(path)?.permissions().mode()));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(options)
    }
}

fn replace_file(stage: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        fs::rename(stage, destination)?;
        return Ok(());
    }
    let backup = destination.with_extension("backup");
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(destination, &backup)?;
    if let Err(error) = fs::rename(stage, destination) {
        if let Err(restore_error) = fs::rename(&backup, destination) {
            bail!("成果物の置換に失敗し、旧版の復元にも失敗しました: 置換={error} / 復元={restore_error}");
        }
        return Err(error.into());
    }
    fs::remove_file(backup)?;
    Ok(())
}

fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut result = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.is_file() {
                result.push(path);
            }
        }
    }
    result.sort();
    Ok(result)
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn test_path(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "picoagent-component-package-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn パッケージキーは対象osを含む() {
        let key = super::package_key("tools");
        assert!(key.starts_with("tools-"));
        assert!(key.ends_with("-x64") || key.ends_with("-arm64"));
    }

    #[test]
    fn 既存成果物を原子的に置き換える() {
        let destination = test_path("destination");
        let stage = test_path("stage");
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&stage, b"new").unwrap();
        super::replace_file(&stage, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(!stage.exists());
        std::fs::remove_file(destination).unwrap();
    }
}
