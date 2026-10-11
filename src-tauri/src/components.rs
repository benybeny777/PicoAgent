// components.rs - 初回起動と設定画面で共用する追加機能管理。
//
// 第三者ソフトウェアはPicoAgentのReleaseから再配布しない。自動導入に対応する
// Aivis／Whisperは各公式配布元から利用者のPCへ直接取得し、それ以外は公式ページと
// 手動設定を案内する。取得物はPicoAgent管理領域だけへ置き、グローバルPATHを変更しない。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
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
    // `OnceLock::set` は2回目以降 Err を返すだけで、
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

/// CUDA 13向け資産は、対応する演算能力とドライバーを取得前に確認する。
pub(crate) async fn nvidia_cuda13_ready() -> bool {
    if !cfg!(any(windows, target_os = "linux")) {
        return false;
    }
    let mut command = crate::tools::process::command("nvidia-smi");
    command.args([
        "--query-gpu=compute_cap,driver_version",
        "--format=csv,noheader,nounits",
    ]);
    crate::tools::process::output(command, 10, "CUDA 13対応確認")
        .await
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8(output.stdout)
                    .is_ok_and(|report| crate::generation_backend::cuda13_report_supported(&report))
        })
}

/// 取得を始める前に、実GPUから生成バックエンドを判定する。
pub(crate) async fn generation_selection() -> crate::generation_backend::BackendSelection {
    let devices = crate::gpu::devices();
    crate::generation_backend::select(&devices, nvidia_cuda_ready().await)
}

/// 不在と破損を分ける導入記録。用途ごとの必須項目は各判定で検査する。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallRecord {
    version: Option<String>,
    runtime: Option<crate::generation_backend::InstalledRuntime>,
    model: Option<String>,
    files: Option<Vec<String>>,
    comfy_commit: Option<String>,
    lock_sha256: Option<String>,
    binary_release: Option<String>,
    source_commit: Option<String>,
}

fn read_record_file(root: &Path, name: &str) -> Result<Option<InstallRecord>> {
    let path = root.join(name);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("導入記録を読めません: {}", path.display()))
        }
    };
    let record: InstallRecord = serde_json::from_str(&text).map_err(|_| {
        anyhow::anyhow!("導入記録のJSONまたは項目の型が不正です: {}", path.display())
    })?;
    if let Some(version) = &record.version {
        anyhow::ensure!(
            !version.trim().is_empty(),
            "導入記録の版が空です: {}",
            path.display()
        );
    }
    Ok(Some(record))
}

fn read_install_record(root: &Path) -> Result<Option<InstallRecord>> {
    read_record_file(root, "install.json")
}

fn recorded_runtime(
    record: Option<&InstallRecord>,
) -> Result<Option<crate::generation_backend::InstalledRuntime>> {
    record
        .map(|record| {
            record
                .runtime
                .clone()
                .context("導入記録に生成ランタイムの素性がありません")
        })
        .transpose()
}

/// 導入記録の読取不能・破損はGPU検査を素通しせず呼出元へ返す。
pub(crate) fn installed_generation_runtime(
    app: &AppHandle,
) -> Result<Option<crate::generation_backend::InstalledRuntime>> {
    recorded_runtime(read_install_record(&installed_dir(app, "character-maker"))?.as_ref())
}

pub(crate) fn installed_generation_runtime_for_app(
) -> Result<Option<crate::generation_backend::InstalledRuntime>> {
    installed_generation_runtime(
        crate::durable_jobs::app_handle().context("アプリの生成環境を確認できません")?,
    )
}

pub(crate) fn generation_runtime_mismatch_for_app() -> Result<Option<String>> {
    generation_runtime_mismatch(
        crate::durable_jobs::app_handle().context("アプリの生成環境を確認できません")?,
    )
}

fn runtime_mismatch(
    installed: Option<&crate::generation_backend::InstalledRuntime>,
) -> Option<String> {
    let installed = installed?;
    if installed.matches_present_gpu(&crate::gpu::devices()) {
        return None;
    }
    Some(format!("導入済みの生成ランタイムは{}（{}）向けですが、いまのPCでは{}を確認できません。設定の「追加機能・更新」から入れ直してください。", installed.gpu_vendor, installed.backend.label(), installed.gpu_vendor))
}

pub(crate) fn generation_runtime_mismatch(app: &AppHandle) -> Result<Option<String>> {
    Ok(runtime_mismatch(
        installed_generation_runtime(app)?.as_ref(),
    ))
}

fn component_record(id: &str, root: &Path) -> Result<Option<InstallRecord>> {
    match id {
        "aivis" => read_record_file(root, "engine_manifest.json"),
        "character-maker" | "whisper" | "video-wan" | "video-ltx" | "image-krea2"
        | "video-ltx25" => read_install_record(root),
        _ => Ok(None),
    }
}

fn record_version(record: Option<&InstallRecord>) -> Result<Option<String>> {
    record
        .map(|record| record.version.clone().context("導入記録に版がありません"))
        .transpose()
}

fn media_record_ready(id: &str, root: &Path, record: Option<&InstallRecord>) -> Result<bool> {
    let expected_model = match id {
        "video-wan" => "wan-2.2-5b",
        "video-ltx" => "ltx-0.9.8-2b-fp8",
        "image-krea2" => "krea-2-turbo-nvfp4",
        "video-ltx25" => "ltx-2.5-nvfp4",
        _ => return Ok(false),
    };
    let Some(record) = record else {
        return Ok(false);
    };
    let model = record
        .model
        .as_deref()
        .context("導入記録にモデルがありません")?;
    let files = record
        .files
        .as_ref()
        .context("導入記録にモデルのファイル一覧がありません")?;
    anyhow::ensure!(!files.is_empty(), "導入記録のモデルファイル一覧が空です");
    for file in files {
        let path = Path::new(file);
        anyhow::ensure!(
            !file.is_empty()
                && path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "導入記録のモデルパスが不正です"
        );
    }
    Ok(model == expected_model && files.iter().all(|file| root.join(file).is_file()))
}

