// components.rs - 初回起動と設定画面で共用する追加機能管理。
//
// 第三者ソフトウェアはPicoAgentのReleaseから再配布しない。自動導入に対応する
// Aivis／Whisperは各公式配布元から利用者のPCへ直接取得し、それ以外は公式ページと
// 手動設定を案内する。取得物はPicoAgent管理領域だけへ置き、グローバルPATHを変更しない。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const COMPONENT_IDS: [&str; 9] = [
    "aivis",
    "whisper",
    "character-maker",
    "video-wan",
    "video-ltx",
    "image-krea2",
    "video-ltx25",
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
static PACKAGED_ROOT: OnceLock<PathBuf> = OnceLock::new();
static CHARACTER_MAKER_ASSETS: OnceLock<PathBuf> = OnceLock::new();
pub(crate) static COMPONENT_OPERATION_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

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
    let _ = RUNTIME_ROOT.set(crate::app_paths::bundled_dir(app));
    if let Some(root) = crate::app_paths::packaged_bundled_dir(app) {
        let _ = PACKAGED_ROOT.set(root);
    }
    // キャラ作成の資産はこのビルドの同梱場所だけを使う（無いときに別の場所を探さない）。
    if let Some(root) = crate::app_paths::packaged_character_maker_dir(app)
        .filter(|root| root.join("sidecar").is_dir())
    {
        let _ = CHARACTER_MAKER_ASSETS.set(root);
    }
    // 前回の導入・入れ替えが途中で落ちた残りを起動時に片付ける。導入操作のときだけ
    // 片付けると、それまで数十GBの途中データや未導入に見える機能が残り続ける。
    // 退避先の復元は現在の配置先も変更するため、導入・削除と同じ排他を取る。
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _operation = COMPONENT_OPERATION_LOCK.lock().await;
        if let Err(error) = cleanup_stale_install_stages(&app).await {
            crate::log_paths::append_app_log(
                &app,
                &format!("追加機能の中断復旧に失敗しました: {error:#}"),
            );
        }
    });
}

pub fn runtime_root() -> Option<&'static Path> {
    RUNTIME_ROOT.get().map(PathBuf::as_path)
}

/// このビルドの同梱物の置き場所（`app_paths::packaged_bundled_dir`）。`AppHandle` を持たない
/// スキルから参照するため、起動時に一度だけ記録する。
pub fn packaged_root() -> Option<&'static Path> {
    PACKAGED_ROOT.get().map(PathBuf::as_path)
}

