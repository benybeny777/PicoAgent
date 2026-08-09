// components.rs - 初回起動と設定画面で共用する追加コンポーネント管理。
//
// 開発版・従来配布版ではアプリ資源内の同梱物をそのまま使い、軽量配布版では
// component-manifest.json に記録したパッケージを利用者データ領域へ原子的に導入する。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

const MAX_PACKAGE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
const COMPONENT_IDS: [&str; 5] = ["aivis", "whisper", "tha3", "office", "tools"];
static RUNTIME_ROOT: OnceLock<PathBuf> = OnceLock::new();

struct InstallCleanup {
    archive: PathBuf,
    stage: PathBuf,
    armed: bool,
}

struct InstallPaths {
    cache: PathBuf,
    archive: PathBuf,
    destination: PathBuf,
    stage: PathBuf,
}

impl Drop for InstallCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // フォールバック許可: 失敗・中断時の一時物掃除なので、元のエラーや終了を優先する。
        let _ = std::fs::remove_file(&self.archive);
        let _ = std::fs::remove_dir_all(&self.stage);
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    base_url: String,
    packages: HashMap<String, Package>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Package {
    file: String,
    sha256: String,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStatus {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    size_label: &'static str,
    ready: bool,
    source: &'static str,
    available: bool,
    removable: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: String,
    stage: String,
    message: String,
    downloaded: u64,
    total: u64,
}

pub fn init(app: &AppHandle) {
    let _ = RUNTIME_ROOT.set(crate::log_paths::bundled_dir(app));
}

pub fn runtime_root() -> Option<&'static Path> {
    RUNTIME_ROOT.get().map(PathBuf::as_path)
}

fn definitions(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match id {
        "aivis" => Some((
            "音声読み上げ",
            "AivisSpeechでキャラクターの声を作ります。導入しても読み上げは自動でONになりません。",
            "約0.8GB＋音声モデル",
        )),
        "whisper" => Some((
            "音声入力・文字起こし",
            "Whisperを端末内で動かし、録音や音声ファイルを文字にします。",
            "約0.5GB",
        )),
        "tha3" => Some((
            "1枚絵からキャラ作成",
            "THA3で透過PNGから口パク・まばたきする2Dキャラを作ります。",
            "約5.2GB（Windows）",
        )),
        "office" => Some((
            "Office変換",
            "OfficeCLIでWord・Excel・PowerPointの変換やPDF出力を補助します。",
            "約1.5GB",
        )),
        "tools" => Some((
            "PDF・画像・文書ツール",
            "Pandoc、FFmpeg、qpdf、Poppler、ImageMagick、ripgrepを追加します。",
            "約1.0GB",
        )),
        _ => None,
    }
}

fn manifest(app: &AppHandle) -> Result<Manifest> {
    let resource = app
        .path()
        .resource_dir()
        .context("アプリ資源フォルダを取得できません")?
        .join("resources/component-manifest.json");
    let text = if resource.is_file() {
        std::fs::read_to_string(&resource)
            .with_context(|| format!("{} を読めません", resource.display()))?
    } else {
        include_str!("../resources/component-manifest.json").to_string()
    };
    let parsed: Manifest =
        serde_json::from_str(&text).context("追加機能マニフェストが壊れています")?;
    anyhow::ensure!(parsed.version == 1, "未対応の追加機能マニフェストです");
    Ok(parsed)
}

