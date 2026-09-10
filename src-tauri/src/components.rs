// components.rs - 初回起動と設定画面で共用する追加機能管理。
//
// 第三者ソフトウェアはPicoAgentのReleaseから再配布しない。自動導入に対応する
// Aivis／Whisperは各公式配布元から利用者のPCへ直接取得し、それ以外は公式ページと
// 手動設定を案内する。取得物はPicoAgent管理領域だけへ置き、グローバルPATHを変更しない。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const COMPONENT_IDS: [&str; 7] = [
    "aivis",
    "whisper",
    "character-maker",
    "video-wan",
    "video-ltx",
    "office",
    "tools",
];
const WHISPER_RELEASE_API: &str =
    "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";
const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin";
const WHISPER_MODEL_SHA256: &str =
    "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b";
static RUNTIME_ROOT: OnceLock<PathBuf> = OnceLock::new();
static CHARACTER_MAKER_ASSETS: OnceLock<PathBuf> = OnceLock::new();
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
    let packaged = app
        .path()
        .resource_dir()
        .ok()
        .map(|root| root.join("character-maker"))
        .filter(|root| root.join("sidecar").is_dir());
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("character-maker"));
    if let Some(root) = packaged.or(development) {
        let _ = CHARACTER_MAKER_ASSETS.set(root);
    }
}

pub fn runtime_root() -> Option<&'static Path> {
    RUNTIME_ROOT.get().map(PathBuf::as_path)
}