/// 追加導入した物（利用者データの `bundled/`）、このビルドの同梱物の順で、名前付きの置き場所を返す。
/// 作業フォルダ・実行ファイルの位置・旧配置からは探さない。
pub fn runtime_dirs(name: &str) -> Vec<PathBuf> {
    [runtime_root(), packaged_root()]
        .into_iter()
        .flatten()
        .map(|root| root.join(name))
        .collect()
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
            "Python 3.12、ComfyUI、Qwen画像編集、背景除去・部位分解モデルをPicoAgent専用領域へ導入します。Windows x64のCUDA対応NVIDIA GPU、またはAMD公式のWindows向けROCm対応GPUで動きます。実GPUに合う方のPyTorchだけを取得します。",
            "約77GB（導入後、作業領域は別。空き85GB以上が必要）",
        )),
        "video-wan" => Some((
            "Wan 2.2動画生成",
            "8GB VRAM向けの標準候補Wan 2.2 TI2V 5Bです。テキストまたは画像から動画を生成します。選択時だけ取得します。モデルはCUDA・ROCmで共通です。",
            "約18.1GB（空き22GB以上が必要）",
        )),
        "video-ltx" => Some((
            "LTX軽量動画生成",
            "8GB VRAM向けのLTX-Video 0.9.8 2B FP8です。LTX 2.5 NVFP4は別の任意機能で提供します。FP8重みはAMD実機での可否が未確認です。",
            "約14.3GB（空き18GB以上が必要）",
        )),
        "image-krea2" => Some((
            "Krea 2 Turbo画像生成（NVFP4）",
            "Windows x64で文章から画像を生成します。Blackwell以降・専用VRAM 8GB以上・RAM 32GB以上が必要です。独自ライセンスへの本人の同意が必要です。RTX 5060 Laptop 8GBで単発生成を確認済み、連続実行は未検証です。",
            "約13.2GB（実行環境と作業領域は別）",
        )),
        "video-ltx25" => Some((
            "LTX 2.5動画生成（NVFP4）",
            "Windows x64で文章または画像から短い動画を生成します。Blackwell以降・専用VRAM 8GB以上・RAM 32GB以上が必要です。提供元の利用条件への本人の同意が必要です。8GB実機で文章からの単発生成を確認済み、音声品質・画像入力・連続実行は検証中です。",
            "約35.9GB（実行環境と作業領域は別）",
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

fn manifest(_app: &AppHandle) -> Result<Manifest> {
    // 取得元情報はビルド時に実行ファイルへ埋め込んだ1つだけを使う（インストール先のファイルと
    // 埋め込みの2段にしない）。
    let parsed: Manifest =
        serde_json::from_str(include_str!("../resources/component-manifest.json"))
            .context("追加機能の取得元情報が壊れています")?;
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

/// このビルドに同梱した追加機能の置き場所。無ければ未同梱として扱い、別の場所は探さない。
fn packaged_dir(app: &AppHandle, id: &str) -> Option<PathBuf> {
    crate::app_paths::packaged_bundled_dir(app)
        .map(|root| root.join(id))
        .filter(|path| path.is_dir())
}

fn installed_dir(app: &AppHandle, id: &str) -> PathBuf {
    crate::app_paths::data_path(app, crate::app_paths::DataPath::Runtime(id))
}

/// GPUを使うPython機能は、機能ごとにPythonと同一wheelを複製せず、この1環境を共有する。
///
/// 共有するのはPython 3.12本体と仮想環境までで、PyTorchは実GPUに合うバックエンド
/// （CUDAまたはROCm）を1つだけ導入する。両方を予備として入れない。
pub(crate) fn shared_python_dir(app: &AppHandle) -> PathBuf {
    crate::app_paths::bundled_dir(app).join("python")
}

/// 仮想環境のPython実行ファイル。OSでレイアウトが違うのでここへ集約する。
///
/// Windowsは`Scripts/python.exe`、macOSは`bin/python3`。
pub(crate) fn python_executable(python_root: &Path) -> PathBuf {
    if cfg!(windows) {
        python_root.join("Scripts/python.exe")
    } else {
        python_root.join("bin/python3")
    }
}

/// 画像・動画・キャラクター生成に対応するOSとCPUアーキテクチャか。
///
/// Windowsはx64、macOSはApple Silicon（aarch64）だけを対象にする。Intel Macは対象外。
pub(crate) fn generation_platform_supported() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
        || cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// nvidia-smiがCUDA対応GPUを報告するか。NVIDIAの既存判定を変えない。
#[cfg(any(windows, target_os = "linux"))]
pub(crate) async fn nvidia_cuda_ready() -> bool {
    let mut command = crate::tools::process::command(if cfg!(windows) {
        "nvidia-smi.exe"
    } else {
        "nvidia-smi"
    });
    command.args(["--query-gpu=name", "--format=csv,noheader"]);
    crate::tools::process::output(command, 10, "NVIDIA GPU確認")
        .await
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) async fn nvidia_cuda_ready() -> bool {
    false
}

/// 取得を始める前に、実GPUから生成バックエンドを判定する。
pub(crate) async fn generation_selection() -> crate::generation_backend::BackendSelection {
    let devices = crate::gpu::devices();
    crate::generation_backend::select(&devices, nvidia_cuda_ready().await)
}

/// 導入済みの生成ランタイムの素性。記録が無い、または壊れている場合は`None`。
pub(crate) fn installed_generation_runtime(
    app: &AppHandle,
) -> Option<crate::generation_backend::InstalledRuntime> {
    let text =
        std::fs::read_to_string(installed_dir(app, "character-maker").join("install.json")).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    serde_json::from_value(value.get("runtime")?.clone()).ok()
}

/// `AppHandle`を受け取れないスキルから使う、導入済みランタイムの読み取り。
pub(crate) fn installed_generation_runtime_for_app(
) -> Option<crate::generation_backend::InstalledRuntime> {
    installed_generation_runtime(crate::durable_jobs::app_handle()?)
}

/// `AppHandle`を受け取れないスキルから使う、導入済みランタイムの照合。
pub(crate) fn generation_runtime_mismatch_for_app() -> Option<String> {
    generation_runtime_mismatch(crate::durable_jobs::app_handle()?)
}

/// 導入済みランタイムをいまのGPUで使い続けてよいか。使えない場合は日本語の理由を返す。
pub(crate) fn generation_runtime_mismatch(app: &AppHandle) -> Option<String> {
    let installed = installed_generation_runtime(app)?;
    let devices = crate::gpu::devices();
    if installed.matches_present_gpu(&devices) {
        return None;
    }
    Some(format!(
        "導入済みの生成ランタイムは{}（{}）向けですが、いまのPCでは{}を確認できません。設定の「追加機能・更新」から入れ直してください。",
        installed.gpu_vendor,
        installed.backend.label(),
        installed.gpu_vendor
    ))
}

fn installed_component_version(id: &str, root: &Path) -> Result<Option<String>> {
    let manifest_name = match id {
        "aivis" => "engine_manifest.json",
        "character-maker" | "video-wan" | "video-ltx" | "image-krea2" | "video-ltx25" => {
            "install.json"
        }
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

fn managed_media_ready(id: &str, root: &Path) -> bool {
    let expected_model = match id {
        "video-wan" => "wan-2.2-5b",
        "video-ltx" => "ltx-0.9.8-2b-fp8",
        "image-krea2" => "krea-2-turbo-nvfp4",
        "video-ltx25" => "ltx-2.5-nvfp4",
        _ => return false,
    };
    let marker = root.join("install.json");
    let Ok(text) = std::fs::read_to_string(&marker) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    if value.get("model").and_then(serde_json::Value::as_str) != Some(expected_model) {
        return false;
    }
    let Some(files) = value.get("files").and_then(serde_json::Value::as_array) else {
        return false;
    };
    !files.is_empty()
        && files.iter().all(|entry| {
            let Some(relative) = entry.as_str().map(Path::new) else {
                return false;
            };
            relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
                && root.join(relative).is_file()
        })
}

/// モデルだけを導入済みと誤認しない。専用環境の固定版と実行入口も必要とする。
fn modern_media_runtime_ready(root: &Path) -> bool {
    if !python_executable(&root.join("venv")).is_file() || !root.join("ComfyUI/main.py").is_file() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(root.join("install.json")) else {
        return false;
    };
    let Ok(marker) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let lock_hash = hex::encode(Sha256::digest(include_bytes!(
        "../../character-maker/sidecar/requirements-media.lock"
    )));
    marker.get("version").and_then(serde_json::Value::as_str) == Some("2026-09-21")
        && marker
            .get("comfyCommit")
            .and_then(serde_json::Value::as_str)
            == Some("73c9bad4d21e7addbe1d13bc92eee0f1431b017d")
        && marker.get("lockSha256").and_then(serde_json::Value::as_str) == Some(lock_hash.as_str())
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
    if installed.trim() == target.trim() {
        return Some(Ordering::Equal);
    }
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
            let installed_root = installed_dir(app, id);
            // GPUを交換した後は、導入済みでも不一致バックエンドを再利用させない。
            let runtime_mismatch = if id == "character-maker" {
                generation_runtime_mismatch(app)
            } else {
                None
            };
            let installed = if id == "character-maker" {
                runtime_mismatch.is_none()
                    && installed_root.join("install.json").is_file()
                    && python_executable(&shared_python_dir(app)).is_file()
                    && installed_root.join("ComfyUI/main.py").is_file()
            } else if matches!(
                id,
                "video-wan" | "video-ltx" | "image-krea2" | "video-ltx25"
            ) {
                managed_media_ready(id, &installed_root)
                    && (!crate::generation_backend::is_nvfp4_component(id)
                        || modern_media_runtime_ready(&installed_dir(app, "media-runtime")))
            } else {
                installed_root.is_dir()
            };
            let packaged_root = packaged_dir(app, id);
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
                && installed_version.as_deref().is_some_and(|current| {
                    target_version.as_deref().is_some_and(|target| {
                        current.trim() != target.trim()
                            && compare_versions(current, target).is_none()
                    })
                });
            if version_unknown && version_error.is_none() {
                version_error = Some(
                    "導入済み版の表記を比較できません。修復すると推奨版を取得します。".to_string(),
                );
            }
            if let Some(reason) = runtime_mismatch {
                version_error = Some(reason);
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
                // 専用runtimeが欠損していても、残った管理モデルの削除は許可する。
                removable: (installed
                    || (crate::generation_backend::is_nvfp4_component(id)
                        && installed_root.is_dir()))
                    && info.install_mode == "managed",
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
    cleanup_stale_install_stages(app).await?;
    // 導入完了・中断復旧後の状態を排他内で検査し、待機中に追加された依存も保護する。
    if id == "character-maker" {
        anyhow::ensure!(
            !installed_dir(app, "video-wan").is_dir()
                && !installed_dir(app, "video-ltx").is_dir()
                && !installed_dir(app, "image-krea2").is_dir()
                && !installed_dir(app, "video-ltx25").is_dir(),
            "先に追加の画像・動画モデルを削除してください。共通Pythonを先に削除すると生成機能が使えなくなります"
        );
    }
    if id == "aivis" {
        crate::aivis::stop(app);
    }
    let target = installed_dir(app, id);
    // 標準の記憶検索が使う共通Pythonは画像生成機能だけの削除で消さない。
    let shared_python = (id == "character-maker"
        && !installed_dir(app, "embedding-memory").join("venv").is_dir())
    .then(|| shared_python_dir(app));
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

pub async fn install(
    app: &AppHandle,
    ids: &[String],
    accepted_license_ids: &[String],
) -> Result<Vec<ComponentStatus>> {
    let manifest = manifest(app)?;
    for id in ids {
        anyhow::ensure!(
            !crate::generation_backend::is_nvfp4_component(id) || accepted_license_ids.contains(id),
            "{id}は提供元の利用条件を確認し、本人が同意してから導入してください。何も取得していません"
        );
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
    // 共通Pythonの交換前に検索を止める。導入排他を先に取ると検索の準備と相互待ちになる。
    let replacing_python = ids.iter().any(|id| id == "character-maker");
    let _search_guard = if replacing_python {
        Some(crate::memory_embedding::pause().await?)
    } else {
        None
    };
    let _operation = COMPONENT_OPERATION_LOCK.lock().await;
    cleanup_stale_install_stages(app).await?;
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
            "image-krea2" => {
                install_media_model(app, "image-krea2", "krea-2-turbo-nvfp4", 18_000_000_000)
                    .await?
            }
            "video-ltx25" => {
                install_media_model(app, "video-ltx25", "ltx-2.5-nvfp4", 45_000_000_000).await?
            }
            _ => anyhow::bail!("{id} は自動導入の対象ではありません"),
        }
    }
    statuses(app)
}

/// uvの配布形式。OSで書庫が違う。
#[derive(Clone, Copy, PartialEq, Eq)]
enum UvArchive {
    Zip,
    TarGz,
}

/// OS別のuv公式アセット。
struct UvAsset {
    url: &'static str,
    cache_name: &'static str,
    executable: &'static str,
    size: u64,
    sha256: &'static str,
    archive: UvArchive,
}

/// この環境向けのuv公式アセット。対象外のOSでは`None`。
fn uv_asset() -> Option<UvAsset> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Some(UvAsset {
            url: "https://github.com/astral-sh/uv/releases/download/0.12.23/uv-x86_64-pc-windows-msvc.zip",
            cache_name: "uv.zip.part",
            executable: "uv.exe",
            size: 18_043_715,
            sha256: "75d05de6762778c31ee183398de7dd15093fad0ed90b1f236d8205ea5ec00c90",
            archive: UvArchive::Zip,
        });
    }
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Some(UvAsset {
            url: "https://github.com/astral-sh/uv/releases/download/0.12.23/uv-aarch64-apple-darwin.tar.gz",
            cache_name: "uv.tar.gz.part",
            executable: "uv",
            size: 17_044_217,
            sha256: "50487ae565ccd96e499056b4674d438f4c53170202617b4c759defe0c6a1b544",
            archive: UvArchive::TarGz,
        });
    }
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Some(UvAsset {
            url: "https://github.com/astral-sh/uv/releases/download/0.12.23/uv-x86_64-unknown-linux-gnu.tar.gz",
            cache_name: "uv.tar.gz.part", executable: "uv", size: 19_906_150,
            sha256: "9167d72b3319674b6303c4cbe071854bba13ebdf3d76b1a7cbdc175471fb66d6",
            archive: UvArchive::TarGz,
        });
    }
    if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        return Some(UvAsset {
            url: "https://github.com/astral-sh/uv/releases/download/0.12.23/uv-aarch64-unknown-linux-gnu.tar.gz",
            cache_name: "uv.tar.gz.part", executable: "uv", size: 18_965_616,
            sha256: "6524bd338177ed50d035d39354e12545e993bbeba2ecbddf0480c5b3a81d313f",
            archive: UvArchive::TarGz,
        });
    }
    None
}

/// 記憶検索の導入用uvを公式固定資産から取得し、サイズとSHA-256を検証する。
pub(crate) async fn acquire_embedding_uv(app: &AppHandle) -> Result<PathBuf> {
    let asset = uv_asset().context("この環境向けのPython管理ツール配布がありません")?;
    let root = crate::app_paths::bundled_dir(app).join("embedding-install-tools");
    std::fs::create_dir_all(&root)?;
    let archive = root.join(asset.cache_name);
    download_to(
        app,
        crate::tools::http::client(),
        DownloadSpec {
            id: "embedding-memory",
            url: asset.url,
            destination: &archive,
            expected_size: asset.size,
            max_size: 30_000_000,
            message: "記憶検索のPython管理ツールを取得中…",
        },
    )
    .await?;
    verify_sha256(&archive, asset.sha256)?;
    let unpack = root.join("uv");
    if unpack.exists() {
        std::fs::remove_dir_all(&unpack)?;
    }
    let kind = asset.archive;
    tokio::task::spawn_blocking(move || match kind {
        UvArchive::Zip => extract_zip(&archive, &unpack),
        UvArchive::TarGz => extract_tar_gz(&archive, &unpack),
    })
    .await??;
    find_named_file(&root.join("uv"), asset.executable, 0).context("Python管理ツールがありません")
}

/// 呼出元がCOMPONENT_OPERATION_LOCKを保持し、画像モデルを取得せず共通Pythonだけ導入する。
pub(crate) async fn ensure_shared_python(app: &AppHandle, uv: &Path) -> Result<PathBuf> {
    let root = shared_python_dir(app);
    let python = python_executable(&root);
    if python.is_file() {
        return Ok(python);
    }
    std::fs::create_dir_all(&root)?;
    let runtime = root.join("runtime");
    run_labeled_command(
        uv,
        &[
            "python",
            "install",
            "3.12.13",
            "--no-bin",
            "--no-registry",
            "--install-dir",
            &runtime.to_string_lossy(),
        ],
        "記憶検索の共通Python導入",
    )
    .await?;
    let base = find_named_file(
        &runtime,
        if cfg!(windows) {
            "python.exe"
        } else {
            "python3.12"
        },
        0,
    )
    .context("共通Python 3.12.13が見つかりません")?;
    let venv = crate::app_paths::bundled_dir(app).join("embedding-install-tools/python-venv");
    if venv.exists() {
        std::fs::remove_dir_all(&venv)?;
    }
    run_labeled_command(
        uv,
        &[
            "venv",
            &venv.to_string_lossy(),
            "--python",
            &base.to_string_lossy(),
            "--relocatable",
        ],
        "記憶検索の共通Python導入",
    )
    .await?;
    copy_dir_all(&venv, &root)?;
    run_labeled_command(
        &python,
        &["-c", "import sys; assert sys.version_info[:2] == (3,12)"],
        "記憶検索の共通Python導入",
    )
    .await?;
    Ok(python)
}

/// tar.gzをOS標準のtarで展開する（macOS）。
///
/// 展開先を作ってから`-C`で中へ入れ、書庫の外へは出させない。
fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let mut command = crate::tools::process::command_sync("tar");
    let output = command
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .output()?;
    // このファイルはストリーミング受信も行うため、`text::guard`が禁じる
    // 逐次デコードなしの置換変換を使わずにエラー本文を取り出す。
    let stderr = String::from_utf8(output.stderr).unwrap_or_else(|error| {
        format!(
            "標準エラーがUTF-8ではありません（{}バイト）",
            error.as_bytes().len()
        )
    });
    anyhow::ensure!(
        output.status.success(),
        "書庫を展開できません: {}",
        stderr.trim()
    );
    Ok(())
}