fn packaged_dir(app: &AppHandle, id: &str) -> Option<PathBuf> {
    let resources = app.path().resource_dir().ok()?;
    [
        resources.join("resources/bundled").join(id),
        resources.join("bundled").join(id),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

fn development_dir(id: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    [
        cwd.join("src-tauri/resources/bundled").join(id),
        cwd.join("resources/bundled").join(id),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

fn installed_dir(app: &AppHandle, id: &str) -> PathBuf {
    crate::log_paths::bundled_dir(app).join(id)
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

fn package_for(manifest: &Manifest, id: &str) -> Option<Package> {
    manifest.packages.get(&package_key(id)).cloned()
}

pub fn statuses(app: &AppHandle) -> Result<Vec<ComponentStatus>> {
    let manifest = manifest(app)?;
    Ok(COMPONENT_IDS
        .into_iter()
        .filter_map(|id| {
            let (label, description, size_label) = definitions(id)?;
            let installed = installed_dir(app, id).is_dir();
            let packaged = packaged_dir(app, id).is_some() || development_dir(id).is_some();
            Some(ComponentStatus {
                id,
                label,
                description,
                size_label,
                ready: installed || packaged,
                source: if installed {
                    "installed"
                } else if packaged {
                    "bundled"
                } else {
                    "missing"
                },
                available: packaged || package_for(&manifest, id).is_some(),
                removable: installed,
            })
        })
        .collect())
}

pub async fn remove(app: &AppHandle, id: &str) -> Result<Vec<ComponentStatus>> {
    anyhow::ensure!(COMPONENT_IDS.contains(&id), "未知の追加機能です: {id}");
    if id == "aivis" {
        crate::aivis::stop(app);
    }
    let target = installed_dir(app, id);
    let label = definitions(id).map(|item| item.0).unwrap_or(id).to_string();
    emit_progress(app, id, "remove", "削除中…", 0, 0);
    tokio::task::spawn_blocking(move || {
        if target.exists() {
            std::fs::remove_dir_all(&target).with_context(|| {
                format!("{label} を削除できません。使用中の機能を終了して再試行してください")
            })?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    statuses(app)
}

fn emit_progress(
    app: &AppHandle,
    id: &str,
    stage: &str,
    message: impl Into<String>,
    downloaded: u64,
    total: u64,
) {
    let _ = app.emit(
        "component:progress",
        Progress {
            id: id.to_string(),
            stage: stage.to_string(),
            message: message.into(),
            downloaded,
            total,
        },
    );
}

pub async fn install(app: &AppHandle, ids: &[String]) -> Result<Vec<ComponentStatus>> {
    let manifest = manifest(app)?;
    for id in ids {
        anyhow::ensure!(
            COMPONENT_IDS.contains(&id.as_str()),
            "未知の追加機能です: {id}"
        );
        let installed = installed_dir(app, id).is_dir();
        let bundled = packaged_dir(app, id).is_some() || development_dir(id).is_some();
        let package = package_for(&manifest, id);
        if bundled || (installed && package.is_none()) {
            emit_progress(app, id, "ready", "準備済みです", 0, 0);
            continue;
        }
        let package = package
            .with_context(|| format!("{id} はこのOS向けの取得先がまだ公開されていません"))?;
        if installed {
            emit_progress(app, id, "install", "再導入して修復します…", 0, package.size);
        }
        install_package(app, &manifest, id, &package).await?;
    }
    statuses(app)
}

async fn install_package(
    app: &AppHandle,
    manifest: &Manifest,
    id: &str,
    package: &Package,
) -> Result<()> {
    let cache = app
        .path()
        .app_cache_dir()
        .context("キャッシュフォルダを取得できません")?
        .join("component-installer");
    let destination = installed_dir(app, id);
    let paths = InstallPaths {
        archive: cache.join(format!("{id}.zip.part")),
        stage: destination.with_file_name(format!(".installing-{id}-{}", std::process::id())),
        cache,
        destination,
    };
    let mut cleanup = InstallCleanup {
        archive: paths.archive.clone(),
        stage: paths.stage.clone(),
        armed: true,
    };
    let result = install_package_inner(app, manifest, id, package, &paths).await;
    if result.is_ok() {
        cleanup.armed = false;
    }
    result
}

async fn install_package_inner(
    app: &AppHandle,
    manifest: &Manifest,
    id: &str,
    package: &Package,
    paths: &InstallPaths,
) -> Result<()> {
    anyhow::ensure!(
        !package.file.trim().is_empty(),
        "{id} のファイル名がありません"
    );
    anyhow::ensure!(
        package.size > 0 && package.size <= MAX_PACKAGE_BYTES,
        "{id} のパッケージサイズが不正です"
    );
    anyhow::ensure!(package.sha256.len() == 64, "{id} のSHA-256が不正です");
    let url = format!(
        "{}/{}",
        manifest.base_url.trim_end_matches('/'),
        package.file
    );
    tokio::fs::create_dir_all(&paths.cache).await?;
    let response = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(120))
        .build()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("{id} の取得を開始できません"))?
        .error_for_status()
        .with_context(|| format!("{id} の取得先がエラーを返しました"))?;
    let total = response.content_length().unwrap_or(package.size);
    anyhow::ensure!(
        total <= MAX_PACKAGE_BYTES,
        "{id} の取得サイズが上限を超えています"
    );
    emit_progress(app, id, "download", "ダウンロード中…", 0, total);
    let mut file = tokio::fs::File::create(&paths.archive).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded = 0_u64;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        anyhow::ensure!(
            downloaded <= MAX_PACKAGE_BYTES,
            "{id} の取得サイズが上限を超えています"
        );
        file.write_all(&chunk).await?;
        emit_progress(app, id, "download", "ダウンロード中…", downloaded, total);
    }
    file.flush().await?;
    drop(file);
    anyhow::ensure!(
        downloaded == package.size,
        "{id} の取得サイズが予定と一致しません"
    );
    let archive_for_check = paths.archive.clone();
    let expected = package.sha256.to_ascii_lowercase();
    tokio::task::spawn_blocking(move || verify_sha256(&archive_for_check, &expected)).await??;
    emit_progress(
        app,
        id,
        "install",
        "安全性を確認して導入中…",
        downloaded,
        total,
    );
    let archive_for_extract = paths.archive.clone();
    let stage_for_extract = paths.stage.clone();
    tokio::task::spawn_blocking(move || extract_zip(&archive_for_extract, &stage_for_extract))
        .await??;
    if paths.destination.exists() {
        let backup = paths
            .destination
            .with_file_name(format!(".backup-{id}-{}", std::process::id()));
        std::fs::rename(&paths.destination, &backup)?;
        if let Err(error) = std::fs::rename(&paths.stage, &paths.destination) {
            if let Err(restore_error) = std::fs::rename(&backup, &paths.destination) {
                anyhow::bail!("追加機能の入れ替えに失敗し、旧版の復元にも失敗しました: 入れ替え={error} / 復元={restore_error}");
            }
            return Err(error.into());
        }
        std::fs::remove_dir_all(backup)?;
    } else {
        std::fs::rename(&paths.stage, &paths.destination)?;
    }
    tokio::fs::remove_file(&paths.archive).await?;
    emit_progress(app, id, "ready", "導入しました", downloaded, total);
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = format!("{:x}", digest.finalize());
    anyhow::ensure!(actual == expected, "パッケージのSHA-256が一致しません");
    Ok(())
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        std::fs::remove_dir_all(destination)?;
    }
    std::fs::create_dir_all(destination)?;
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    anyhow::ensure!(
        zip.len() <= MAX_FILES,
        "パッケージ内のファイル数が上限を超えています"
    );
    let mut extracted = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .context("危険なパスを含むパッケージです")?
            .to_path_buf();
        let output = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        extracted = extracted.saturating_add(entry.size());
        anyhow::ensure!(
            extracted <= MAX_EXTRACTED_BYTES,
            "展開サイズが上限を超えています"
        );
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut target = std::fs::File::create(output)?;
        std::io::copy(&mut entry, &mut target)?;
        target.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            target.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "picoagent-components-{name}-{}-{nonce}",
            std::process::id(),
        ))
    }

    #[test]
    fn 対応コンポーネントの定義が揃っている() {
        for id in COMPONENT_IDS {
            assert!(definitions(id).is_some());
        }
        assert!(definitions("unknown").is_none());
    }

    #[test]
    fn パッケージキーはosとcpuを含む() {
        let key = package_key("whisper");
        assert!(key.starts_with("whisper-"));
        assert!(key.ends_with("-x64") || key.ends_with("-arm64"));
    }

    #[test]
    fn sha256が一致しないファイルを拒否する() {
        let path = test_path("sha256");
        std::fs::write(&path, b"picoagent").unwrap();
        let correct = format!("{:x}", Sha256::digest(b"picoagent"));
        assert!(verify_sha256(&path, &correct).is_ok());
        assert!(verify_sha256(&path, &"0".repeat(64)).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn zip内の親フォルダ脱出を拒否する() {
        let archive = test_path("escape.zip");
        let destination = test_path("extract");
        let outside = destination.with_file_name("escaped.txt");
        let file = std::fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("../escaped.txt", zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(b"blocked").unwrap();
        writer.finish().unwrap();

        assert!(extract_zip(&archive, &destination).is_err());
        assert!(!outside.exists());
        std::fs::remove_file(archive).unwrap();
        if destination.exists() {
            std::fs::remove_dir_all(destination).unwrap();
        }
    }
}