fn modern_record_ready(root: &Path, record: Option<&InstallRecord>) -> Result<bool> {
    let Some(record) = record else {
        return Ok(false);
    };
    let version = record
        .version
        .as_deref()
        .context("導入記録に生成環境の版がありません")?;
    let commit = record
        .comfy_commit
        .as_deref()
        .context("導入記録にComfyUIの固定版がありません")?;
    let lock = record
        .lock_sha256
        .as_deref()
        .context("導入記録に依存ロックの検証値がありません")?;
    let lock_hash = hex::encode(Sha256::digest(include_bytes!(
        "../../character-maker/sidecar/requirements-media.lock"
    )));
    Ok(version == "2026-09-21"
        && commit == "b0b743566f65daafc423b4fea8a2fbda94b3384a"
        && lock == lock_hash
        && python_executable(&root.join("venv")).is_file()
        && root.join("ComfyUI/main.py").is_file())
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
    let modern_root = installed_dir(app, "media-runtime");
    let modern_record = read_install_record(&modern_root);
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
            let packaged_root = packaged_dir(app, id);
            let packaged = packaged_root.is_some();
            let system = external_ready(id);
            // 管理配置に記録の異常があるとき、同梱配置へ切り替えて理由を隠さない。
            let version_root = if installed_root.is_dir() {
                Some(installed_root.clone())
            } else {
                packaged_root.clone()
            };
            let inspected = (|| -> Result<(bool, Option<String>, Option<String>)> {
                let record = version_root
                    .as_deref()
                    .map(|root| component_record(id, root))
                    .transpose()?
                    .flatten();
                let version = record_version(record.as_ref())?;
                if id == "whisper" && version_root.is_some() {
                    let marker = record
                        .as_ref()
                        .context("Whisperの固定版導入記録がありません")?;
                    let binary = marker
                        .binary_release
                        .as_deref()
                        .context("Whisperの導入記録に配布版がありません")?;
                    let commit = marker
                        .source_commit
                        .as_deref()
                        .context("Whisperの導入記録にソース版がありません")?;
                    anyhow::ensure!(
                        !binary.is_empty()
                            && commit.len() == 40
                            && commit.chars().all(|char| char.is_ascii_hexdigit()),
                        "Whisperの固定版導入記録が不正です"
                    );
                }

                let mismatch = if id == "character-maker" {
                    runtime_mismatch(recorded_runtime(record.as_ref())?.as_ref())
                } else {
                    None
                };
                let installed = if id == "character-maker" {
                    mismatch.is_none()
                        && record.is_some()
                        && python_executable(&shared_python_dir(app)).is_file()
                        && installed_root.join("ComfyUI/main.py").is_file()
                } else if matches!(
                    id,
                    "video-wan" | "video-ltx" | "image-krea2" | "video-ltx25"
                ) {
                    media_record_ready(id, &installed_root, record.as_ref())?
                        && (!crate::generation_backend::is_nvfp4_component(id)
                            || modern_record_ready(
                                &modern_root,
                                modern_record
                                    .as_ref()
                                    .map_err(|error| anyhow::anyhow!("{error:#}"))?
                                    .as_ref(),
                            )?)
                } else {
                    installed_root.is_dir()
                };
                Ok((installed, version, mismatch))
            })();
            let (installed, installed_version, mut version_error) = match inspected {
                Ok(state) => state,
                Err(error) => (
                    false,
                    None,
                    Some(format!(
                        "導入記録を確認できません: {error:#}。設定の修復操作で入れ直してください"
                    )),
                ),
            };
            let valid_configuration = version_error.is_none();
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
            Ok(ComponentStatus {
                id,
                label,
                description,
                size_label,
                ready: valid_configuration && (installed || packaged || system),
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
                removable: installed_root.is_dir() && info.install_mode == "managed",
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
async fn acquire_uv(app: &AppHandle, cache: &Path, id: &str) -> Result<PathBuf> {
    let asset = uv_asset().context("この環境向けのPython管理ツール配布がありません")?;
    std::fs::create_dir_all(cache)?;
    let archive = cache.join(asset.cache_name);
    download_to(
        app,
        DownloadSpec {
            id,
            url: asset.url,
            destination: &archive,
            expected_size: asset.size,
            max_size: 30_000_000,
            sha256: Some(asset.sha256),
            message: "Python管理ツールを取得中…",
        },
    )
    .await?;
    let unpack = prepare_cached_extraction(cache, "uv")?;
    let extract = unpack.clone();
    tokio::task::spawn_blocking(move || match asset.archive {
        UvArchive::Zip => extract_zip(&archive, &extract),
        UvArchive::TarGz => extract_tar_gz(&archive, &extract),
    })
    .await??;
    find_named_file(&unpack, asset.executable, 0)?.context("Python管理ツールがありません")
}

pub(crate) async fn acquire_embedding_uv(app: &AppHandle) -> Result<PathBuf> {
    acquire_uv(
        app,
        &crate::app_paths::bundled_dir(app).join("embedding-install-tools"),
        "embedding-memory",
    )
    .await
}

fn base_python_name(windows: bool) -> &'static str {
    if windows {
        "python.exe"
    } else {
        "python3.12"
    }
}

const MANAGED_PYTHON_FIND_ARGS: &[&str] = &[
    "python",
    "find",
    "--managed-python",
    "--no-python-downloads",
    "3.12.13",
];

fn managed_python_path(store: &Path, windows: bool, output: &str) -> Result<PathBuf> {
    let line = output.strip_suffix('\n').unwrap_or(output);
    let line = line.strip_suffix('\r').unwrap_or(line);
    anyhow::ensure!(
        !line.is_empty() && !line.contains(['\r', '\n', '\0']),
        "uvのPython応答は絶対パス一行が必要です"
    );
    let path = PathBuf::from(line);
    anyhow::ensure!(
        path.is_absolute()
            && path.file_name().and_then(|x| x.to_str()) == Some(base_python_name(windows)),
        "uvのPython応答が対象OSの管理実行ファイルではありません"
    );
    let root = crate::installer::fs::checked_directory(store)?;
    let resolved = path
        .canonicalize()
        .context("uvの管理Python実体を確認できません")?;
    anyhow::ensure!(
        resolved.starts_with(&root),
        "uvのPython応答が管理領域外です"
    );
    let metadata = std::fs::symlink_metadata(&resolved)?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "uvの管理Pythonが通常ファイルではありません"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        anyhow::ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "uvの管理Python実体がリンクです"
        );
    }
    Ok(resolved)
}

async fn install_base_python_using(
    store: &Path,
    windows: bool,
    install: impl std::future::Future<Output = Result<()>>,
    find: impl std::future::Future<Output = Result<String>>,
) -> Result<PathBuf> {
    install.await?;
    managed_python_path(store, windows, &find.await?)
}

fn managed_python_query(uv: &Path, store: &Path) -> Result<tokio::process::Command> {
    let mut command = crate::tools::process::command(uv);
    command.args(MANAGED_PYTHON_FIND_ARGS);
    configure_component_environment(&mut command, uv)?;
    command.env("UV_PYTHON_INSTALL_DIR", store);
    Ok(command)
}

async fn install_base_python(uv: &Path, store: &Path, label: &str) -> Result<PathBuf> {
    install_base_python_using(
        store,
        cfg!(windows),
        async {
            run_labeled_command(
                uv,
                &[
                    "python",
                    "install",
                    "3.12.13",
                    "--no-bin",
                    "--no-registry",
                    "--install-dir",
                    &store.to_string_lossy(),
                ],
                label,
            )
            .await
        },
        async { find_managed_python_output(uv, store, label).await },
    )
    .await
}

