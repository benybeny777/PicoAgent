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
const RELEASE_PART_BYTES: u64 = 1_900_000_000;

struct ZipRoot {
    source: PathBuf,
    prefix: &'static str,
}

pub fn package_current_platform() -> Result<()> {
    std::thread::Builder::new()
        .name("component-packager".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(package_current_platform_inner)?
        .join()
        .map_err(|_| anyhow::anyhow!("追加機能パッケージ作成スレッドが異常終了しました"))?
}

pub fn verify_current_platform() -> Result<()> {
    std::thread::Builder::new()
        .name("component-verifier".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(verify_current_platform_inner)?
        .join()
        .map_err(|_| anyhow::anyhow!("追加機能パッケージ検証スレッドが異常終了しました"))?
}

fn verify_current_platform_inner() -> Result<()> {
    let root = repo_root();
    let output = root.join("distribution/components");
    let manifest: Value = serde_json::from_str(&fs::read_to_string(
        root.join("src-tauri/resources/component-manifest.json"),
    )?)?;
    let packages = manifest["packages"]
        .as_object()
        .context("追加機能マニフェストのpackagesがありません")?;
    for id in COMPONENTS {
        let key = package_key(id);
        let package = packages
            .get(&key)
            .with_context(|| format!("{key} がマニフェストにありません"))?;
        let expected_size = package["size"].as_u64().context("sizeがありません")?;
        let expected_sha = package["sha256"].as_str().context("sha256がありません")?;
        let parts = package["parts"].as_array().context("partsがありません")?;
        let paths = if parts.is_empty() {
            vec![output.join(package["file"].as_str().context("fileがありません")?)]
        } else {
            let mut paths = Vec::with_capacity(parts.len());
            for part in parts {
                let path = output.join(part["file"].as_str().context("分割fileがありません")?);
                anyhow::ensure!(
                    fs::metadata(&path)?.len()
                        == part["size"].as_u64().context("分割sizeがありません")?,
                    "{} のサイズが一致しません",
                    path.display()
                );
                anyhow::ensure!(
                    file_sha256(&path)?
                        == part["sha256"].as_str().context("分割sha256がありません")?,
                    "{} のSHA-256が一致しません",
                    path.display()
                );
                paths.push(path);
            }
            paths
        };
        let actual_size = paths.iter().try_fold(0_u64, |sum, path| {
            Ok::<_, anyhow::Error>(sum + fs::metadata(path)?.len())
        })?;
        anyhow::ensure!(
            actual_size == expected_size,
            "{key} の合計サイズが一致しません"
        );
        anyhow::ensure!(
            files_sha256(&paths)? == expected_sha,
            "{key} の連結SHA-256が一致しません"
        );
        log(
            "verify-components",
            format!("{key}: OK（{} MB）", actual_size / 1024 / 1024),
        );
    }
    Ok(())
}

fn package_current_platform_inner() -> Result<()> {
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
        let roots = component_roots(&root, id, &source)?;
        let key = package_key(id);
        let file_name = format!("{key}.zip");
        let archive = output.join(format!(".{key}.full.tmp"));
        log("package-components", format!("{id}: ZIP作成を開始します"));
        write_zip(&roots, &archive)?;
        let size = fs::metadata(&archive)?.len();
        let sha256 = file_sha256(&archive)?;
        let (file, parts) = publish_archive(&archive, &output, &key)?;
        let published_file_count = if parts.is_empty() { 1 } else { parts.len() };
        packages.insert(
            key,
            json!({ "file": file, "sha256": sha256, "size": size, "parts": parts }),
        );
        log(
            "package-components",
            format!(
                "{id}: {} MB / 公開ファイル {}件（基準名: {file_name}）",
                size / 1024 / 1024,
                published_file_count
            ),
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

fn component_roots(root: &Path, id: &str, source: &Path) -> Result<Vec<ZipRoot>> {
    let mut roots = vec![ZipRoot {
        source: source.to_path_buf(),
        prefix: "",
    }];
    if id != "tha3" {
        return Ok(roots);
    }
    if source.join("runtime").is_dir() {
        return Ok(roots);
    }
    let runtime = root.join("tools/tha3-runtime/runtime");
    anyhow::ensure!(
        runtime.join("python").is_dir() && runtime.join("packages/torch/__init__.py").is_file(),
        "THA3の専用ランタイムがありません。先に cargo xtask setup tha3 を実行してください"
    );
    roots.push(ZipRoot {
        source: runtime,
        prefix: "runtime",
    });
    Ok(roots)
}

fn write_zip(roots: &[ZipRoot], archive: &Path) -> Result<()> {
    let stage = archive.with_extension("zip.tmp");
    if stage.exists() {
        fs::remove_file(&stage)?;
    }
    let result = write_zip_inner(roots, &stage).and_then(|_| replace_file(&stage, archive));
    if result.is_err() && stage.exists() {
        // フォールバック許可: パッケージ作成失敗時の一時ファイル掃除では元のエラーを優先する。
        let _ = fs::remove_file(&stage);
    }
    result
}

fn write_zip_inner(roots: &[ZipRoot], stage: &Path) -> Result<()> {
    let file = fs::File::create(stage)?;
    let mut zip = zip::ZipWriter::new(file);
    for root in roots {
        for path in files(&root.source)? {
            let relative = path
                .strip_prefix(&root.source)?
                .to_string_lossy()
                .replace('\\', "/");
            let entry = if root.prefix.is_empty() {
                relative
            } else {
                format!("{}/{relative}", root.prefix)
            };
            zip.start_file(entry, file_options(&path)?)?;
            let mut input = fs::File::open(&path)?;
            std::io::copy(&mut input, &mut zip)?;
        }
    }
    zip.finish()?.sync_all()?;
    Ok(())
}

fn publish_archive(archive: &Path, output: &Path, key: &str) -> Result<(String, Vec<Value>)> {
    publish_archive_with_limit(archive, output, key, RELEASE_PART_BYTES)
}

fn publish_archive_with_limit(
    archive: &Path,
    output: &Path,
    key: &str,
    part_bytes: u64,
) -> Result<(String, Vec<Value>)> {
    let size = fs::metadata(archive)?.len();
    if size <= part_bytes {
        remove_old_assets(output, key)?;
        let name = format!("{key}.zip");
        replace_file(archive, &output.join(&name))?;
        return Ok((name, Vec::new()));
    }

    let mut input = fs::File::open(archive)?;
    let mut staged = Vec::new();
    let mut parts = Vec::new();
    let mut index = 1_u32;
    loop {
        let name = format!("{key}.zip.{index:03}");
        let stage = output.join(format!(".{name}.tmp"));
        let mut target = fs::File::create(&stage)?;
        let copied = std::io::copy(&mut input.by_ref().take(part_bytes), &mut target)?;
        if copied == 0 {
            fs::remove_file(stage)?;
            break;
        }
        target.sync_all()?;
        parts.push(json!({
            "file": name,
            "sha256": file_sha256(&stage)?,
            "size": copied,
        }));
        staged.push(stage);
        index += 1;
    }
    remove_old_assets(output, key)?;
    for (stage, part) in staged.iter().zip(&parts) {
        let name = part["file"]
            .as_str()
            .context("分割ファイル名がありません")?;
        fs::rename(stage, output.join(name))?;
    }
    fs::remove_file(archive)?;
    Ok((String::new(), parts))
}

fn remove_old_assets(output: &Path, key: &str) -> Result<()> {
    let prefix = format!("{key}.zip");
    for entry in fs::read_dir(output)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == prefix || name.starts_with(&format!("{prefix}.")))
        {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn file_options(path: &Path) -> Result<FileOptions> {
    // 速度優先: 数GB・2万ファイル超のランタイムを毎回Deflateすると公開作業が
    // 十数分止まる。Release側で分割上限を守るため、ZIPは格納のみとする。
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
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
    files_sha256(&[path.to_path_buf()])
}

fn files_sha256(paths: &[PathBuf]) -> Result<String> {
    let mut digest = Sha256::new();
    for path in paths {
        let mut input = fs::File::open(path)?;
        let mut buffer = [0_u8; 1024 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
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

    #[test]
    fn 大きい成果物を公開上限以下へ分割する() {
        let output = test_path("split-output");
        std::fs::create_dir_all(&output).unwrap();
        let archive = output.join("full.tmp");
        std::fs::write(&archive, b"0123456789").unwrap();
        let (file, parts) =
            super::publish_archive_with_limit(&archive, &output, "sample", 4).unwrap();
        assert!(file.is_empty());
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0]["size"], 4);
        assert_eq!(parts[2]["size"], 2);
        assert!(output.join("sample.zip.001").is_file());
        assert!(!archive.exists());
        std::fs::remove_dir_all(output).unwrap();
    }
}