async fn install_character_maker(app: &AppHandle) -> Result<()> {
    anyhow::ensure!(
        generation_platform_supported(),
        "キャラクター作成はWindows x64とApple Silicon Mac専用です（Intel Macは対象外）"
    );
    // 取得開始前にGPUを判定する。非対応GPUではランタイムも大容量モデルも取得しない。
    let selected = match generation_selection().await {
        crate::generation_backend::BackendSelection::Supported(value) => value,
        crate::generation_backend::BackendSelection::Unsupported(reason) => {
            anyhow::bail!("{reason}")
        }
    };
    let backend = selected.backend;
    let assets = character_maker_assets().context("キャラ作成資源がありません")?;
    let lock =
        crate::generation_backend::lock_path(&assets, backend).map_err(anyhow::Error::msg)?;
    emit_progress(
        app,
        "character-maker",
        "download",
        format!(
            "{}（{}）向けに{}版PyTorchを準備します",
            selected.device.name,
            selected.device.vendor.label(),
            backend.label()
        ),
        0,
        0,
    );
    let bundled = crate::app_paths::bundled_dir(app);
    std::fs::create_dir_all(&bundled)?;
    anyhow::ensure!(
        free_disk_bytes(&bundled)? >= 85_000_000_000,
        "キャラクター作成の導入には85GB以上の空き容量が必要です。何も取得していません"
    );

    // uvは同じ0.12.23をOS別の公式アセットで取る。サイズとSHA-256は公式の
    // `<資産名>.sha256`と照合済み（全4資産: 2026-10-08）。
    let uv_asset = uv_asset().context("この環境向けのPython管理ツール配布がありません")?;
    const COMFY_URL: &str =
        "https://github.com/Comfy-Org/ComfyUI/archive/12d5279438bfefc058a269eae805ceab6047777f.zip";
    const COMFY_SHA: &str = "a3c51a42bc7568d4b57267af3a1aa86a6cb379a199f95a90e8e3679c52651241";
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(120))
        .user_agent("PicoAgent/1.0 character maker installer")
        .build()?;
    let cache = crate::app_paths::cache_dir(app).join("character-maker-installer");
    let stage = installed_dir(app, "character-maker").with_file_name(format!(
        ".installing-character-maker-{}",
        std::process::id()
    ));
    let python_stage = crate::app_paths::bundled_dir(app)
        .join(format!(".installing-python-{}", std::process::id()));
    cleanup_dir(app, "古いキャラ作成導入途中フォルダ", &stage).await;
    cleanup_dir(app, "古い共通Python導入途中フォルダ", &python_stage).await;
    tokio::fs::create_dir_all(&cache).await?;
    tokio::fs::create_dir_all(&stage).await?;
    let result = async {
        let uv_zip = cache.join(uv_asset.cache_name);
        download_to(
            app,
            &client,
            DownloadSpec {
                id: "character-maker",
                url: uv_asset.url,
                destination: &uv_zip,
                expected_size: uv_asset.size,
                max_size: 30_000_000,
                message: "Python管理ツールを取得中…",
            },
        )
        .await?;
        verify_sha256(&uv_zip, uv_asset.sha256)?;
        let uv_dir = cache.join("uv");
        cleanup_dir(app, "uv展開途中フォルダ", &uv_dir).await;
        let uv_zip_copy = uv_zip.clone();
        let uv_dir_copy = uv_dir.clone();
        let archive_kind = uv_asset.archive;
        tokio::task::spawn_blocking(move || match archive_kind {
            UvArchive::Zip => extract_zip(&uv_zip_copy, &uv_dir_copy),
            UvArchive::TarGz => extract_tar_gz(&uv_zip_copy, &uv_dir_copy),
        })
        .await??;
        let uv = find_named_file(&uv_dir, uv_asset.executable, 0)
            .with_context(|| format!("{}が配布物にありません", uv_asset.executable))?;
        // uvの仮想環境は基準Pythonを参照する。キャッシュへ置くと起動時清掃で壊れるため、
        // 基準Pythonとvenvを同じ管理対象世代へまとめて原子的に公開する。
        let python_store = python_stage.join("runtime");
        run_component_command(
            &uv,
            &[
                "python",
                "install",
                "3.12.13",
                "--no-bin",
                "--no-registry",
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
        let staged_python = python_executable(&python);
        // 実GPUに必要なバックエンドのwheelだけを入れる。CUDA版とROCm版を同じ環境へ同時に導入しない。
        let mut sync = vec![
            "pip".to_string(),
            "sync".to_string(),
            "--python".to_string(),
            staged_python.to_string_lossy().to_string(),
            lock.to_string_lossy().to_string(),
            "--index-strategy".to_string(),
            "unsafe-best-match".to_string(),
            "--require-hashes".to_string(),
        ];
        if backend == crate::generation_backend::GenerationBackend::Cuda {
            // CUDA版torchはPyTorch公式インデックスから取る（既存経路を変えない）。
            // ROCm版はロック内の公式wheel URLを直接使うため追加インデックスを渡さない
            // （渡すとPyPIのCPU版torchで上書きされることがある）。
            sync.push("--extra-index-url".to_string());
            sync.push("https://download.pytorch.org/whl/cu128".to_string());
        }
        run_component_command(&uv, &sync.iter().map(String::as_str).collect::<Vec<_>>()).await?;
        // ROCmを有効にする前に、実際のGPUで動くことを確かめる。失敗したら大容量モデルを取得しない。
        emit_progress(
            app,
            "character-maker",
            "verify",
            format!("{}版PyTorchの動作を確認しています…", backend.label()),
            0,
            0,
        );
        run_component_command(
            &staged_python,
            &[
                &assets.join("sidecar/verify_backend.py").to_string_lossy(),
                "--backend",
                backend.id(),
            ],
        )
        .await
        .with_context(|| {
            format!(
                "{}版PyTorchが実GPUで動作しません。何も有効にしていません",
                backend.label()
            )
        })?;
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
            &python_executable(&python),
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
            &serde_json::json!({
                "version":"2026.09.10",
                "python":"3.12.13",
                "comfy":"v0.34.0",
                // GPU種別・バックエンド・PyTorch版・CUDA／ROCm版を残し、
                // GPU交換やドライバー変更後に古いバックエンドを再利用しない。
                "runtime": crate::generation_backend::InstalledRuntime::of(&selected),
            }),
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
    if result.is_err()
        && !["python", "character-maker"].iter().any(|id| {
            crate::app_paths::bundled_dir(app)
                .join(format!(".backup-{id}-{}", std::process::id()))
                .exists()
        })
    {
        // 復元も失敗した場合、次回起動で組を戻す判断に必要な途中配置は残す。
        cleanup_dir(app, "キャラ作成導入途中フォルダ", &stage).await;
        cleanup_dir(app, "共通Python導入途中フォルダ", &python_stage).await;
    }
    result
}

async fn install_modern_media_runtime(app: &AppHandle, id: &str, model: &str) -> Result<()> {
    crate::generation_backend::validate_nvfp4_hardware()
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        cfg!(all(target_os = "windows", target_arch = "x86_64")),
        "NVFP4専用ランタイムはWindows x64専用です"
    );
    let python = python_executable(&shared_python_dir(app));
    anyhow::ensure!(python.is_file(), "共通Pythonが未導入です。画像モデルは取得していません。既存キャラクター作成環境または開発用の共通Python導入が必要です");
    let assets = character_maker_assets().context("生成資源がありません")?;
    let cache = crate::app_paths::cache_dir(app).join("media-runtime-installer");
    tokio::fs::create_dir_all(&cache).await?;
    let model_root = installed_dir(app, id);
    tokio::fs::create_dir_all(&model_root).await?;
    // 利用条件・認証の不足で止まる場合、先に大容量の実行環境を取得しない。
    run_component_command(
        &python,
        &[
            &assets
                .join("sidecar/media/setup_models.py")
                .to_string_lossy(),
            "--model",
            model,
            "--runtime",
            &model_root.to_string_lossy(),
            "--cache",
            &cache.to_string_lossy(),
            "--progress",
            &cache.join("progress.json").to_string_lossy(),
            "--accept-license",
            "2026-09-21",
            "--preflight-only",
        ],
    )
    .await?;
    let uv_asset = uv_asset().context("Python管理ツールが未対応です")?;
    let archive = cache.join(uv_asset.cache_name);
    download_to(
        app,
        crate::tools::http::client(),
        DownloadSpec {
            id,
            url: uv_asset.url,
            destination: &archive,
            expected_size: uv_asset.size,
            max_size: 30_000_000,
            message: "専用生成環境の管理ツールを取得中…",
        },
    )
    .await?;
    verify_sha256(&archive, uv_asset.sha256)?;
    let uv_dir = cache.join("uv");
    cleanup_dir(app, "Python管理ツールの展開途中", &uv_dir).await;
    let extract_archive = archive.clone();
    let extract_root = uv_dir.clone();
    tokio::task::spawn_blocking(move || extract_zip(&extract_archive, &extract_root)).await??;
    let uv = find_named_file(&uv_dir, uv_asset.executable, 0)
        .context("Python管理ツールが見つかりません")?;
    run_component_command(
        &python,
        &[
            &assets
                .join("sidecar/media/setup_runtime.py")
                .to_string_lossy(),
            "--runtime",
            &installed_dir(app, "media-runtime").to_string_lossy(),
            "--cache",
            &cache.join("runtime").to_string_lossy(),
            "--uv",
            &uv.to_string_lossy(),
            "--accept-license",
            "2026-09-21",
        ],
    )
    .await?;
    cleanup_dir(app, "専用生成環境の取得キャッシュ", &cache).await;
    Ok(())
}

async fn install_media_model(
    app: &AppHandle,
    id: &'static str,
    model: &'static str,
    required_free: u64,
) -> Result<()> {
    anyhow::ensure!(
        generation_platform_supported(),
        "ローカル動画生成はWindows x64とApple Silicon Mac専用です（Intel Macは対象外）"
    );
    let nvfp4 = crate::generation_backend::is_nvfp4_component(id);
    let bundled = crate::app_paths::bundled_dir(app);
    std::fs::create_dir_all(&bundled)?;
    // 専用ランタイムも大容量なので、取得・更新する前に全工程の必要容量を確かめる。
    anyhow::ensure!(
        free_disk_bytes(&bundled)? >= required_free,
        "{}の導入に必要な空き容量がありません。動画モデルは取得していません",
        definitions(id).map(|value| value.0).unwrap_or(id)
    );
    if nvfp4 {
        install_modern_media_runtime(app, id, model).await?;
    }
    let runtime = installed_dir(
        app,
        if nvfp4 {
            "media-runtime"
        } else {
            "character-maker"
        },
    );
    let python = if nvfp4 {
        python_executable(&runtime.join("venv"))
    } else {
        python_executable(&shared_python_dir(app))
    };
    anyhow::ensure!(
        runtime.join("ComfyUI/main.py").is_file() && python.is_file(),
        "対応する生成ランタイムと共通Pythonを先に導入してください。モデルはまだ取得していません"
    );
    // 導入済みランタイムが今のGPU向けでないまま、動画モデルだけを取得させない。
    if !nvfp4 {
        if let Some(reason) = generation_runtime_mismatch(app) {
            anyhow::bail!("{reason}動画モデルは取得していません。");
        }
    }
    // 動画モデルも、取得開始前にGPUを判定する。非対応GPUでは何も取得しない。
    let selected = match generation_selection().await {
        crate::generation_backend::BackendSelection::Supported(value) => value,
        crate::generation_backend::BackendSelection::Unsupported(reason) => {
            anyhow::bail!("{reason}")
        }
    };
    if let Some(model) = crate::generation_backend::model_for(id) {
        if let Some(reason) = crate::generation_backend::model_memory_shortage(
            model,
            &selected.device,
            crate::engine::total_memory_bytes(),
        ) {
            anyhow::bail!("{reason}");
        }
    }
    emit_progress(
        app,
        id,
        "download",
        format!(
            "{}（{}）向けに{}で動く動画モデルを取得します",
            selected.device.name,
            selected.device.vendor.label(),
            selected.backend.label()
        ),
        0,
        0,
    );
    let stage =
        installed_dir(app, id).with_file_name(format!(".installing-{id}-{}", std::process::id()));
    let cache = crate::app_paths::cache_dir(app)
        .join("media-model-installer")
        .join(id);
    cleanup_dir(app, "古い動画モデル導入途中フォルダ", &stage).await;
    tokio::fs::create_dir_all(&stage).await?;
    tokio::fs::create_dir_all(&cache).await?;
    let progress = cache.join("progress.json");
    let assets = character_maker_assets().context("画像・動画生成資源がありません")?;
    let script = assets.join("sidecar/media/setup_models.py");
    let download_cache = cache.join("download");
    let mut arguments = vec![
        script.to_string_lossy().into_owned(),
        "--model".into(),
        model.into(),
        "--runtime".into(),
        stage.to_string_lossy().into_owned(),
        "--cache".into(),
        download_cache.to_string_lossy().into_owned(),
        "--progress".into(),
        progress.to_string_lossy().into_owned(),
    ];
    if nvfp4 {
        // 呼出元installで本人の同意を確認したモデルだけ、取得スクリプトへ渡す。
        arguments.push("--accept-license".into());
        arguments.push("2026-09-21".into());
    }
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let result = run_component_command_with_progress(app, id, &python, &arguments, &progress).await;
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
        ".installing-image-krea2-",
        ".installing-video-ltx25-",
        ".installing-media-runtime-",
    ]
    .iter()
    .find_map(|prefix| name.strip_prefix(prefix))
    .filter(|pid| *pid != current_pid)
    .and_then(|pid| pid.parse::<u32>().ok())
}