pub(crate) fn character_maker_assets() -> Option<PathBuf> {
    CHARACTER_MAKER_ASSETS.get().cloned()
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
        "character-maker" => Some((
            "画像編集・キャラクター作成基盤",
            "Python 3.12、ComfyUI、Qwen画像編集、背景除去・部位分解モデルをPicoAgent専用領域へ導入します。CUDA対応NVIDIA GPU専用です。",
            "約77GB（導入後、作業領域は別。空き85GB以上が必要）",
        )),
        "video-wan" => Some((
            "Wan 2.2動画生成",
            "8GB VRAM向けの標準候補Wan 2.2 TI2V 5Bです。テキストまたは画像から動画を生成します。選択時だけ取得します。",
            "約18.1GB（空き22GB以上が必要）",
        )),
        "video-ltx" => Some((
            "LTX軽量動画生成",
            "8GB VRAM向けのLTX-Video 0.9.8 2B FP8です。最新LTX 2.5は公式基準16GB以上のため取得しません。",
            "約14.3GB（空き18GB以上が必要）",
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

/// CUDA対応Python機能は、機能ごとにPythonと同一wheelを複製せず、この1環境を共有する。
pub(crate) fn shared_python_dir(app: &AppHandle) -> PathBuf {
    crate::log_paths::bundled_dir(app).join("python")
}

fn installed_component_version(id: &str, root: &Path) -> Result<Option<String>> {
    let manifest_name = match id {
        "aivis" => "engine_manifest.json",
        "character-maker" | "video-wan" | "video-ltx" => "install.json",
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

fn command_exists(program: &str) -> bool {
    let mut command = if cfg!(windows) {
        let mut command = crate::tools::process::command_sync("where.exe");
        command.arg(program);
        command
    } else {
        let mut command = crate::tools::process::command_sync("sh");
        command.args(["-lc", &format!("command -v {program}")]);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn external_ready(id: &str) -> bool {
    match id {
        "office" => command_exists(if cfg!(windows) {
            "soffice.exe"
        } else {
            "soffice"
        }),
        "tools" => ["pandoc", "ffmpeg", "qpdf", "pdftotext", "magick", "rg"]
            .into_iter()
            .all(command_exists),
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
            let installed = if id == "character-maker" {
                installed_dir(app, id).join("install.json").is_file()
                    && shared_python_dir(app).join("Scripts/python.exe").is_file()
                    && installed_dir(app, id).join("ComfyUI/main.py").is_file()
            } else {
                installed_dir(app, id).is_dir()
            };
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
    let shared_python = (id == "character-maker").then(|| shared_python_dir(app));
    let label = definitions(id).map(|item| item.0).unwrap_or(id).to_string();
    emit_progress(app, id, "remove", "削除中…", 0, 0);
    tokio::task::spawn_blocking(move || {
        if target.exists() {
            std::fs::remove_dir_all(&target).with_context(|| {
                format!("{label} を削除できません。使用中の機能を終了して再試行してください")
            })?;
        }
        if let Some(shared_python) = shared_python {
            if shared_python.exists() {
                std::fs::remove_dir_all(&shared_python).with_context(|| {
                    format!("共通Pythonを削除できません: {}", shared_python.display())
                })?;
            }
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
            "character-maker" => install_character_maker(app).await?,
            "video-wan" => {
                install_media_model(app, "video-wan", "wan-2.2-5b", 22_000_000_000).await?
            }
            "video-ltx" => {
                install_media_model(app, "video-ltx", "ltx-0.9.8-2b-fp8", 18_000_000_000).await?
            }
            _ => anyhow::bail!("{id} は自動導入の対象ではありません"),
        }
    }
    statuses(app)
}

async fn install_character_maker(app: &AppHandle) -> Result<()> {
    anyhow::ensure!(
        cfg!(all(target_os = "windows", target_arch = "x86_64")),
        "キャラクター作成はWindows x64専用です"
    );
    let gpu = tokio::task::spawn_blocking(|| {
        crate::tools::process::command_sync("nvidia-smi.exe")
            .args(["--query-gpu=name", "--format=csv,noheader"])
            .stdin(Stdio::null())
            .output()
    })
    .await??;
    anyhow::ensure!(
        gpu.status.success() && !gpu.stdout.is_empty(),
        "CUDA対応NVIDIA GPUを確認できません。何も取得していません"
    );
    let bundled = crate::log_paths::bundled_dir(app);
    std::fs::create_dir_all(&bundled)?;
    anyhow::ensure!(
        free_disk_bytes(&bundled)? >= 85_000_000_000,
        "キャラクター作成の導入には85GB以上の空き容量が必要です。何も取得していません"
    );

    const UV_URL: &str =
        "https://github.com/astral-sh/uv/releases/download/0.12.11/uv-x86_64-pc-windows-msvc.zip";
    const UV_SHA: &str = "e94225dea91e051472847bd6d146d7d66c4f54ffcd1f106678866a99580845f9";
    const COMFY_URL: &str =
        "https://github.com/Comfy-Org/ComfyUI/archive/12d5279438bfefc058a269eae805ceab6047777f.zip";
    const COMFY_SHA: &str = "a3c51a42bc7568d4b57267af3a1aa86a6cb379a199f95a90e8e3679c52651241";
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(120))
        .user_agent("PicoAgent/1.0 character maker installer")
        .build()?;
    let cache = app
        .path()
        .app_cache_dir()?
        .join("character-maker-installer");
    let stage = installed_dir(app, "character-maker").with_file_name(format!(
        ".installing-character-maker-{}",
        std::process::id()
    ));
    let python_stage = crate::log_paths::bundled_dir(app)
        .join(format!(".installing-python-{}", std::process::id()));
    cleanup_stale_install_stages(app).await;
    cleanup_dir(app, "古いキャラ作成導入途中フォルダ", &stage).await;
    cleanup_dir(app, "古い共通Python導入途中フォルダ", &python_stage).await;
    tokio::fs::create_dir_all(&cache).await?;
    tokio::fs::create_dir_all(&stage).await?;
    let result = async {
        let uv_zip = cache.join("uv.zip.part");
        download_to(
            app,
            &client,
            DownloadSpec {
                id: "character-maker",
                url: UV_URL,
                destination: &uv_zip,
                expected_size: 16_996_332,
                max_size: 30_000_000,
                message: "Python管理ツールを取得中…",
            },
        )
        .await?;
        verify_sha256(&uv_zip, UV_SHA)?;
        let uv_dir = cache.join("uv");
        cleanup_dir(app, "uv展開途中フォルダ", &uv_dir).await;
        let uv_zip_copy = uv_zip.clone();
        let uv_dir_copy = uv_dir.clone();
        tokio::task::spawn_blocking(move || extract_zip(&uv_zip_copy, &uv_dir_copy)).await??;
        let uv = find_named_file(&uv_dir, "uv.exe", 0).context("uv.exeが配布物にありません")?;
        // uvの仮想環境は基準Pythonを参照する。キャッシュへ置くと起動時清掃で壊れるため、
        // 基準Pythonとvenvを同じ管理対象世代へまとめて原子的に公開する。
        let python_store = python_stage.join("runtime");
        run_component_command(
            &uv,
            &[
                "python",
                "install",
                "3.12.13",
                "--install-dir",
                &python_store.to_string_lossy(),
            ],
        )
        .await?;
        let base_python = find_named_file(&python_store, "python.exe", 0)
            .context("Python 3.12.13を導入できません")?;
        let venv_stage = cache.join("python-venv");
        cleanup_dir(app, "共通Python仮想環境の作成途中フォルダ", &venv_stage).await;
        run_component_command(
            &uv,
            &[
                "venv",
                &venv_stage.to_string_lossy(),
                "--python",
                &base_python.to_string_lossy(),
                "--relocatable",
            ],
        )
        .await?;
        let python_stage_copy = python_stage.clone();
        let venv_stage_copy = venv_stage.clone();
        tokio::task::spawn_blocking(move || copy_dir_all(&venv_stage_copy, &python_stage_copy))
            .await??;
        let python = python_stage.clone();
        let assets = character_maker_assets().context("キャラ作成資源がありません")?;
        run_component_command(
            &uv,
            &[
                "pip",
                "sync",
                "--python",
                &python.join("Scripts/python.exe").to_string_lossy(),
                &assets
                    .join("sidecar/requirements-comfy.lock")
                    .to_string_lossy(),
                "--extra-index-url",
                "https://download.pytorch.org/whl/cu128",
                "--index-strategy",
                "unsafe-best-match",
                "--require-hashes",
            ],
        )
        .await?;
        let comfy_zip = cache.join("comfy.zip.part");
        download_to(
            app,
            &client,
            DownloadSpec {
                id: "character-maker",
                url: COMFY_URL,
                destination: &comfy_zip,
                expected_size: 12_698_942,
                max_size: 30_000_000,
                message: "ComfyUI v0.34.0を取得中…",
            },
        )
        .await?;
        verify_sha256(&comfy_zip, COMFY_SHA)?;
        let comfy_unpack = cache.join("comfy");
        cleanup_dir(app, "ComfyUI展開途中フォルダ", &comfy_unpack).await;
        let archive = comfy_zip.clone();
        let unpack = comfy_unpack.clone();
        tokio::task::spawn_blocking(move || extract_zip(&archive, &unpack)).await??;
        let main =
            find_named_file(&comfy_unpack, "main.py", 0).context("ComfyUI main.pyがありません")?;
        let comfy_root = main
            .parent()
            .context("ComfyUIの親フォルダがありません")?
            .to_path_buf();
        let destination = stage.join("ComfyUI");
        tokio::task::spawn_blocking(move || copy_dir_all(&comfy_root, &destination)).await??;
        let model_progress = cache.join("model-progress.json");
        run_component_command_with_progress(
            app,
            "character-maker",
            &python.join("Scripts/python.exe"),
            &[
                &assets
                    .join("installer/setup-runtime-models.py")
                    .to_string_lossy(),
                "--runtime",
                &stage.to_string_lossy(),
                "--cache",
                &cache.join("model-download").to_string_lossy(),
                "--progress",
                &model_progress.to_string_lossy(),
            ],
            &model_progress,
        )
        .await?;
        crate::store::write_json_atomic(
            &stage.join("install.json"),
            &serde_json::json!({"version":"2026.09.10","python":"3.12.13","comfy":"v0.34.0"}),
        )?;
        replace_installed_pair(
            (&python_stage, &shared_python_dir(app), "python"),
            (
                &stage,
                &installed_dir(app, "character-maker"),
                "character-maker",
            ),
        )?;
        cleanup_file(app, "uv取得ファイル", &uv_zip).await;
        cleanup_file(app, "ComfyUI取得ファイル", &comfy_zip).await;
        cleanup_dir(app, "uv展開フォルダ", &uv_dir).await;
        cleanup_dir(app, "ComfyUI展開フォルダ", &comfy_unpack).await;
        cleanup_dir(app, "共通Python仮想環境の作成途中フォルダ", &venv_stage).await;
        cleanup_dir(app, "モデル取得キャッシュ", &cache.join("model-download")).await;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        cleanup_dir(app, "キャラ作成導入途中フォルダ", &stage).await;
        cleanup_dir(app, "共通Python導入途中フォルダ", &python_stage).await;
    }
    result
}

async fn install_media_model(
    app: &AppHandle,
    id: &'static str,
    model: &'static str,
    required_free: u64,
) -> Result<()> {
    anyhow::ensure!(
        cfg!(all(target_os = "windows", target_arch = "x86_64")),
        "ローカル動画生成はWindows x64専用です"
    );
    let python = shared_python_dir(app).join("Scripts/python.exe");
    anyhow::ensure!(
        installed_dir(app, "character-maker")
            .join("ComfyUI/main.py")
            .is_file()
            && python.is_file(),
        "先に「画像編集・キャラクター作成基盤」を導入してください。動画モデルはまだ取得していません"
    );
    let gpu = tokio::task::spawn_blocking(|| {
        crate::tools::process::command_sync("nvidia-smi.exe")
            .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .stdin(Stdio::null())
            .output()
    })
    .await??;
    let memory_mb = String::from_utf8_lossy(&gpu.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    anyhow::ensure!(
        gpu.status.success() && memory_mb >= 7_500,
        "8GB以上のCUDA対応NVIDIA GPUを確認できません。動画モデルは取得していません"
    );
    let bundled = crate::log_paths::bundled_dir(app);
    std::fs::create_dir_all(&bundled)?;
    anyhow::ensure!(
        free_disk_bytes(&bundled)? >= required_free,
        "{}の導入に必要な空き容量がありません。動画モデルは取得していません",
        definitions(id).map(|value| value.0).unwrap_or(id)
    );
    let stage =
        installed_dir(app, id).with_file_name(format!(".installing-{id}-{}", std::process::id()));
    let cache = app
        .path()
        .app_cache_dir()?
        .join("media-model-installer")
        .join(id);
    cleanup_dir(app, "古い動画モデル導入途中フォルダ", &stage).await;
    tokio::fs::create_dir_all(&stage).await?;
    tokio::fs::create_dir_all(&cache).await?;
    let progress = cache.join("progress.json");
    let assets = character_maker_assets().context("画像・動画生成資源がありません")?;
    let script = assets.join("sidecar/media/setup_models.py");
    let download_cache = cache.join("download");
    let result = run_component_command_with_progress(
        app,
        id,
        &python,
        &[
            &script.to_string_lossy(),
            "--model",
            model,
            "--runtime",
            &stage.to_string_lossy(),
            "--cache",
            &download_cache.to_string_lossy(),
            "--progress",
            &progress.to_string_lossy(),
        ],
        &progress,
    )
    .await;
    if let Err(error) = result {
        cleanup_dir(app, "動画モデル導入途中フォルダ", &stage).await;
        return Err(error);
    }
    replace_installed(&stage, &installed_dir(app, id), id)?;
    cleanup_dir(app, "動画モデル取得キャッシュ", &cache).await;
    emit_progress(
        app,
        id,
        "ready",
        format!(
            "{}を導入しました",
            definitions(id).map(|value| value.0).unwrap_or(id)
        ),
        1,
        1,
    );
    Ok(())
}

fn stale_install_pid(name: &str, current_pid: &str) -> Option<u32> {
    [
        ".installing-character-maker-",
        ".installing-python-",
        ".installing-whisper-",
        ".installing-video-wan-",
        ".installing-video-ltx-",
    ]
    .iter()
    .find_map(|prefix| name.strip_prefix(prefix))
    .filter(|pid| *pid != current_pid)
    .and_then(|pid| pid.parse::<u32>().ok())
}

async fn cleanup_stale_install_stages(app: &AppHandle) {
    let bundled = crate::log_paths::bundled_dir(app);
    let Ok(mut entries) = tokio::fs::read_dir(&bundled).await else {
        return;
    };
    let current_pid = std::process::id().to_string();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let stale_pid = stale_install_pid(&name, &current_pid);
        if stale_pid.is_some_and(|pid| !process_exists(pid)) {
            cleanup_dir(app, "終了済み導入途中フォルダ", &entry.path()).await;
        }
    }
}

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    let Ok(output) = crate::tools::process::command_sync("tasklist.exe")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
    else {
        // 判定不能時は削除しない。
        return true;
    };
    let Ok(stdout) = String::from_utf8(output.stdout) else {
        // 判定不能時は別プロセスの作業中フォルダを消さない。
        return true;
    };
    stdout.contains(&format!("\"{pid}\""))
}

#[cfg(not(windows))]
fn process_exists(_pid: u32) -> bool {
    // Windows専用機能だが、別OSで呼ばれても不明なPIDのフォルダは削除しない。
    true
}

async fn run_component_command(program: &Path, args: &[&str]) -> Result<()> {
    let mut command = crate::tools::process::command(program);
    command.args(args).env(
        "UV_CACHE_DIR",
        program.parent().unwrap_or(Path::new(".")).join("cache"),
    );
    let output = crate::tools::process::output(command, 7_200, "キャラクター作成ランタイム導入")
        .await
        .map_err(anyhow::Error::msg)?;
    let stderr = String::from_utf8(output.stderr).unwrap_or_else(|error| {
        format!(
            "標準エラーがUTF-8ではありません（{}バイト）",
            error.as_bytes().len()
        )
    });
    anyhow::ensure!(
        output.status.success(),
        "{} の実行に失敗しました: {stderr}",
        program.display()
    );
    Ok(())
}

async fn run_component_command_with_progress(
    app: &AppHandle,
    id: &str,
    program: &Path,
    args: &[&str],
    progress_path: &Path,
) -> Result<()> {
    let mut command = crate::tools::process::command(program);
    command.args(args).env(
        "UV_CACHE_DIR",
        program.parent().unwrap_or(Path::new(".")).join("cache"),
    );
    let output = crate::tools::process::output(command, 7_200, "キャラクター作成モデル導入");
    tokio::pin!(output);
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    let result = loop {
        tokio::select! {
            result = &mut output => break result.map_err(anyhow::Error::msg)?,
            _ = ticker.tick() => {
                let Ok(bytes) = tokio::fs::read(progress_path).await else { continue; };
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else { continue; };
                let message = value.get("message").and_then(serde_json::Value::as_str).unwrap_or("モデルを取得中…");
                let downloaded = value.get("downloaded").and_then(serde_json::Value::as_u64).unwrap_or(0);
                let total = value.get("total").and_then(serde_json::Value::as_u64).unwrap_or(0);
                emit_progress(app, id, "download", message, downloaded, total);
            }
        }
    };
    cleanup_file(app, "モデル取得進捗ファイル", progress_path).await;
    let stderr = String::from_utf8(result.stderr).unwrap_or_else(|error| {
        format!(
            "標準エラーがUTF-8ではありません（{}バイト）",
            error.as_bytes().len()
        )
    });
    anyhow::ensure!(
        result.status.success(),
        "{} の実行に失敗しました: {stderr}",
        program.display()
    );
    Ok(())
}

#[cfg(windows)]
fn free_disk_bytes(path: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    anyhow::ensure!(
        ok != 0,
        "保存先の空き容量を確認できません: {}",
        path.display()
    );
    Ok(available)
}

#[cfg(not(windows))]
fn free_disk_bytes(_path: &Path) -> Result<u64> {
    Ok(0)
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
    cleanup_stale_install_stages(app).await;
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

fn replace_installed_pair(first: (&Path, &Path, &str), second: (&Path, &Path, &str)) -> Result<()> {
    let pairs = [first, second];
    let backups = pairs.map(|(_, destination, id)| {
        destination.with_file_name(format!(".backup-{id}-{}", std::process::id()))
    });
    for backup in &backups {
        if backup.exists() {
            std::fs::remove_dir_all(backup)?;
        }
    }
    let mut backed_up = 0_usize;
    for (index, (_, destination, _)) in pairs.iter().enumerate() {
        if destination.exists() {
            if let Err(error) = std::fs::rename(destination, &backups[index]) {
                for restore in (0..backed_up).rev() {
                    if backups[restore].exists() {
                        std::fs::rename(&backups[restore], pairs[restore].1)?;
                    }
                }
                return Err(error.into());
            }
        }
        backed_up = index + 1;
    }
    let mut published = 0_usize;
    for (index, (stage, destination, _)) in pairs.iter().enumerate() {
        if let Err(error) = std::fs::rename(stage, destination) {
            let mut restore_errors = Vec::new();
            for rollback in (0..published).rev() {
                if let Err(rollback_error) = std::fs::rename(pairs[rollback].1, pairs[rollback].0) {
                    restore_errors.push(rollback_error.to_string());
                }
            }
            for restore in (0..backed_up).rev() {
                if backups[restore].exists() {
                    if let Err(restore_error) = std::fs::rename(&backups[restore], pairs[restore].1)
                    {
                        restore_errors.push(restore_error.to_string());
                    }
                }
            }
            if !restore_errors.is_empty() {
                anyhow::bail!(
                    "キャラ作成ランタイムの入れ替えと旧版復元に失敗しました: 入れ替え={error} / 復元={}",
                    restore_errors.join(" / ")
                );
            }
            return Err(error.into());
        }
        published = index + 1;
    }
    for backup in backups {
        if backup.exists() {
            std::fs::remove_dir_all(backup)?;
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
    fn python環境の配置は共通入口だけで決める() {
        let components_source = include_str!("components.rs");
        let maker_source = include_str!("character_maker.rs");
        assert_eq!(components_source.matches(".join(\"python\")").count(), 1);
        assert!(!maker_source.contains(".join(\"python\")"));
        assert!(maker_source.contains("components::shared_python_dir(app)"));
        assert!(components_source.contains("python_stage.join(\"runtime\")"));
        assert!(components_source.contains("\"--relocatable\""));
        assert_eq!(components_source.matches("Python取得キャッシュ").count(), 1);
    }

    #[test]
    fn 終了済みpid付きの導入途中名だけを回収対象にする() {
        assert_eq!(
            stale_install_pid(".installing-whisper-123", "999"),
            Some(123)
        );
        assert_eq!(
            stale_install_pid(".installing-character-maker-123", "999"),
            Some(123)
        );
        assert_eq!(
            stale_install_pid(".installing-python-123", "999"),
            Some(123)
        );
        assert_eq!(stale_install_pid(".installing-whisper-999", "999"), None);
        assert_eq!(stale_install_pid("whisper", "999"), None);
        assert_eq!(
            stale_install_pid(".installing-whisper-invalid", "999"),
            None
        );
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