async fn find_managed_python_output(uv: &Path, store: &Path, label: &str) -> Result<String> {
    let command = managed_python_query(uv, store)?;
    let output = checked_component_output(command, uv, label).await?;
    String::from_utf8(output.stdout).context("uvのPython応答がUTF-8ではありません")
}

fn venv_args(base: &Path, destination: &Path) -> Vec<String> {
    [
        "venv".to_owned(),
        destination.to_string_lossy().into_owned(),
        "--python".to_owned(),
        base.to_string_lossy().into_owned(),
        "--allow-existing".to_owned(),
        "--relocatable".to_owned(),
    ]
    .into()
}
async fn create_venv(uv: &Path, base: &Path, destination: &Path, label: &str) -> Result<()> {
    let args = venv_args(base, destination);
    run_labeled_command(
        uv,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        label,
    )
    .await
}

// uv所有の基底runtimeへ向くPythonリンクだけを管理stage内の相対リンクへする。
// 共通fsのリンク拒否は緩めず、他の仮想環境リンク・特殊型を公開前に拒否する。
fn normalize_python_links(stage: &Path) -> Result<()> {
    let root = crate::installer::fs::checked_directory(stage)?;
    let runtime = crate::installer::fs::checked_directory(&root.join("runtime"))?;
    let mut directories = vec![root.clone()];
    let mut links = Vec::new();
    let mut count = 0usize;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            count += 1;
            anyhow::ensure!(
                count <= MAX_FILES,
                "Python仮想環境の項目数が導入上限を超えています"
            );
            let metadata = std::fs::symlink_metadata(&path)?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                anyhow::ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "Python仮想環境に対応外のリンクがあります: {}",
                    path.display()
                );
            }
            if metadata.file_type().is_symlink() {
                let target = path
                    .canonicalize()
                    .context("Python実行リンクの実体を確認できません")?;
                if path == root.join("lib64") {
                    anyhow::ensure!(
                        std::fs::read_link(&path)? == Path::new("lib")
                            && target == root.join("lib").canonicalize()?
                            && target.starts_with(&root)
                            && target.is_dir(),
                        "Pythonのlib64リンクは管理内のlibへの相対リンクが必要です"
                    );
                } else {
                    anyhow::ensure!(
                        path.parent() == Some(root.join("bin").as_path())
                            && path.file_name().and_then(|x| x.to_str()).is_some_and(|x| [
                                "python",
                                "python3",
                                "python3.12"
                            ]
                            .contains(&x)),
                        "Python仮想環境に対応外のリンクがあります: {}",
                        path.display()
                    );
                    anyhow::ensure!(
                        target.starts_with(&runtime) && target.is_file(),
                        "Python実行リンクが管理runtime外または通常ファイル以外です: {}",
                        path.display()
                    );
                    links.push((
                        path,
                        Path::new("../runtime").join(target.strip_prefix(&runtime)?),
                    ));
                }
            } else if metadata.is_dir() {
                if path != runtime {
                    directories.push(path);
                }
            } else {
                anyhow::ensure!(
                    metadata.is_file(),
                    "Python仮想環境に特殊ファイルがあります: {}",
                    path.display()
                );
            }
        }
    }
    #[cfg(unix)]
    for (path, target) in links {
        std::fs::remove_file(&path)?;
        std::os::unix::fs::symlink(target, path)?;
    }
    #[cfg(not(unix))]
    anyhow::ensure!(
        links.is_empty(),
        "このOSのPythonリンク配置には対応していません"
    );

    Ok(())
}

async fn prepare_shared_python(uv: &Path, stage: &Path, label: &str) -> Result<PathBuf> {
    let base = install_base_python(uv, &stage.join("runtime"), label).await?;
    create_venv(uv, &base, stage, label).await?;
    normalize_python_links(stage)?;
    let python = python_executable(stage);
    verify_shared_python(&python, label).await?;
    Ok(python)
}

async fn verify_shared_python(python: &Path, label: &str) -> Result<()> {
    run_labeled_command(
        python,
        &["-c", "import sys; assert sys.version_info[:3] == (3,12,13)"],
        label,
    )
    .await
}

async fn finalize_python_using(
    root: &Path,
    configure: impl std::future::Future<Output = Result<()>>,
    verify: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    configure.await?;
    normalize_python_links(root)?;
    verify.await
}

// uvのrelocatableはpyvenv.cfgのhomeを相対化しない。最終配置で公式設定を再生成する。
async fn finalize_shared_python(uv: &Path, root: &Path, label: &str) -> Result<()> {
    let python = python_executable(root);
    finalize_python_using(
        root,
        async {
            let store = root.join("runtime");
            let output = find_managed_python_output(uv, &store, label).await?;
            let base = managed_python_path(&store, cfg!(windows), &output)?;
            create_venv(uv, &base, root, label).await
        },
        verify_shared_python(&python, label),
    )
    .await
}

/// 呼出元がCOMPONENT_OPERATION_LOCKを保持する。共通Pythonも検証後だけ配置する。
pub(crate) async fn ensure_shared_python(app: &AppHandle, uv: &Path) -> Result<PathBuf> {
    let root = shared_python_dir(app);
    let python = python_executable(&root);
    if python.is_file() {
        verify_shared_python(&python, "共通Python確認").await?;
        return Ok(python);
    }
    let stage = root.with_file_name(format!(".installing-python-{}", std::process::id()));
    if stage.exists() {
        std::fs::remove_dir_all(&stage)?;
    }
    prepare_shared_python(uv, &stage, "共通Python導入").await?;
    crate::installer::swap::complete(
        app,
        crate::installer::swap::replace_with_validation(
            &stage,
            &root,
            "python",
            finalize_shared_python(uv, &root, "共通Python配置確認"),
        )
        .await,
    )?;
    Ok(python)
}

fn extract_options() -> crate::installer::archive::ExtractOptions {
    use crate::installer::archive::{ExtractOptions, Mode, NameEncoding, Permissions};
    ExtractOptions {
        mode: Mode::Fresh,
        name_encoding: NameEncoding::Utf8,
        permissions: Permissions::Executable,
    }
}

