// components.rs - 初回起動と設定画面で共用する追加機能管理。
//
// 第三者ソフトウェアはPicoAgentのReleaseから再配布しない。自動導入に対応する
// Aivis／Whisperは各公式配布元から利用者のPCへ直接取得し、それ以外は公式ページと
// 手動設定を案内する。取得物はPicoAgent管理領域だけへ置き、グローバルPATHを変更しない。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const COMPONENT_IDS: [&str; 5] = ["aivis", "whisper", "tha3", "office", "tools"];
const WHISPER_RELEASE_API: &str =
    "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";
const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin";
const WHISPER_MODEL_SHA256: &str =
    "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b";
static RUNTIME_ROOT: OnceLock<PathBuf> = OnceLock::new();
static COMPONENT_OPERATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    policy: String,
    components: HashMap<String, ComponentSource>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentSource {
    install_mode: String,
    #[serde(default)]
    target_version: Option<String>,
    sources: Vec<SourceLink>,
}

#[derive(Clone, Deserialize, Serialize)]
struct SourceLink {
    name: String,
    url: String,
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
    install_mode: String,
    installed_version: Option<String>,
    target_version: Option<String>,
    update_available: bool,
    newer_than_target: bool,
    version_unknown: bool,
    version_error: Option<String>,
    sources: Vec<SourceLink>,
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
    // フォールバック許可: `OnceLock::set` は2回目以降 Err を返すだけで、
    // 先に入っている同じ値がそのまま残る（起動時に一度だけ呼ぶ初期化）。
    let _ = RUNTIME_ROOT.set(crate::log_paths::bundled_dir(app));
}

pub fn runtime_root() -> Option<&'static Path> {
    RUNTIME_ROOT.get().map(PathBuf::as_path)
}

async fn cleanup_file(app: &AppHandle, label: &str, path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            crate::log_paths::append_app_log(
                app,
                &format!("{label}を削除できません: {} ({error})", path.display()),
            );
        }
    }
}

async fn cleanup_dir(app: &AppHandle, label: &str, path: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            crate::log_paths::append_app_log(
                app,
                &format!("{label}を削除できません: {} ({error})", path.display()),
            );
        }
    }
}