async fn cleanup_stale_install_stages(app: &AppHandle) -> Result<()> {
    let bundled = crate::app_paths::bundled_dir(app);
    let mut entries = match tokio::fs::read_dir(&bundled).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let current_pid = std::process::id().to_string();
    // 個別の途中フォルダを消す前に、Python と生成基盤を組で旧版へ戻す。
    // Python 配置直後の停止で、新 Python と旧 ComfyUI が混ざることを防ぐ。
    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        paths.push(entry.path());
    }
    for path in &paths {
        let name = path
            .file_name()
            .context("追加機能の名前がありません")?
            .to_string_lossy();
        if let Some((id, pid)) = stale_backup(&name, &current_pid) {
            if matches!(id.as_str(), "python" | "character-maker") && !process_exists(pid) {
                recover_installed_pair(&bundled, pid)?;
            }
        }
    }
    for path in paths {
        if !path.exists() {
            continue;
        }
        let name = path
            .file_name()
            .context("追加機能の名前がありません")?
            .to_string_lossy()
            .to_string();
        let stale_pid = stale_install_pid(&name, &current_pid);
        if stale_pid.is_some_and(|pid| !process_exists(pid)) {
            cleanup_dir(app, "終了済み導入途中フォルダ", &path).await;
            continue;
        }
        // 入れ替えの途中で落ちた退避先。名前にプロセスIDを含むため、拾わないと
        // 数十GBの旧版が残り続け、配置前に落ちた場合は機能が未導入に見える。
        if let Some((id, pid)) = stale_backup(&name, &current_pid) {
            if process_exists(pid) {
                continue;
            }
            let destination = bundled.join(&id);
            if destination.exists() {
                cleanup_dir(app, "入れ替え済み追加機能の退避先", &path).await;
            } else {
                match tokio::fs::rename(&path, &destination).await {
                    Ok(()) => crate::log_paths::append_app_log(
                        app,
                        &format!("中断した入れ替えから追加機能の旧版を戻しました: {id}"),
                    ),
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!(
                                "中断した入れ替えから追加機能の旧版を戻せません: {}",
                                path.display()
                            )
                        })
                    }
                }
            }
        }
    }
    Ok(())
}