/// 導入専用キャッシュの展開先だけを空へ戻し、後段失敗後も新規展開を再試行できるようにする。
fn prepare_cached_extraction(cache: &Path, name: &str) -> Result<PathBuf> {
    let mut parts = Path::new(name).components();
    anyhow::ensure!(
        matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none(),
        "導入キャッシュの展開名が不正です"
    );
    let cache = crate::installer::fs::checked_directory(cache)?;
    let destination = cache.join(name);
    match std::fs::symlink_metadata(&destination) {
        Ok(_) => {
            // 祖先と対象のリンク・特殊ファイルを拒否してから、この直下の一領域だけを回収する。
            let existing = crate::installer::fs::checked_directory(&destination)?;
            anyhow::ensure!(
                existing.parent() == Some(cache.as_path()),
                "展開先が導入キャッシュの直下ではありません"
            );
            std::fs::remove_dir_all(existing)
                .context("前回の導入キャッシュ展開先を回収できません")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(destination)
}

fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<()> {
    crate::installer::archive::extract_tar_gz(
        archive,
        destination,
        component_limits(),
        extract_options(),
    )
}

async fn verify_generation_python(
    python: &Path,
    assets: &Path,
    backend: crate::generation_backend::GenerationBackend,
) -> Result<()> {
    run_component_command(
        python,
        &[
            &assets.join("sidecar/verify_backend.py").to_string_lossy(),
            "--backend",
            backend.id(),
        ],
    )
    .await
    .with_context(|| {
        format!(
            "{}版PyTorchが配置先の実GPUで動作しません。新版は有効にしていません",
            backend.label()
        )
    })
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
    const COMFY_URL: &str =
        "https://github.com/Comfy-Org/ComfyUI/archive/12d5279438bfefc058a269eae805ceab6047777f.zip";
    const COMFY_SHA: &str = "a3c51a42bc7568d4b57267af3a1aa86a6cb379a199f95a90e8e3679c52651241";
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
        let uv = acquire_uv(app, &cache, "character-maker").await?;
        // 基準Pythonもvenvも同じ管理世代へ置き、モデル取得前に実行を検証する。
        let python = python_stage.clone();
        let staged_python =
            prepare_shared_python(&uv, &python, "キャラ作成の共通Python導入").await?;
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
        verify_generation_python(&staged_python, &assets, backend).await?;
        let comfy_zip = cache.join("comfy.zip.part");
        download_to(
            app,
            DownloadSpec {
                id: "character-maker",
                url: COMFY_URL,
                destination: &comfy_zip,
                expected_size: 12_698_942,
                max_size: 30_000_000,
                sha256: Some(COMFY_SHA),
                message: "ComfyUI v0.34.0を取得中…",
            },
        )
        .await?;

        let comfy_unpack = prepare_cached_extraction(&cache, "comfy")?;
        let archive = comfy_zip.clone();
        let unpack = comfy_unpack.clone();
        tokio::task::spawn_blocking(move || extract_zip(&archive, &unpack)).await??;
        let main =
            find_named_file(&comfy_unpack, "main.py", 0)?.context("ComfyUI main.pyがありません")?;
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
        let final_python = python_executable(&shared_python_dir(app));
        replace_installed_pair(
            app,
            (&python_stage, &shared_python_dir(app), "python"),
            (
                &stage,
                &installed_dir(app, "character-maker"),
                "character-maker",
            ),
            async {
                finalize_shared_python(
                    &uv,
                    &shared_python_dir(app),
                    "キャラ作成の共通Python配置確認",
                )
                .await?;
                verify_generation_python(&final_python, &assets, backend).await
            },
        )
        .await?;
        cleanup_file(
            app,
            "uv取得ファイル",
            &cache.join(
                uv_asset()
                    .context("Python管理ツールが未対応です")?
                    .cache_name,
            ),
        )
        .await;
        cleanup_file(app, "ComfyUI取得ファイル", &comfy_zip).await;
        cleanup_dir(app, "uv展開フォルダ", &cache.join("uv")).await;
        cleanup_dir(app, "ComfyUI展開フォルダ", &comfy_unpack).await;
        cleanup_dir(
            app,
            "旧方式の共通Python仮想環境キャッシュ",
            &cache.join("python-venv"),
        )
        .await;
        cleanup_dir(app, "モデル取得キャッシュ", &cache.join("model-download")).await;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err()
        && !crate::installer::swap::has_pending(&bundled, "python").unwrap_or(true)
        && !crate::installer::swap::has_pending(&bundled, "character-maker").unwrap_or(true)
    {
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
    let uv = acquire_uv(app, &cache, id).await?;
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
        if let Some(reason) = generation_runtime_mismatch(app)? {
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
    replace_installed(app, &stage, &installed_dir(app, id), id)?;
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

static INSTALL_RECOVERY_FAILURE: crate::notice::FailureLatch = crate::notice::FailureLatch::new();

async fn cleanup_stale_install_stages(app: &AppHandle) -> Result<()> {
    let result = recover_stale_install_stages(app).await;
    INSTALL_RECOVERY_FAILURE.report(
        app,
        "component-install-recovery",
        "追加機能の中断復旧",
        result
            .as_ref()
            .map(|_| ())
            .map_err(|error| format!("追加機能の中断復旧に失敗しました: {error:#}")),
        "追加機能の中断復旧が再び利用できます",
    );
    result
}

async fn recover_stale_install_stages(app: &AppHandle) -> Result<()> {
    let bundled = crate::app_paths::bundled_dir(app);
    crate::installer::swap::recover(&bundled)?;
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
            if matches!(id.as_str(), "python" | "character-maker")
                && !crate::tools::process::process_alive(pid)?
            {
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
        if let Some(pid) = stale_pid {
            if !crate::tools::process::process_alive(pid)? {
                cleanup_dir(app, "終了済み導入途中フォルダ", &path).await;
                continue;
            }
        }
        // 入れ替えの途中で落ちた退避先。名前にプロセスIDを含むため、拾わないと
        // 数十GBの旧版が残り続け、配置前に落ちた場合は機能が未導入に見える。
        if let Some((id, pid)) = stale_backup(&name, &current_pid) {
            if crate::tools::process::process_alive(pid)? {
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
        .context("追加機能のプログラムの親フォルダを確認できません")?
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
    checked_component_output(command, program, label).await?;
    Ok(())
}

async fn checked_component_output(
    command: tokio::process::Command,
    program: &Path,
    label: &str,
) -> Result<std::process::Output> {
    let output = crate::tools::process::output(command, 7_200, label)
        .await
        .map_err(anyhow::Error::msg)?;
    let stderr = std::str::from_utf8(&output.stderr)
        .map(str::to_owned)
        .unwrap_or_else(|_| {
            format!(
                "標準エラーがUTF-8ではありません（{}バイト）",
                output.stderr.len()
            )
        });
    anyhow::ensure!(
        output.status.success(),
        "{} の実行に失敗しました: {stderr}",
        program.display()
    );
    Ok(output)
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

fn free_disk_bytes(path: &Path) -> Result<u64> {
    crate::installer::disk::available_bytes(path)
}

fn whisper_pin() -> Result<picoagent_core::whisper::ReleasePin> {
    picoagent_core::whisper::ReleasePin::from_manifest(include_str!(
        "../resources/component-manifest.json"
    ))
    .map_err(anyhow::Error::msg)
}

async fn whisper_tool(program: &Path, args: &[String], seconds: u64) -> Result<String> {
    let mut command = crate::tools::process::command(program);
    command.args(args);
    let output = crate::tools::process::output(command, seconds, "Whisperソースビルド")
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        output.status.success(),
        "Whisperビルドツールが失敗しました: {}: {}",
        program.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).context("Whisperビルドツールの応答がUTF-8ではありません")
}

async fn build_whisper_source(
    app: &AppHandle,
    pin: &picoagent_core::whisper::ReleasePin,
    cache: &Path,
    stage: &Path,
) -> Result<()> {
    let cmake = crate::tools::process::resolve_executable("cmake").context("Mac版Whisperの導入にはCMakeが必要です。CMakeを明示導入して再試行してください。SDKは自動取得しません")?;
    let version = whisper_tool(&cmake, &["--version".into()], 30).await?;
    picoagent_core::whisper::validate_cmake(&version).map_err(anyhow::Error::msg)?;
    let xcrun = Path::new("/usr/bin/xcrun");
    let mut paths = Vec::new();
    for args in picoagent_core::whisper::XCRUN_PROBES {
        let output = whisper_tool(xcrun, &args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>(), 30).await
            .context("Mac版WhisperにはC/C++コンパイラとmacOS SDKが必要です。XcodeとMetalビルドツールを明示導入して再試行してください")?;
        let path = PathBuf::from(output.trim());
        anyhow::ensure!(
            path.is_absolute() && path.exists(),
            "Mac版WhisperのビルドツールまたはSDKの実体がありません"
        );
        paths.push(path);
    }
    let probe = cache.join("compiler-probe.cpp");
    crate::store::write_atomic(&probe, picoagent_core::whisper::COMPILER_PROBE.as_bytes())?;
    let check = whisper_tool(
        &paths[1],
        &[
            "-fsyntax-only".into(),
            "-isysroot".into(),
            paths[2].to_string_lossy().into_owned(),
            probe.to_string_lossy().into_owned(),
        ],
        30,
    )
    .await;
    std::fs::remove_file(&probe)?;
    check.context("Mac版Whisperの取得前コンパイラ・SDK検査に失敗しました。何も取得していません")?;
    let archive = cache.join("whisper-source.tar.gz");
    download_to(
        app,
        DownloadSpec {
            id: "whisper",
            url: &pin.source_url(),
            destination: &archive,
            expected_size: pin.source_size,
            max_size: pin.source_size,
            sha256: Some(&pin.source_sha256),
            message: "固定版Whisperの公式ソースを取得中…",
        },
    )
    .await?;
    let unpacked = prepare_cached_extraction(cache, "source")?;
    extract_tar_gz(&archive, &unpacked)?;
    let source = unpacked.join(format!("whisper.cpp-{}", pin.source_commit));
    anyhow::ensure!(
        source.join("CMakeLists.txt").is_file(),
        "Whisper固定ソースのCMake定義がありません"
    );
    let build = cache.join("build");
    whisper_tool(
        &cmake,
        &picoagent_core::whisper::configure_args(&source, &build, &paths[0], &paths[1], &paths[2]),
        120,
    )
    .await?;
    whisper_tool(&cmake, &picoagent_core::whisper::build_args(&build), 900).await?;
    std::fs::create_dir_all(stage)?;
    let binary = build.join("bin/whisper-cli");
    anyhow::ensure!(binary.is_file(), "WhisperソースビルドのCLIがありません");
    std::fs::copy(binary, stage.join(whisper_exe_name()))?;
    Ok(())
}

async fn install_whisper(app: &AppHandle) -> Result<()> {
    let pin = whisper_pin()?;
    let cache = crate::app_paths::cache_dir(app).join("component-installer/whisper");
    tokio::fs::create_dir_all(&cache).await?;
    let stage = installed_dir(app, "whisper")
        .with_file_name(format!(".installing-whisper-{}", std::process::id()));
    cleanup_dir(app, "古いWhisper導入途中フォルダ", &stage).await;
    let result = async {
        if picoagent_core::whisper::binary_name(std::env::consts::OS, std::env::consts::ARCH).map_err(anyhow::Error::msg)?.is_none() {
            build_whisper_source(app, &pin, &cache, &stage).await?;
        } else {
            let release = crate::installer::http::client().map_err(anyhow::Error::msg)?.get(pin.release_api()).send().await?.error_for_status()?.json::<serde_json::Value>().await?;
            pin.validate_release(&release).map_err(anyhow::Error::msg)?;
            let (asset_url, asset_size, asset_sha256) = whisper_asset(&release)?;
            let source_name = picoagent_core::whisper::binary_name(std::env::consts::OS, std::env::consts::ARCH).map_err(anyhow::Error::msg)?.context("Whisper公式配布名がありません")?;
            let archive = cache.join(source_name);
            download_to(app, DownloadSpec { id: "whisper", url: &asset_url, destination: &archive, expected_size: asset_size,
                max_size: MAX_DOWNLOAD_BYTES, sha256: Some(&asset_sha256), message: "固定版Whisperの公式配布を取得中…" }).await?;
            let unpacked = prepare_cached_extraction(&cache, "binary")?;
            if source_name.ends_with(".tar.gz") { extract_tar_gz(&archive, &unpacked)?; } else { extract_zip(&archive, &unpacked)?; }
            let executable = find_named_file(&unpacked, whisper_exe_name(), 0)?.context("公式Whisper配布にCLIがありません")?;
            copy_dir_all(executable.parent().context("Whisper CLIの親フォルダがありません")?, &stage)?;
        }
        let model = stage.join("models/ggml-small.bin");
        tokio::fs::create_dir_all(model.parent().context("Whisperモデルの親フォルダがありません")?).await?;
        download_to(app, DownloadSpec { id: "whisper", url: WHISPER_MODEL_URL, destination: &model, expected_size: 487_601_967,
            max_size: MAX_DOWNLOAD_BYTES, sha256: Some(WHISPER_MODEL_SHA256), message: "公式Whisper smallモデルを取得中…" }).await?;
        crate::store::write_json_atomic(&stage.join("whisper.json"), &serde_json::json!({"program": whisper_exe_name(), "model": "models/ggml-small.bin",
            "args": "-m {model} -f {audio} -otxt -of {output_stem}", "diarizeArgs": "-tdrz"}))?;
        crate::store::write_json_atomic(&stage.join("install.json"), &serde_json::json!({"version": pin.target_version, "binaryRelease": pin.binary_release, "sourceCommit": pin.source_commit}))?;
        replace_installed(app, &stage, &installed_dir(app, "whisper"), "whisper")?;
        cleanup_dir(app, "Whisper取得・ビルドキャッシュ", &cache).await;
        emit_progress(app, "whisper", "ready", "Whisper固定版を公式配布元から導入しました", 0, 0);
        Ok::<_, anyhow::Error>(())
    }.await;
    if result.is_err()
        && !crate::installer::swap::has_pending(
            stage.parent().context("Whisper管理領域がありません")?,
            "whisper",
        )
        .unwrap_or(true)
    {
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
    let suffix = picoagent_core::whisper::binary_name(std::env::consts::OS, std::env::consts::ARCH)
        .map_err(anyhow::Error::msg)?
        .context("この環境のWhisperは公式ソースをビルドします")?;
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
    sha256: Option<&'a str>,
    message: &'a str,
}

async fn download_to(app: &AppHandle, spec: DownloadSpec<'_>) -> Result<()> {
    crate::installer::download::fetch_fixed(
        crate::installer::download::FetchSpec {
            url: spec.url,
            destination: spec.destination,
            expected_size: Some(spec.expected_size),
            max_size: spec.max_size,
            sha256: spec.sha256,
        },
        &mut |downloaded, total| {
            emit_progress(app, spec.id, "download", spec.message, downloaded, total)
        },
    )
    .await
}

fn component_limits() -> crate::installer::Limits {
    crate::installer::Limits {
        max_entries: MAX_FILES,
        max_total_bytes: MAX_EXTRACTED_BYTES,
        max_file_bytes: MAX_EXTRACTED_BYTES,
        file_limit: None,
    }
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    crate::installer::archive::extract_zip(
        archive,
        destination,
        component_limits(),
        extract_options(),
    )
}

fn find_named_file(dir: &Path, name: &str, depth: usize) -> Result<Option<PathBuf>> {
    crate::installer::fs::find_file(dir, name, 8usize.saturating_sub(depth))
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<()> {
    crate::installer::fs::copy_dir(source, destination, component_limits())
}

fn replace_installed(app: &AppHandle, stage: &Path, destination: &Path, id: &str) -> Result<()> {
    crate::installer::swap::complete(app, crate::installer::swap::replace(stage, destination, id))
}

async fn replace_installed_pair(
    app: &AppHandle,
    first: (&Path, &Path, &str),
    second: (&Path, &Path, &str),
    validate: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    crate::installer::swap::complete(
        app,
        crate::installer::swap::replace_pair_with_validation(first, second, validate).await,
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    fn installed_component_version(id: &str, root: &Path) -> Result<Option<String>> {
        record_version(component_record(id, root)?.as_ref())
    }

    fn managed_media_ready(id: &str, root: &Path) -> Result<bool> {
        media_record_ready(id, root, read_install_record(root)?.as_ref())
    }

    fn modern_media_runtime_ready(root: &Path) -> Result<bool> {
        modern_record_ready(root, read_install_record(root)?.as_ref())
    }

    use super::*;

    fn test_path(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().canonicalize().unwrap().join(format!(
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
        let components_source = include_str!("components.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let maker_source = include_str!("character_maker.rs");
        assert_eq!(components_source.matches(".join(\"python\")").count(), 1);
        assert!(!maker_source.contains(".join(\"python\")"));
        assert!(maker_source.contains("components::shared_python_dir(app)"));
        assert!(components_source.contains("stage.join(\"runtime\")"));
        assert!(components_source.contains("\"--relocatable\""));
        assert_eq!(
            components_source
                .matches("async fn install_base_python(")
                .count(),
            1
        );
        assert_eq!(
            components_source.matches("async fn create_venv(").count(),
            1
        );
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
    fn 導入の後段失敗後も専用展開先だけを作り直して再試行できる() {
        for (name, format) in [
            ("uv", UvArchive::Zip),
            ("uv", UvArchive::TarGz),
            ("comfy", UvArchive::Zip),
            ("binary", UvArchive::Zip),
            ("binary", UvArchive::TarGz),
            ("source", UvArchive::TarGz),
        ] {
            let directory = crate::installer::fs::test_directory();
            let cache = directory.path();
            let archive = cache.join("archive");
            let file = std::fs::File::create(&archive).unwrap();
            match format {
                UvArchive::Zip => {
                    let mut writer = zip::ZipWriter::new(file);
                    writer
                        .start_file("runtime", zip::write::SimpleFileOptions::default())
                        .unwrap();
                    writer.write_all(b"verified").unwrap();
                    writer.finish().unwrap();
                }
                UvArchive::TarGz => {
                    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
                    let mut writer = tar::Builder::new(encoder);
                    let mut header = tar::Header::new_gnu();
                    header.set_mode(0o644);
                    header.set_size(8);
                    header.set_cksum();
                    writer
                        .append_data(&mut header, "runtime", &b"verified"[..])
                        .unwrap();
                    writer.into_inner().unwrap().finish().unwrap();
                }
            }
            let original_archive = std::fs::read(&archive).unwrap();
            let sibling = cache.join("保持する取得キャッシュ");
            std::fs::write(&sibling, b"retain").unwrap();
            let attempt = |fail_after_extract: bool| -> Result<()> {
                let unpack = prepare_cached_extraction(cache, name)?;
                match format {
                    UvArchive::Zip => extract_zip(&archive, &unpack)?,
                    UvArchive::TarGz => extract_tar_gz(&archive, &unpack)?,
                }
                anyhow::ensure!(!fail_after_extract, "展開後のモデル取得に失敗しました");
                Ok(())
            };
            assert!(attempt(true).unwrap_err().to_string().contains("展開後"));
            let unpack = cache.join(name);
            std::fs::write(unpack.join("前回の残物"), b"stale").unwrap();
            attempt(false).unwrap();
            assert_eq!(std::fs::read(unpack.join("runtime")).unwrap(), b"verified");
            assert!(!unpack.join("前回の残物").exists());
            assert_eq!(std::fs::read(&archive).unwrap(), original_archive);
            assert_eq!(std::fs::read(sibling).unwrap(), b"retain");
        }
    }

    #[test]
    fn 展開先回収は管理直下以外と通常ファイルを拒否する() {
        let directory = crate::installer::fs::test_directory();
        let cache = directory.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        let existing = cache.join("uv");
        std::fs::write(&existing, b"retain").unwrap();
        for name in ["", ".", "..", "../outside", "nested/uv", "uv"] {
            assert!(prepare_cached_extraction(&cache, name).is_err(), "{name}");
        }
        assert_eq!(std::fs::read(&existing).unwrap(), b"retain");
    }

    #[cfg(unix)]
    #[test]
    fn 展開先回収は対象と祖先のリンクを辿らない() {
        let directory = crate::installer::fs::test_directory();
        let cache = directory.path().join("cache");
        let outside = directory.path().join("outside");
        std::fs::create_dir(&cache).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("retain"), b"retain").unwrap();
        std::os::unix::fs::symlink(&outside, cache.join("uv")).unwrap();
        assert!(prepare_cached_extraction(&cache, "uv").is_err());
        let ancestor = directory.path().join("linked-cache");
        std::os::unix::fs::symlink(&outside, &ancestor).unwrap();
        assert!(prepare_cached_extraction(&ancestor, "uv").is_err());
        assert_eq!(std::fs::read(outside.join("retain")).unwrap(), b"retain");
        assert!(std::fs::symlink_metadata(cache.join("uv"))
            .unwrap()
            .file_type()
            .is_symlink());
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
            "comfyCommit": "b0b743566f65daafc423b4fea8a2fbda94b3384a",
            "lockSha256": hex::encode(Sha256::digest(include_bytes!(
                "../../character-maker/sidecar/requirements-media.lock"
            ))),
        })
        .to_string();
        std::fs::write(root.join("install.json"), &marker).unwrap();
        assert!(!modern_media_runtime_ready(&root).unwrap());
        std::fs::write(&python, b"python").unwrap();
        assert!(!modern_media_runtime_ready(&root).unwrap());
        std::fs::write(root.join("ComfyUI/main.py"), b"main").unwrap();
        assert!(modern_media_runtime_ready(&root).unwrap());
        std::fs::write(
            root.join("install.json"),
            marker.replace(
                "b0b743566f65daafc423b4fea8a2fbda94b3384a",
                "73c9bad4d21e7addbe1d13bc92eee0f1431b017d",
            ),
        )
        .unwrap();
        assert!(!modern_media_runtime_ready(&root).unwrap());
        std::fs::write(root.join("install.json"), b"{}").unwrap();
        assert!(modern_media_runtime_ready(&root).is_err());
        std::fs::write(root.join("install.json"), &marker).unwrap();
        std::fs::remove_file(&python).unwrap();
        assert!(!modern_media_runtime_ready(&root).unwrap());
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
        assert!(managed_media_ready("video-wan", &root).unwrap());

        std::fs::remove_file(root.join("diffusion_models/model.safetensors")).unwrap();
        assert!(!managed_media_ready("video-wan", &root).unwrap());
        assert!(!managed_media_ready("video-ltx", &root).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn aivisの実体マニフェストから導入版を読む() {
        let root = test_path("aivis-version");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("engine_manifest.json"), br#"{"version":"1.2.3"}"#).unwrap();
        assert_eq!(
            installed_component_version("aivis", &root)
                .unwrap()
                .as_deref(),
            Some("1.2.3")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn 導入記録は不在と読取不能と不正型を区別して原本を保持する() {
        let root = test_path("install-record");
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("install.json");
        assert!(read_install_record(&root).unwrap().is_none());
        for text in [
            "壊れたJSON",
            "[]",
            r#"{"runtime":"秘密の値"}"#,
            r#"{"files":[7]}"#,
            r#"{"version":""}"#,
        ] {
            std::fs::write(&marker, text).unwrap();
            let error = read_install_record(&root).unwrap_err();
            assert!(!format!("{error:#}").contains("秘密の値"));
            assert_eq!(std::fs::read_to_string(&marker).unwrap(), text);
        }
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert!(read_install_record(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 素性欠損とモデル不正の導入記録では依存工程へ進まない() {
        let root = test_path("record-consumer");
        std::fs::create_dir_all(&root).unwrap();
        for text in ["{}", r#"{"version":"2026.09.10"}"#, r#"{"runtime":null}"#] {
            std::fs::write(root.join("install.json"), text).unwrap();
            let mut downstream = 0;
            let result = (|| -> Result<()> {
                let record = read_install_record(&root)?;
                recorded_runtime(record.as_ref())?;
                downstream += 1;
                Ok(())
            })();
            assert!(result.is_err());
            assert_eq!(downstream, 0);
        }
        for files in [
            serde_json::json!([]),
            serde_json::json!(["../outside"]),
            serde_json::json!([""]),
        ] {
            crate::store::write_json_atomic(
                &root.join("install.json"),
                &serde_json::json!({"model":"wan-2.2-5b", "files":files}),
            )
            .unwrap();
            assert!(managed_media_ready("video-wan", &root).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn 管理pythonの問い合わせは指定osと領域だけを採用し失敗後venvへ進まない() {
        for windows in [false, true] {
            let root = test_path("base-python");
            std::fs::create_dir_all(&root).unwrap();
            let expected = root.join(base_python_name(windows));
            std::fs::write(&expected, b"python").unwrap();
            let answer = format!("{}\n", expected.display());
            assert_eq!(
                install_base_python_using(&root, windows, async { Ok(()) }, async {
                    Ok(answer.clone())
                })
                .await
                .unwrap(),
                expected.canonicalize().unwrap()
            );
            for invalid in [
                String::new(),
                format!("{answer}{answer}"),
                format!("{answer}\n"),
                "python3.12".into(),
                root.to_string_lossy().into_owned(),
                root.join(base_python_name(!windows))
                    .to_string_lossy()
                    .into_owned(),
            ] {
                assert!(managed_python_path(&root, windows, &invalid).is_err());
            }
            let outside = test_path("outside-python");
            std::fs::create_dir_all(&outside).unwrap();
            let external = outside.join(base_python_name(windows));
            std::fs::write(&external, b"python").unwrap();
            assert!(managed_python_path(&root, windows, &external.to_string_lossy()).is_err());
            #[cfg(unix)]
            {
                let alias = root.join("bin");
                std::fs::create_dir_all(&alias).unwrap();
                let link = alias.join(base_python_name(windows));
                std::os::unix::fs::symlink(&expected, &link).unwrap();
                assert_eq!(
                    managed_python_path(&root, windows, &link.to_string_lossy()).unwrap(),
                    expected.canonicalize().unwrap()
                );
                std::fs::remove_file(&link).unwrap();
                std::os::unix::fs::symlink(&external, &link).unwrap();
                assert!(managed_python_path(&root, windows, &link.to_string_lossy()).is_err());
            }
            let finds = std::cell::Cell::new(0);
            let venvs = std::cell::Cell::new(0);
            for install_ok in [false, true] {
                let result = async {
                    install_base_python_using(
                        &root,
                        windows,
                        async {
                            anyhow::ensure!(install_ok, "uv導入失敗");
                            Ok(())
                        },
                        async {
                            finds.set(finds.get() + 1);
                            anyhow::bail!("uv問い合わせ失敗")
                        },
                    )
                    .await?;
                    venvs.set(venvs.get() + 1);
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                assert!(result.is_err());
            }
            assert_eq!(finds.get(), 1);
            assert_eq!(venvs.get(), 0);
            std::fs::remove_dir_all(root).unwrap();
            std::fs::remove_dir_all(outside).unwrap();
        }
    }
    #[test]
    fn 管理python問い合わせは環境管理先と取得禁止argvを固定する() {
        let root = test_path("uv-query");
        std::fs::create_dir_all(&root).unwrap();
        let uv = root.join(if cfg!(windows) { "uv.exe" } else { "uv" });
        let store = root.join("runtime");
        let command = managed_python_query(&uv, &store).unwrap();
        let standard = command.as_std();
        assert_eq!(
            standard.get_args().collect::<Vec<_>>(),
            MANAGED_PYTHON_FIND_ARGS
                .iter()
                .map(std::ffi::OsStr::new)
                .collect::<Vec<_>>()
        );
        assert!(
            standard
                .get_envs()
                .any(|(key, value)| key == "UV_PYTHON_INSTALL_DIR"
                    && value == Some(store.as_os_str()))
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn 仮想環境は管理stageへ直接生成して基底runtimeを保持する() {
        let stage = Path::new("managed-stage");
        let base = stage.join("runtime").join(base_python_name(cfg!(windows)));
        assert_eq!(
            venv_args(&base, stage),
            vec![
                "venv".to_owned(),
                stage.to_string_lossy().into_owned(),
                "--python".to_owned(),
                base.to_string_lossy().into_owned(),
                "--allow-existing".to_owned(),
                "--relocatable".to_owned()
            ]
        );
        let source = include_str!("components.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let prepare = source
            .split("async fn prepare_shared_python(")
            .nth(1)
            .unwrap()
            .split("async fn verify_shared_python")
            .next()
            .unwrap();
        assert!(prepare.contains("create_venv(uv, &base, stage, label).await?"));
        assert!(!prepare.contains("copy_dir_all"));
        assert!(
            prepare.find("normalize_python_links(stage)?").unwrap()
                < prepare
                    .find("verify_shared_python(&python, label).await?")
                    .unwrap()
        );
    }
    #[cfg(unix)]
    #[test]
    fn 管理pythonリンクは移動後も同世代runtimeだけを指す() {
        let root = test_path("python-relative-links");
        let stage = root.join("stage");
        let base = stage.join("runtime/bin/python3.12");
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        std::fs::write(&base, b"python").unwrap();
        std::fs::create_dir_all(stage.join("bin")).unwrap();
        for name in ["python", "python3", "python3.12"] {
            std::os::unix::fs::symlink(&base, stage.join("bin").join(name)).unwrap();
        }
        normalize_python_links(&stage).unwrap();
        let destination = root.join("python");
        std::fs::rename(&stage, &destination).unwrap();
        for name in ["python", "python3", "python3.12"] {
            let link = destination.join("bin").join(name);
            assert!(!std::fs::read_link(&link).unwrap().is_absolute());
            assert_eq!(
                link.canonicalize().unwrap(),
                destination
                    .join("runtime/bin/python3.12")
                    .canonicalize()
                    .unwrap()
            );
        }
        let outside = root.join("outside");
        std::fs::write(&outside, b"external").unwrap();
        std::fs::remove_file(destination.join("bin/python")).unwrap();
        std::os::unix::fs::symlink(&outside, destination.join("bin/python")).unwrap();
        assert!(normalize_python_links(&destination).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"external");
        std::fs::remove_file(destination.join("bin/python")).unwrap();
        std::os::unix::fs::symlink(
            destination.join("runtime/bin/python3.12"),
            destination.join("bin/other"),
        )
        .unwrap();
        assert!(normalize_python_links(&destination).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn 最終python設定の再生成は依存を保持し失敗時旧版を復元する() {
        for (fail, paired) in [(false, false), (true, false), (false, true), (true, true)] {
            let root = test_path("final-python");
            let stage = root.join(format!(".installing-python-{}", std::process::id()));
            let destination = root.join("python");
            let runtime = stage.join("runtime");
            std::fs::create_dir_all(&runtime).unwrap();
            std::fs::write(runtime.join(base_python_name(cfg!(windows))), b"python").unwrap();
            std::fs::create_dir_all(stage.join("lib/site-packages")).unwrap();
            std::fs::write(stage.join("lib/site-packages/keep"), b"dependency").unwrap();
            std::fs::write(stage.join("pyvenv.cfg"), "home = old-stage").unwrap();
            std::fs::create_dir_all(&destination).unwrap();
            std::fs::write(destination.join("old"), b"old-python").unwrap();
            let maker_stage = root.join(format!(
                ".installing-character-maker-{}",
                std::process::id()
            ));
            let maker = root.join("character-maker");
            std::fs::create_dir_all(&maker_stage).unwrap();
            std::fs::write(maker_stage.join("value"), b"new-maker").unwrap();
            std::fs::create_dir_all(&maker).unwrap();
            std::fs::write(maker.join("value"), b"old-maker").unwrap();
            let configured = std::cell::Cell::new(false);
            let verified = std::cell::Cell::new(false);
            let validation = finalize_python_using(
                &destination,
                async {
                    assert!(destination.join("runtime").is_dir());
                    let final_base = destination
                        .join("runtime")
                        .join(base_python_name(cfg!(windows)));
                    let args = venv_args(&final_base, &destination);
                    assert_eq!(args[1], destination.to_string_lossy().into_owned());
                    assert_eq!(args[3], final_base.to_string_lossy().into_owned());
                    assert_eq!(
                        std::fs::read(destination.join("lib/site-packages/keep"))?,
                        b"dependency"
                    );
                    anyhow::ensure!(!fail, "uv最終配置設定失敗");
                    std::fs::write(
                        destination.join("pyvenv.cfg"),
                        format!("home = {}", final_base.parent().unwrap().display()),
                    )?;
                    configured.set(true);
                    Ok(())
                },
                async {
                    assert!(configured.get());
                    assert!(std::fs::read_to_string(destination.join("pyvenv.cfg"))?
                        .contains(&destination.join("runtime").to_string_lossy().to_string()));
                    assert_eq!(
                        std::fs::read(destination.join("lib/site-packages/keep"))?,
                        b"dependency"
                    );
                    verified.set(true);
                    Ok(())
                },
            );
            let result = if paired {
                crate::installer::swap::replace_pair_with_validation(
                    (&stage, &destination, "python"),
                    (&maker_stage, &maker, "character-maker"),
                    validation,
                )
                .await
            } else {
                crate::installer::swap::replace_with_validation(
                    &stage,
                    &destination,
                    "python",
                    validation,
                )
                .await
            };
            if paired {
                assert_eq!(
                    std::fs::read(maker.join("value")).unwrap(),
                    if fail { b"old-maker" } else { b"new-maker" }
                );
            }
            assert_eq!(result.is_err(), fail, "最終配置の検証結果: {result:?}");
            assert_eq!(verified.get(), !fail);
            if fail {
                assert_eq!(
                    std::fs::read(destination.join("old")).unwrap(),
                    b"old-python"
                );
            } else {
                assert!(!destination.join("old").exists());
            }
            assert!(!crate::installer::swap::has_pending(&root, "python").unwrap());
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[cfg(unix)]
    #[test]
    fn linuxのlib64は管理内libへの正確な相対リンクだけ許可する() {
        let root = test_path("python-lib64");
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::os::unix::fs::symlink("lib", root.join("lib64")).unwrap();
        normalize_python_links(&root).unwrap();
        assert_eq!(
            std::fs::read_link(root.join("lib64")).unwrap(),
            Path::new("lib")
        );
        std::fs::remove_file(root.join("lib64")).unwrap();
        std::os::unix::fs::symlink(root.join("lib"), root.join("lib64")).unwrap();
        assert!(normalize_python_links(&root).is_err());
        std::fs::remove_file(root.join("lib64")).unwrap();
        std::os::unix::fs::symlink("runtime", root.join("lib64")).unwrap();
        assert!(normalize_python_links(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