fn definitions(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match id {
        "aivis" => Some((
            "音声読み上げ",
            "AivisSpeech Engineを公式GitHubから直接取得します。導入しても読み上げは自動でONになりません。",
            "約0.8GB＋利用者が選ぶ音声モデル",
        )),
        "whisper" => Some((
            "音声入力・文字起こし",
            "whisper.cpp本体とsmallモデルを各公式配布元から直接取得します。",
            "約0.5GB",
        )),
        "tha3" => Some((
            "1枚絵からキャラ作成",
            "作者のGitHubとモデル配布から利用者が取得し、キャラ設定でTHA3フォルダを指定します。CUDA対応NVIDIA GPUが必要です。",
            "約5.2GB（Windows・手動導入）",
        )),
        "office" => Some((
            "Office変換",
            "LibreOffice公式版を利用者が導入し、必要なら設定でsofficeの場所を指定します。",
            "外部アプリ（公式インストーラ）",
        )),
        "tools" => Some((
            "PDF・画像・文書ツール",
            "Pandoc、FFmpeg、qpdf、Poppler、ImageMagick、ripgrepを各提供元から導入します。",
            "外部ツール（必要なものだけ）",
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
        serde_json::from_str(&text).context("追加機能の取得元情報が壊れています")?;
    anyhow::ensure!(parsed.version == 2, "未対応の追加機能取得元情報です");
    anyhow::ensure!(
        parsed.policy == "direct-upstream",
        "第三者ソフトウェアを公式配布元から直接取得する設定ではありません"
    );
    for id in COMPONENT_IDS {
        let source = parsed
            .components
            .get(id)
            .with_context(|| format!("{id} の取得元情報がありません"))?;
        anyhow::ensure!(
            matches!(source.install_mode.as_str(), "managed" | "external"),
            "{id} の導入方式が不正です"
        );
        anyhow::ensure!(!source.sources.is_empty(), "{id} の公式取得元がありません");
        if id == "aivis" {
            let target_version = source
                .target_version
                .as_deref()
                .filter(|version| numeric_version(version).is_some())
                .context("Aivis の配布対象版が不正です")?;
            anyhow::ensure!(
                !target_version.contains(['/', '\\']),
                "Aivis の配布対象版にパス区切りを含められません"
            );
            let expected_suffix = format!("/tag/{target_version}");
            anyhow::ensure!(
                source
                    .sources
                    .iter()
                    .any(|link| link.url.ends_with(&expected_suffix)),
                "Aivis の公式取得元表示が配布対象版と一致しません"
            );
        }
        for link in &source.sources {
            anyhow::ensure!(
                !link.name.trim().is_empty() && link.url.starts_with("https://"),
                "{id} の取得元表示またはURLが不正です"
            );
        }
    }
    Ok(parsed)
}

pub fn managed_target_version(app: &AppHandle, id: &str) -> Result<String> {
    let parsed = manifest(app)?;
    let component = parsed
        .components
        .get(id)
        .with_context(|| format!("{id} の取得元情報がありません"))?;
    anyhow::ensure!(
        component.install_mode == "managed",
        "{id} はPicoAgent管理対象ではありません"
    );
    component
        .target_version
        .clone()
        .with_context(|| format!("{id} の配布対象版がありません"))
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

fn installed_component_version(id: &str, root: &Path) -> Result<Option<String>> {
    let manifest_name = match id {
        "aivis" => "engine_manifest.json",
        _ => return Ok(None),
    };
    let path = root.join(manifest_name);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} を読めません", path.display()))?;
    let value = serde_json::from_str::<serde_json::Value>(&text)
        .with_context(|| format!("{} が不正なJSONです", path.display()))?;
    Ok(value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(ToString::to_string))
}

fn numeric_version(value: &str) -> Option<Vec<u64>> {
    let core = value.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let parts = core
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (!parts.is_empty()).then_some(parts)
}

fn compare_versions(installed: &str, target: &str) -> Option<Ordering> {
    let (Some(mut installed), Some(mut target)) =
        (numeric_version(installed), numeric_version(target))
    else {
        return None;
    };
    let width = installed.len().max(target.len());
    installed.resize(width, 0);
    target.resize(width, 0);
    Some(installed.cmp(&target))
}

/// 外部導入コンポーネントが使える状態か。
///
/// 以前は `sh -lc "command -v ..."`（ログインシェル）で存在を見ていたが、ログインシェルは
/// `/etc/paths.d` から PATH を組み直すため「ある」と判定されるのに、GUI アプリが継承する
/// 素の PATH には無く、実際に spawn すると `No such file or directory` になる。同じ食い違いが
/// `aivis.rs` の p7zip で実際に起きたため、判定は共通の `resolve_executable()` へ揃える。
/// 拡張子は解決側が付けるので、ここでは OS 共通の名前だけを渡す。
fn external_ready(id: &str) -> bool {
    match id {
        "office" => crate::tools::process::has_executable("soffice"),
        "tools" => ["pandoc", "ffmpeg", "qpdf", "pdftotext", "magick", "rg"]
            .into_iter()
            .all(crate::tools::process::has_executable),
        _ => false,
    }
}

pub fn statuses(app: &AppHandle) -> Result<Vec<ComponentStatus>> {
    let manifest = manifest(app)?;
    COMPONENT_IDS
        .into_iter()
        .map(|id| -> Result<ComponentStatus> {
            let (label, description, size_label) =
                definitions(id).with_context(|| format!("{id} の定義がありません"))?;
            let info = manifest
                .components
                .get(id)
                .with_context(|| format!("{id} の取得元情報がありません"))?
                .clone();
            let installed = installed_dir(app, id).is_dir();
            let packaged_root = packaged_dir(app, id).or_else(|| development_dir(id));
            let packaged = packaged_root.is_some();
            let system = external_ready(id);
            let version_root = if installed {
                Some(installed_dir(app, id))
            } else {
                packaged_root
            };
            let (installed_version, mut version_error) = match version_root
                .as_deref()
                .map(|root| installed_component_version(id, root))
                .transpose()
            {
                Ok(version) => (version.flatten(), None),
                Err(error) => {
                    crate::log_paths::append_app_log(
                        app,
                        &format!("{label}の導入済み版を確認できません: {error:#}"),
                    );
                    (
                        None,
                        Some(
                            "導入済み版を確認できません。修復すると推奨版を取得します。"
                                .to_string(),
                        ),
                    )
                }
            };
            let target_version = info.target_version.clone();
            let version_ordering = installed_version
                .as_deref()
                .zip(target_version.as_deref())
                .and_then(|(current, target)| compare_versions(current, target));
            let update_available = version_ordering == Some(Ordering::Less);
            let newer_than_target = version_ordering == Some(Ordering::Greater);
            let version_unknown = (installed || packaged)
                && target_version.is_some()
                && installed_version
                    .as_deref()
                    .and_then(numeric_version)
                    .is_none();
            if version_unknown && version_error.is_none() {
                version_error = Some(
                    "導入済み版の表記を比較できません。修復すると推奨版を取得します。".to_string(),
                );
            }
            Ok(ComponentStatus {
                id,
                label,
                description,
                size_label,
                ready: installed || packaged || system,
                source: if installed {
                    "installed"
                } else if packaged {
                    "bundled"
                } else if system {
                    "system"
                } else {
                    "missing"
                },
                available: true,
                removable: installed && info.install_mode == "managed",
                install_mode: info.install_mode,
                installed_version,
                target_version,
                update_available,
                newer_than_target,
                version_unknown,
                version_error,
                sources: info.sources,
            })
        })
        .collect()
}

pub async fn remove(app: &AppHandle, id: &str) -> Result<Vec<ComponentStatus>> {
    anyhow::ensure!(COMPONENT_IDS.contains(&id), "未知の追加機能です: {id}");
    let info = manifest(app)?
        .components
        .remove(id)
        .with_context(|| format!("{id} の取得元情報がありません"))?;
    anyhow::ensure!(
        info.install_mode == "managed",
        "外部アプリはPicoAgentから削除できません。提供元の手順で削除してください"
    );
    let _operation = COMPONENT_OPERATION_LOCK.lock().await;
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
    if let Err(error) = app.emit(
        "component:progress",
        Progress {
            id: id.to_string(),
            stage: stage.to_string(),
            message: message.into(),
            downloaded,
            total,
        },
    ) {
        crate::log_paths::append_app_log(
            app,
            &format!("追加機能の進捗を画面へ通知できません: {error}"),
        );
    }
}

pub async fn install(app: &AppHandle, ids: &[String]) -> Result<Vec<ComponentStatus>> {
    let manifest = manifest(app)?;
    for id in ids {
        anyhow::ensure!(
            COMPONENT_IDS.contains(&id.as_str()),
            "未知の追加機能です: {id}"
        );
        let info = manifest
            .components
            .get(id)
            .with_context(|| format!("{id} の取得元情報がありません"))?;
        anyhow::ensure!(
            info.install_mode == "managed",
            "{} は提供元の公式ページから導入してください",
            definitions(id).map(|item| item.0).unwrap_or(id)
        );
    }
    let _operation = COMPONENT_OPERATION_LOCK.lock().await;
    for id in ids {
        match id.as_str() {
            "aivis" => {
                emit_progress(app, id, "download", "Aivis公式配布から取得中…", 0, 0);
                crate::aivis::install_component(app).await?;
                emit_progress(app, id, "ready", "Aivisを導入しました", 0, 0);
            }
            "whisper" => install_whisper(app).await?,
            _ => anyhow::bail!("{id} は自動導入の対象ではありません"),
        }
    }
    statuses(app)
}

async fn install_whisper(app: &AppHandle) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(120))
        .user_agent("PicoAgent/1.0 (official component installer)")
        .build()?;
    let release = client
        .get(WHISPER_RELEASE_API)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let (asset_url, asset_size, asset_sha256) = whisper_asset(&release)?;
    let cache = app
        .path()
        .app_cache_dir()
        .context("キャッシュフォルダを取得できません")?
        .join("component-installer");
    tokio::fs::create_dir_all(&cache).await?;
    let archive = cache.join("whisper.zip.part");
    let unpacked = cache.join(format!("whisper-unpacked-{}", std::process::id()));
    let stage = installed_dir(app, "whisper")
        .with_file_name(format!(".installing-whisper-{}", std::process::id()));
    cleanup_file(app, "古いWhisper取得途中ファイル", &archive).await;
    cleanup_dir(app, "古いWhisper展開途中フォルダ", &unpacked).await;
    cleanup_dir(app, "古いWhisper導入途中フォルダ", &stage).await;

    let result = async {
        download_to(
            app,
            &client,
            DownloadSpec {
                id: "whisper",
                url: &asset_url,
                destination: &archive,
                expected_size: asset_size,
                max_size: MAX_DOWNLOAD_BYTES,
                message: "whisper.cpp公式配布をダウンロード中…",
            },
        )
        .await?;
        verify_sha256(&archive, &asset_sha256)?;
        let archive_for_extract = archive.clone();
        let unpacked_for_extract = unpacked.clone();
        tokio::task::spawn_blocking(move || {
            extract_zip(&archive_for_extract, &unpacked_for_extract)
        })
        .await??;
        let executable = find_named_file(&unpacked, whisper_exe_name(), 0)
            .context("公式whisper.cpp配布にwhisper-cliが見つかりません")?;
        let binary_root = executable
            .parent()
            .context("whisper-cliの親フォルダを取得できません")?
            .to_path_buf();
        let stage_for_copy = stage.clone();
        tokio::task::spawn_blocking(move || copy_dir_all(&binary_root, &stage_for_copy)).await??;

        let model = stage.join("models/ggml-small.bin");
        if let Some(parent) = model.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        download_to(
            app,
            &client,
            DownloadSpec {
                id: "whisper",
                url: WHISPER_MODEL_URL,
                destination: &model,
                expected_size: 487_601_967,
                max_size: MAX_DOWNLOAD_BYTES,
                message: "公式Whisper smallモデルをダウンロード中…",
            },
        )
        .await?;
        verify_sha256(&model, WHISPER_MODEL_SHA256)?;
        let config = serde_json::json!({
            "program": whisper_exe_name(),
            "model": "models/ggml-small.bin",
            "args": "-m {model} -f {audio} -otxt -of {output_stem}",
            "diarizeArgs": "-tdrz"
        });
        crate::store::write_atomic(
            &stage.join("whisper.json"),
            format!("{}\n", serde_json::to_string_pretty(&config)?).as_bytes(),
        )?;
        replace_installed(&stage, &installed_dir(app, "whisper"), "whisper")?;
        emit_progress(
            app,
            "whisper",
            "ready",
            "Whisperを公式配布元から導入しました",
            0,
            0,
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;

    // フォールバック許可: 成否にかかわらずPicoAgent管理下の作業用ファイルだけを掃除する。
    cleanup_file(app, "Whisper取得途中ファイル", &archive).await;
    cleanup_dir(app, "Whisper展開途中フォルダ", &unpacked).await;
    if result.is_err() {
        cleanup_dir(app, "Whisper導入途中フォルダ", &stage).await;
    }
    result
}

fn whisper_exe_name() -> &'static str {
    if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    }
}

fn whisper_asset(release: &serde_json::Value) -> Result<(String, u64, String)> {
    let suffix = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "whisper-bin-arm64.zip",
        ("windows", _) => "whisper-bin-x64.zip",
        ("macos", "aarch64") => "whisper-bin-macos-arm64.zip",
        ("macos", _) => "whisper-bin-macos-x64.zip",
        _ => anyhow::bail!("このOS向けのwhisper.cpp公式配布は自動導入できません"),
    };
    let asset = release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| name.ends_with(suffix))
            })
        })
        .with_context(|| format!("whisper.cpp公式Releaseに{suffix}がありません"))?;
    let url = asset
        .get("browser_download_url")
        .and_then(serde_json::Value::as_str)
        .context("whisper.cppの公式ダウンロードURLがありません")?;
    let size = asset
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .context("whisper.cpp配布サイズがありません")?;
    let digest = asset
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64)
        .context("whisper.cpp公式ReleaseにSHA-256 digestがありません")?;
    Ok((url.to_string(), size, digest.to_ascii_lowercase()))
}