fn recover_installed_pair(bundled: &Path, pid: u32) -> Result<()> {
    let pairs = ["python", "character-maker"].map(|id| {
        (
            bundled.join(format!(".installing-{id}-{pid}")),
            bundled.join(id),
            bundled.join(format!(".backup-{id}-{pid}")),
        )
    });
    if !pairs.iter().any(|(stage, _, _)| stage.exists()) {
        // 両方とも配置済みなら、後続の通常掃除で旧版だけを回収する。
        return Ok(());
    }
    for (stage, destination, backup) in &pairs {
        if !stage.exists() && destination.exists() {
            std::fs::rename(destination, stage).with_context(|| {
                format!(
                    "中断した組み合わせの新版を退避できません: {}",
                    destination.display()
                )
            })?;
        }
        if backup.exists() {
            std::fs::rename(backup, destination).with_context(|| {
                format!(
                    "中断した組み合わせの旧版を復元できません: {}",
                    backup.display()
                )
            })?;
        }
    }
    Ok(())
}

/// `replace_installed` の退避先名 `.backup-<id>-<pid>` から、別プロセスのものだけを読み取る。
/// id は管理対象の追加機能（と共通Python）に限り、知らない名前のフォルダには触らない。
fn stale_backup(name: &str, current_pid: &str) -> Option<(String, u32)> {
    let (id, pid) = name.strip_prefix(".backup-")?.rsplit_once('-')?;
    if pid == current_pid || (id != "python" && definitions(id).is_none()) {
        return None;
    }
    Some((id.to_string(), pid.parse().ok()?))
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
fn process_exists(pid: u32) -> bool {
    // キャラ作成は Apple Silicon Mac でも導入するため、macOS / Linux でも判定する。
    // `ps -p` は対象が無いと失敗で終わる。起動自体できない判定不能時は削除しない。
    match crate::tools::process::command_sync("ps")
        .args(["-p", &pid.to_string()])
        .output()
    {
        Ok(output) => output.status.success(),
        Err(_) => true,
    }
}

fn configure_component_environment(
    command: &mut tokio::process::Command,
    program: &Path,
) -> Result<()> {
    // ホームを管理領域へ隔離しても、本人のHF CLI認証ファイルの場所は維持する。
    // 秘密値は読み出さず、標準クライアントが参照するパスだけを子へ引き継ぐ。
    if std::env::var_os("HF_TOKEN_PATH").is_none() {
        let hf_home = std::env::var_os("HF_HOME").map(PathBuf::from).or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".cache/huggingface"))
        });
        if let Some(home) = hf_home {
            command.env("HF_TOKEN_PATH", home.join("token"));
        }
    }
    let root = program
        .parent()
        .unwrap_or(Path::new("."))
        .join("picoagent-runtime-data");
    let home = root.join("home");
    let cache = root.join("cache");
    let temp = root.join("temp");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&cache)?;
    std::fs::create_dir_all(&temp)?;
    command
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("HF_HOME", cache.join("huggingface"))
        .env("TORCH_HOME", cache.join("torch"))
        .env("PIP_CACHE_DIR", cache.join("pip"))
        .env("UV_CACHE_DIR", cache.join("uv"))
        .env("PYTHONPYCACHEPREFIX", cache.join("pycache"))
        .env("TMPDIR", &temp)
        .env("TMP", &temp)
        .env("TEMP", &temp);
    #[cfg(windows)]
    command
        .env("USERPROFILE", &home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"));
    Ok(())
}