struct DownloadSpec<'a> {
    id: &'a str,
    url: &'a str,
    destination: &'a Path,
    expected_size: u64,
    max_size: u64,
    message: &'a str,
}

async fn download_to(
    app: &AppHandle,
    client: &reqwest::Client,
    spec: DownloadSpec<'_>,
) -> Result<()> {
    anyhow::ensure!(
        spec.url.starts_with("https://"),
        "HTTPS以外からは取得できません"
    );
    anyhow::ensure!(
        spec.expected_size > 0 && spec.expected_size <= spec.max_size,
        "取得予定サイズが不正です"
    );
    let response = client.get(spec.url).send().await?.error_for_status()?;
    if let Some(length) = response.content_length() {
        anyhow::ensure!(
            length == spec.expected_size,
            "公式配布のサイズが予定と一致しません"
        );
    }
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(spec.destination).await?;
    let mut downloaded = 0_u64;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        anyhow::ensure!(
            downloaded <= spec.max_size,
            "取得サイズが上限を超えています"
        );
        file.write_all(&chunk).await?;
        emit_progress(
            app,
            spec.id,
            "download",
            spec.message,
            downloaded,
            spec.expected_size,
        );
    }
    file.flush().await?;
    drop(file);
    anyhow::ensure!(
        downloaded == spec.expected_size,
        "公式配布のサイズが予定と一致しません"
    );
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    anyhow::ensure!(expected.len() == 64, "SHA-256が不正です");
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = hex::encode(digest.finalize());
    anyhow::ensure!(
        actual == expected.to_ascii_lowercase(),
        "SHA-256が一致しません"
    );
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
        "配布物内のファイル数が上限を超えています"
    );
    let mut extracted = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .context("危険なパスを含む配布物です")?
            .to_path_buf();
        let output = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut target = std::fs::File::create(output)?;
        let remaining = MAX_EXTRACTED_BYTES.saturating_sub(extracted);
        let mut limited = std::io::Read::take(&mut entry, remaining.saturating_add(1));
        let copied = std::io::copy(&mut limited, &mut target)?;
        anyhow::ensure!(copied <= remaining, "展開サイズが上限を超えています");
        extracted = extracted.saturating_add(copied);
        target.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            target.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

fn find_named_file(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 8 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|item| item.to_str()) == Some(name) {
            return Some(path);
        }
        if path.is_dir() {
            subdirs.push(path);
        }
    }
    subdirs
        .into_iter()
        .find_map(|subdir| find_named_file(&subdir, name, depth + 1))
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(from, to)?;
        }
    }
    Ok(())
}

fn replace_installed(stage: &Path, destination: &Path, id: &str) -> Result<()> {
    if destination.exists() {
        let backup = destination.with_file_name(format!(".backup-{id}-{}", std::process::id()));
        if backup.exists() {
            std::fs::remove_dir_all(&backup)?;
        }
        std::fs::rename(destination, &backup)?;
        if let Err(error) = std::fs::rename(stage, destination) {
            if let Err(restore_error) = std::fs::rename(&backup, destination) {
                anyhow::bail!("追加機能の入れ替えと旧版復元に失敗しました: 入れ替え={error} / 復元={restore_error}");
            }
            return Err(error.into());
        }
        std::fs::remove_dir_all(backup)?;
    } else {
        std::fs::rename(stage, destination)?;
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
    fn sha256が一致しないファイルを拒否する() {
        let path = test_path("sha256");
        std::fs::write(&path, b"picoagent").unwrap();
        let correct = hex::encode(Sha256::digest(b"picoagent"));
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
            .start_file("../escaped.txt", zip::write::SimpleFileOptions::default())
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

    #[test]
    fn 取得元情報は公式直接取得ポリシーだけを受け付ける() {
        let value: Manifest =
            serde_json::from_str(include_str!("../resources/component-manifest.json")).unwrap();
        assert_eq!(value.version, 2);
        assert_eq!(value.policy, "direct-upstream");
        assert_eq!(value.components["aivis"].install_mode, "managed");
        let aivis_target = value.components["aivis"]
            .target_version
            .as_deref()
            .expect("Aivisの配布対象版");
        assert!(super::numeric_version(aivis_target).is_some());
        assert!(value.components["aivis"]
            .sources
            .iter()
            .any(|source| source.url.ends_with(&format!("/tag/{aivis_target}"))));
        assert_eq!(value.components["office"].install_mode, "external");
        assert!(value
            .components
            .values()
            .flat_map(|component| &component.sources)
            .all(|source| source.url.starts_with("https://")));
    }

    #[test]
    fn 配布対象より古い版だけを更新対象にする() {
        assert_eq!(
            super::compare_versions("1.2.0", "1.3.0"),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            super::compare_versions("v1.2", "1.2.1"),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(super::compare_versions("不明", "1.2.0"), None);
        assert_eq!(
            super::compare_versions("1.3.0", "1.2.0"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            super::compare_versions("1.2", "1.2.0"),
            Some(std::cmp::Ordering::Equal)
        );
    }

    #[test]
    fn aivisの実体マニフェストから導入版を読む() {
        let root = test_path("aivis-version");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("engine_manifest.json"), br#"{"version":"1.2.3"}"#).unwrap();
        assert_eq!(
            super::installed_component_version("aivis", &root)
                .unwrap()
                .as_deref(),
            Some("1.2.3")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