async fn run_component_command(program: &Path, args: &[&str]) -> Result<()> {
    run_labeled_command(program, args, "キャラクター作成ランタイム導入").await
}

/// 時間切れ・起動失敗の文言に出す処理名を呼出元が指定する（記憶検索の導入を画像生成と誤表示しない）。
pub(crate) async fn run_labeled_command(program: &Path, args: &[&str], label: &str) -> Result<()> {
    let mut command = crate::tools::process::command(program);
    command.args(args);
    configure_component_environment(&mut command, program)?;
    let output = crate::tools::process::output(command, 7_200, label)
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
    command.args(args);
    configure_component_environment(&mut command, program)?;
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
    let cache = crate::app_paths::cache_dir(app).join("component-installer");
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
        remove_replaced_backup(&backup);
    } else {
        std::fs::rename(stage, destination)?;
    }
    Ok(())
}

/// 入れ替え後の旧版を消す。新版はもう配置済みなので、消せなくても導入は成功として扱う
/// （残った退避先は次回の `cleanup_stale_install_stages` が片付ける）。
fn remove_replaced_backup(backup: &Path) {
    if let Err(error) = std::fs::remove_dir_all(backup) {
        crate::log_paths::log_detached(&format!(
            "入れ替え前の追加機能を削除できません（次回の起動時に消します）: {} ({error})",
            backup.display()
        ));
    }
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
            remove_replaced_backup(&backup);
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
    fn 共通python削除の依存検査は導入排他と中断復旧の後に行う() {
        let source = include_str!("components.rs");
        let remove = source
            .split_once("pub async fn remove(")
            .unwrap()
            .1
            .split_once("fn emit_progress(")
            .unwrap()
            .0;
        let lock = remove
            .find("COMPONENT_OPERATION_LOCK.lock().await")
            .unwrap();
        let recovery = remove
            .find("cleanup_stale_install_stages(app).await?")
            .unwrap();
        let dependency = remove.find("if id == \"character-maker\"").unwrap();
        let deletion = remove.find("std::fs::remove_dir_all(&target)").unwrap();
        assert!(lock < recovery && recovery < dependency && dependency < deletion);
    }

    #[test]
    fn 画像動画導入の空き容量は専用ランタイム取得前に確認する() {
        let source = include_str!("components.rs");
        let install = source
            .split_once("async fn install_media_model(")
            .unwrap()
            .1
            .split_once("fn stale_install_pid(")
            .unwrap()
            .0;
        let capacity = install
            .find("free_disk_bytes(&bundled)? >= required_free")
            .unwrap();
        let runtime = install
            .find("install_modern_media_runtime(app, id, model).await?")
            .unwrap();
        assert!(capacity < runtime);
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
    fn 別プロセスの入れ替え退避先からidとpidを読む() {
        assert_eq!(
            stale_backup(".backup-character-maker-123", "999"),
            Some(("character-maker".to_string(), 123))
        );
        assert_eq!(
            stale_backup(".backup-python-123", "999"),
            Some(("python".to_string(), 123))
        );
        // この起動の入れ替え中のもの・名前が違うもの・pidでないものは触らない。
        assert_eq!(stale_backup(".backup-python-999", "999"), None);
        assert_eq!(stale_backup(".backup-python-x", "999"), None);
        assert_eq!(stale_backup(".backup--123", "999"), None);
        assert_eq!(stale_backup("python", "999"), None);
        assert_eq!(stale_backup(".backup-../x-123", "999"), None);
    }

    #[test]
    fn 組の入れ替え途中では両方を旧版へ戻し配置完了後は新版を保つ() {
        for completed in 1..=4 {
            let root = test_path("pair-recovery");
            let paths = ["python", "character-maker"].map(|id| {
                (
                    root.join(format!(".installing-{id}-123")),
                    root.join(id),
                    root.join(format!(".backup-{id}-123")),
                )
            });
            for (stage, destination, _) in &paths {
                std::fs::create_dir_all(stage).unwrap();
                std::fs::create_dir_all(destination).unwrap();
                std::fs::write(stage.join("version"), b"new").unwrap();
                std::fs::write(destination.join("version"), b"old").unwrap();
            }
            for (index, (_, destination, backup)) in paths.iter().enumerate() {
                if completed > index {
                    std::fs::rename(destination, backup).unwrap();
                }
            }
            for (index, (stage, destination, _)) in paths.iter().enumerate() {
                if completed > index + 2 {
                    std::fs::rename(stage, destination).unwrap();
                }
            }
            recover_installed_pair(&root, 123).unwrap();
            recover_installed_pair(&root, 123).unwrap();
            for (_, destination, _) in &paths {
                assert_eq!(
                    std::fs::read(destination.join("version")).unwrap(),
                    if completed == 4 { b"new" } else { b"old" }
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
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
        assert_eq!(
            super::compare_versions("wan-2.2-5b", "wan-2.2-5b"),
            Some(std::cmp::Ordering::Equal)
        );
    }

    #[test]
    fn 新生成環境は固定マーカーとpythonとcomfyの全てを必須にする() {
        let root = test_path("modern-media-runtime");
        let python = super::python_executable(&root.join("venv"));
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("ComfyUI")).unwrap();
        let marker = serde_json::json!({
            "version": "2026-09-21",
            "comfyCommit": "73c9bad4d21e7addbe1d13bc92eee0f1431b017d",
            "lockSha256": hex::encode(Sha256::digest(include_bytes!(
                "../../character-maker/sidecar/requirements-media.lock"
            ))),
        })
        .to_string();
        std::fs::write(root.join("install.json"), &marker).unwrap();
        assert!(!super::modern_media_runtime_ready(&root));
        std::fs::write(&python, b"python").unwrap();
        assert!(!super::modern_media_runtime_ready(&root));
        std::fs::write(root.join("ComfyUI/main.py"), b"main").unwrap();
        assert!(super::modern_media_runtime_ready(&root));
        std::fs::write(root.join("install.json"), b"{}").unwrap();
        assert!(!super::modern_media_runtime_ready(&root));
        std::fs::write(root.join("install.json"), &marker).unwrap();
        std::fs::remove_file(&python).unwrap();
        assert!(!super::modern_media_runtime_ready(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 動画モデルはマーカーと全ファイルが揃った場合だけ導入済みにする() {
        let root = test_path("managed-media");
        std::fs::create_dir_all(root.join("diffusion_models")).unwrap();
        std::fs::write(root.join("diffusion_models/model.safetensors"), b"model").unwrap();
        std::fs::write(
            root.join("install.json"),
            r#"{"version":"wan-2.2-5b","model":"wan-2.2-5b","files":["diffusion_models/model.safetensors"]}"#,
        )
        .unwrap();
        assert!(super::managed_media_ready("video-wan", &root));

        std::fs::remove_file(root.join("diffusion_models/model.safetensors")).unwrap();
        assert!(!super::managed_media_ready("video-wan", &root));
        assert!(!super::managed_media_ready("video-ltx", &root));
        std::fs::remove_dir_all(root).unwrap();
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
