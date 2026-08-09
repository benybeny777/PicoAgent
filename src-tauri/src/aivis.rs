// aivis.rs - 同梱 AivisSpeech Engine の接続確認と裏起動を管理する。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::Manager;
use tokio::io::AsyncWriteExt;

use crate::commands::AppState;

const DEFAULT_PORT: u16 = 11436;
const LEGACY_DEFAULT_PORT: u16 = 10102;
const AIVIS_RELEASE: &str = "1.2.0";
const AIVIS_API_URL: &str =
    "https://api.github.com/repos/Aivis-Project/AivisSpeech-Engine/releases/tags/1.2.0";
const DEFAULT_DEVICE: &str = "auto";
const STARTUP_CHECKS: usize = 360;
const MODEL_INSTALL_TIMEOUT: Duration = Duration::from_secs(900);

struct AivisInstallCleanup(PathBuf);

impl Drop for AivisInstallCleanup {
    fn drop(&mut self) {
        // フォールバック許可: 正常・失敗・中断時のPicoAgent管理下一時物だけを掃除する。
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct AivisState {
    child: Mutex<Option<Child>>,
    /// `stop()` が呼ばれたら true に設定し、`ensure_started` がその後に spawn しないようにする。
    stopping: AtomicBool,
}

impl AivisState {
    pub fn new() -> Self {
        AivisState {
            child: Mutex::new(None),
            stopping: AtomicBool::new(false),
        }
    }
}

impl Default for AivisState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn port(app: &tauri::AppHandle, state: &AppState) -> u16 {
    let raw = state
        .config
        .get("PICOAGENT_TTS_PORT", &DEFAULT_PORT.to_string());
    let port = match raw.parse::<u16>() {
        Ok(port) => port,
        Err(error) => {
            let _ = append_log(
                app,
                &format!(
                    "Aivis ポート設定が不正なため既定値 {DEFAULT_PORT} を使います: {raw} ({error})"
                ),
            );
            DEFAULT_PORT
        }
    };
    if port == LEGACY_DEFAULT_PORT {
        let next = DEFAULT_PORT.to_string();
        if let Err(error) = state.config.set_string("PICOAGENT_TTS_PORT", next.clone()) {
            let _ = append_log(
                app,
                &format!("Aivis ポートの旧既定値移行に失敗しました: {error}"),
            );
        } else {
            let _ = append_log(
                app,
                &format!(
                    "Aivis ポートを旧既定値 {LEGACY_DEFAULT_PORT} から {next} へ移行しました。"
                ),
            );
        }
        return DEFAULT_PORT;
    }
    port
}

pub fn endpoint(app: &tauri::AppHandle, state: &AppState) -> String {
    format!("http://127.0.0.1:{}", port(app, state))
}

async fn is_up(endpoint: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build();
    match client {
        Ok(client) => client
            .get(format!("{endpoint}/version"))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// 指定 speaker_uuid のモデルが未導入なら AivisHub から AIVMX を導入する。
/// 導入確認は style ID ではなく speaker_uuid の有無で行う（global style ID は
/// エンジンが導入時に算出するため、事前にハードコードした値と一致しないことがある）。
pub async fn ensure_model_installed(
    app: &tauri::AppHandle,
    endpoint: &str,
    model_uuid: &str,
    speaker_uuid: &str,
) -> anyhow::Result<()> {
    if speaker_uuid_available(endpoint, speaker_uuid).await? {
        return Ok(());
    }
    let download_url = format!(
        "https://api.aivis-project.com/v1/aivm-models/{model_uuid}/download?model_type=AIVMX"
    );
    append_log(
        app,
        &format!("Aivis モデルを導入します: model_uuid={model_uuid} speaker_uuid={speaker_uuid}"),
    )?;
    let client = reqwest::Client::builder()
        .timeout(MODEL_INSTALL_TIMEOUT)
        .build()?;
    let form = reqwest::multipart::Form::new().text("url", download_url);
    let response = client
        .post(format!("{endpoint}/aivm_models/install"))
        .multipart(form)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Aivis モデル導入に失敗しました: HTTP {status} {body}");
    }
    for _ in 0..60 {
        if speaker_uuid_available(endpoint, speaker_uuid).await? {
            append_log(
                app,
                &format!(
                    "Aivis モデル導入完了: model_uuid={model_uuid} speaker_uuid={speaker_uuid}"
                ),
            )?;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!(
        "Aivis モデル導入後に speaker_uuid={speaker_uuid} が /speakers に見つかりません。"
    );
}

/// `/speakers` を取得する。
async fn fetch_speakers(endpoint: &str) -> anyhow::Result<Vec<Value>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;
    let response = client.get(format!("{endpoint}/speakers")).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("Aivis /speakers failed: HTTP {}", response.status());
    }
    let speakers: Value = response.json().await?;
    speakers
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Aivis /speakers の応答形式が配列ではありません。"))
}

async fn speaker_uuid_available(endpoint: &str, speaker_uuid: &str) -> anyhow::Result<bool> {
    Ok(fetch_speakers(endpoint)
        .await?
        .iter()
        .any(|speaker| speaker.get("speaker_uuid").and_then(Value::as_str) == Some(speaker_uuid)))
}

/// speaker_uuid（＋任意でスタイル名）から、合成に使う実 style ID を `/speakers` から解決する。
/// スタイル名が一致しなければ先頭スタイルを使う。
pub async fn resolve_style_id(
    endpoint: &str,
    speaker_uuid: &str,
    style_name: Option<&str>,
) -> anyhow::Result<String> {
    let speakers = fetch_speakers(endpoint).await?;
    let speaker = speakers
        .iter()
        .find(|speaker| speaker.get("speaker_uuid").and_then(Value::as_str) == Some(speaker_uuid))
        .ok_or_else(|| {
            anyhow::anyhow!("speaker_uuid={speaker_uuid} が /speakers に見つかりません。")
        })?;
    let styles = speaker
        .get("styles")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("speaker_uuid={speaker_uuid} に styles がありません。"))?;
    let style = style_name
        .and_then(|name| {
            styles
                .iter()
                .find(|style| style.get("name").and_then(Value::as_str) == Some(name))
        })
        .or_else(|| styles.first())
        .ok_or_else(|| {
            anyhow::anyhow!("speaker_uuid={speaker_uuid} に利用可能な style がありません。")
        })?;
    let id = style
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("style ID を取得できません。"))?;
    Ok(id.to_string())
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn runtime_engine_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "run.exe" } else { "run" };
    [
        crate::log_paths::bundled_dir(app).join("aivis").join(exe),
        crate::log_paths::data_dir(app).join("aivis").join(exe),
    ]
    .into_iter()
    .find_map(existing)
}

fn resource_candidates(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let exe = if cfg!(windows) { "run.exe" } else { "run" };
    let mut candidates = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/bundled/aivis").join(exe));
        candidates.push(res.join("bundled/aivis").join(exe));
        candidates.push(res.join("resources/aivis").join(exe));
        candidates.push(res.join("aivis").join(exe));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("resources/bundled/aivis").join(exe));
        candidates.push(cwd.join("src-tauri/resources/bundled/aivis").join(exe));
        candidates.push(cwd.join("resources/aivis").join(exe));
        candidates.push(cwd.join("src-tauri/resources/aivis").join(exe));
    }
    candidates
}

fn resource_engine_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    resource_candidates(app).into_iter().find_map(existing)
}

fn resource_is_newer(resource: &Path, installed: &Path) -> bool {
    let Ok(resource_time) = resource.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    let Ok(installed_time) = installed
        .metadata()
        .and_then(|metadata| metadata.modified())
    else {
        return true;
    };
    resource_time > installed_time
}

fn path_hint() -> &'static str {
    "Aivis の実行ファイルが見つかりません。ネットワーク接続を確認するか、cargo xtask setup aivis を実行してください。"
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    if permissions.mode() & 0o111 != 0 {
        return Ok(());
    }
    permissions.set_mode(permissions.mode() | 0o755);
    if let Err(error) = std::fs::set_permissions(path, permissions) {
        // フォールバック許可: .app 配下など実行時に chmod できない同梱リソースは spawn 側の成否に委ねる。
        if error.kind() != std::io::ErrorKind::PermissionDenied {
            return Err(error.into());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeviceMode {
    Auto,
    Cpu,
    Gpu,
}

enum StartupWait {
    Ready,
    Exited(ExitStatus),
    Failed(String),
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchDevice {
    Cpu,
    Gpu,
}

impl LaunchDevice {
    fn use_gpu(self) -> bool {
        matches!(self, LaunchDevice::Gpu)
    }

    fn label(self) -> &'static str {
        match self {
            LaunchDevice::Cpu => "cpu",
            LaunchDevice::Gpu => "gpu",
        }
    }

    fn flag(self) -> &'static str {
        if self.use_gpu() {
            "--use_gpu"
        } else {
            "--no-use_gpu"
        }
    }
}

fn device_mode(state: &AppState) -> DeviceMode {
    match state
        .config
        .get("PICOAGENT_TTS_DEVICE", DEFAULT_DEVICE)
        .to_lowercase()
        .as_str()
    {
        "gpu" => DeviceMode::Gpu,
        "cpu" => DeviceMode::Cpu,
        _ => DeviceMode::Auto,
    }
}

fn log_path(app: &tauri::AppHandle) -> PathBuf {
    crate::log_paths::migrate_legacy_app_log(app, "aivis.log");
    crate::log_paths::log_file(app, "aivis.log")
}

fn append_log(app: &tauri::AppHandle, message: &str) -> anyhow::Result<()> {
    let log_path = log_path(app);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    writeln!(log, "{message}")?;
    Ok(())
}

fn log_tail(app: &tauri::AppHandle) -> String {
    let path = log_path(app);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let tail = text
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    if tail.is_empty() {
        format!("ログ: {}", path.display())
    } else {
        format!("ログ: {}\n{}", path.display(), tail)
    }
}

fn install_dir(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    Ok(crate::log_paths::bundled_dir(app).join("aivis"))
}

fn legacy_install_dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut dirs = vec![crate::log_paths::data_dir(app).join("aivis")];
    if let Ok(path) = app.path().app_data_dir() {
        dirs.push(path.join("aivis"));
    }
    dirs
}

fn release_asset_pattern() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "AivisSpeech-Engine-Windows-x64-",
        ("macos", "aarch64") => "AivisSpeech-Engine-macOS-arm64-",
        ("macos", _) => "AivisSpeech-Engine-macOS-x64-",
        _ => "",
    }
}

async fn aivis_release_asset_url() -> anyhow::Result<(String, String, u64, String)> {
    let data: serde_json::Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("PicoAgent/1.0 (Aivis runtime installer)")
        .build()?
        .get(AIVIS_API_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let prefix = release_asset_pattern();
    if prefix.is_empty() {
        anyhow::bail!("このOS/CPU向けの Aivis 配布物を自動選択できません。");
    }
    let Some(asset) = data
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(|name| name.starts_with(prefix) && name.ends_with(".7z.001"))
                    .unwrap_or(false)
            })
        })
    else {
        anyhow::bail!("Aivis {AIVIS_RELEASE} の自動取得対象アセットが見つかりません。");
    };
    let name = asset
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Aivis アセット名を読めません。"))?
        .to_string();
    let url = asset
        .get("browser_download_url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Aivis ダウンロードURLを読めません。"))?
        .to_string();
    let size = asset
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .filter(|size| *size > 0 && *size <= 2 * 1024 * 1024 * 1024)
        .ok_or_else(|| anyhow::anyhow!("Aivis アセットサイズが不正です。"))?;
    let digest = asset
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64)
        .ok_or_else(|| anyhow::anyhow!("Aivis 公式ReleaseにSHA-256 digestがありません。"))?
        .to_ascii_lowercase();
    Ok((name, url, size, digest))
}

async fn download_aivis_archive(url: &str, dest: &Path, expected_size: u64) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("PicoAgent/1.0 (Aivis runtime installer)")
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut downloaded = 0_u64;
    while let Some(chunk) = response.chunk().await? {
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        anyhow::ensure!(
            downloaded <= expected_size,
            "Aivis の取得サイズが予定を超えています。"
        );
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    anyhow::ensure!(
        downloaded == expected_size,
        "Aivis の取得サイズが予定と一致しません。"
    );
    tokio::fs::rename(tmp, dest).await?;
    Ok(())
}

fn verify_archive_sha256(path: &Path, expected: &str) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = std::io::Read::read(&mut file, &mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    anyhow::ensure!(
        format!("{:x}", digest.finalize()) == expected,
        "Aivis 公式配布のSHA-256が一致しません。"
    );
    Ok(())
}

fn command_exists(command: &str) -> bool {
    let checker = if cfg!(windows) { "where.exe" } else { "sh" };
    let mut cmd = crate::tools::process::command_sync(checker);
    if cfg!(windows) {
        cmd.arg(command);
    } else {
        cmd.args(["-lc", &format!("command -v {command}")]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn extract_archive(archive: &Path, out_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)?;
    let mut command = if command_exists("7z") {
        let mut cmd = crate::tools::process::command_sync("7z");
        cmd.args([
            "x",
            archive.to_string_lossy().as_ref(),
            &format!("-o{}", out_dir.display()),
            "-y",
        ]);
        cmd
    } else if command_exists("7za") {
        let mut cmd = crate::tools::process::command_sync("7za");
        cmd.args([
            "x",
            archive.to_string_lossy().as_ref(),
            &format!("-o{}", out_dir.display()),
            "-y",
        ]);
        cmd
    } else {
        let mut cmd = crate::tools::process::command_sync("tar");
        cmd.args(["-xf", archive.to_string_lossy().as_ref(), "-C"]);
        cmd.arg(out_dir);
        cmd
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = command.status()?;
    if !status.success() {
        anyhow::bail!(
            "Aivis アーカイブの展開に失敗しました。7-Zip/7za または tar を利用できる環境が必要です。macOS で .7z.001 を展開する場合は p7zip を導入してください。"
        );
    }
    Ok(())
}

fn find_run_file(dir: &Path) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "run.exe" } else { "run" };
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_run_file(&path) {
                return Some(found);
            }
        } else if path.file_name().and_then(|name| name.to_str()) == Some(exe) {
            return Some(path);
        }
    }
    None
}

fn copy_dir_all(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn copy_resource_to_install(app: &tauri::AppHandle) -> anyhow::Result<Option<PathBuf>> {
    let Some(resource_run) = resource_engine_path(app) else {
        return Ok(None);
    };
    let resource_root = resource_run
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Aivis 同梱リソースの親ディレクトリを取得できません。"))?
        .to_path_buf();
    let install = install_dir(app)?;
    if install.exists() {
        std::fs::remove_dir_all(&install)?;
    }
    copy_dir_all(&resource_root, &install)?;
    let Some(path) = runtime_engine_path(app) else {
        anyhow::bail!("Aivis 同梱リソースをコピーしましたが、実行ファイルを見つけられません。");
    };
    ensure_executable(&path)?;
    append_log(
        app,
        &format!(
            "Aivis 同梱リソースを実行時ディレクトリへコピーしました: {} -> {}",
            resource_root.display(),
            install.display()
        ),
    )?;
    Ok(Some(path))
}

async fn ensure_installed(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    migrate_legacy_install(app)?;
    if let Some(path) = runtime_engine_path(app) {
        if let Some(resource) = resource_engine_path(app) {
            if resource_is_newer(&resource, &path) {
                if let Some(copied) = copy_resource_to_install(app)? {
                    return Ok(copied);
                }
            }
        }
        ensure_executable(&path)?;
        return Ok(path);
    }
    if let Some(path) = copy_resource_to_install(app)? {
        ensure_executable(&path)?;
        return Ok(path);
    }
    append_log(
        app,
        "Aivis が見つからないため、公式リリースから自動取得します。",
    )?;
    let (asset_name, asset_url, asset_size, asset_sha256) = aivis_release_asset_url().await?;
    let work = std::env::temp_dir().join("picoagent-aivis-runtime");
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work).await?;
    let _cleanup = AivisInstallCleanup(work.clone());
    let archive = work.join(&asset_name);
    append_log(app, &format!("Aivis ダウンロード開始: {asset_name}"))?;
    download_aivis_archive(&asset_url, &archive, asset_size).await?;
    let archive_for_verify = archive.clone();
    tokio::task::spawn_blocking(move || verify_archive_sha256(&archive_for_verify, &asset_sha256))
        .await??;
    append_log(app, "Aivis ダウンロード完了。展開します。")?;

    let extract_dir = work.join("extract");
    let archive_for_extract = archive.clone();
    let extract_for_extract = extract_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_archive(&archive_for_extract, &extract_for_extract)
    })
    .await??;
    let run_path = find_run_file(&extract_dir)
        .ok_or_else(|| anyhow::anyhow!("展開した Aivis に run 実行ファイルが見つかりません。"))?;
    let root = run_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Aivis 実行ファイルの親ディレクトリを取得できません。"))?
        .to_path_buf();
    let install = install_dir(app)?;
    if install.exists() {
        tokio::fs::remove_dir_all(&install).await?;
    }
    let root_for_copy = root.clone();
    let install_for_copy = install.clone();
    tokio::task::spawn_blocking(move || copy_dir_all(&root_for_copy, &install_for_copy)).await??;
    let installed = runtime_engine_path(app).ok_or_else(|| {
        anyhow::anyhow!("Aivis を配置しましたが、実行ファイルを見つけられません。")
    })?;
    ensure_executable(&installed)?;
    append_log(app, &format!("Aivis 自動取得完了: {}", installed.display()))?;
    Ok(installed)
}

/// 追加機能画面からAivis本体だけを公式Releaseから導入する。
/// エンジンの起動や読み上げ設定の変更は行わない。
pub async fn install_component(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    ensure_installed(app).await
}

fn migrate_legacy_install(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let new_dir = install_dir(app)?;
    let exe = if cfg!(windows) { "run.exe" } else { "run" };
    if new_dir.join(exe).exists() {
        return Ok(());
    }
    let Some(old_dir) = legacy_install_dirs(app)
        .into_iter()
        .find(|path| path.join(exe).exists())
    else {
        return Ok(());
    };
    append_log(
        app,
        &format!(
            "旧 Aivis 配置を PicoAgent bundled ディレクトリへ移行します: {} -> {}",
            old_dir.display(),
            new_dir.display()
        ),
    )?;
    if let Some(parent) = new_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(&old_dir, &new_dir) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_dir_all(&old_dir, &new_dir)?;
            let _ = std::fs::remove_dir_all(old_dir);
            Ok(())
        }
    }
}

fn child_exit_status(app: &tauri::AppHandle) -> Result<Option<ExitStatus>, String> {
    let Some(state) = app.try_state::<AivisState>() else {
        return Err("AivisState を取得できません".to_string());
    };
    let mut slot = match state.child.lock() {
        Ok(slot) => slot,
        // 終了確認と解放を優先し、poison 後も保持中のハンドルを回収する。
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(child) = slot.as_mut() else {
        return Ok(None);
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            *slot = None;
            Ok(Some(status))
        }
        Ok(None) => Ok(None),
        Err(error) => {
            if let Some(mut child) = slot.take() {
                terminate_child(&mut child);
            }
            Err(format!("Aivis の終了状態を確認できません: {error}"))
        }
    }
}

fn has_running_child(app: &tauri::AppHandle) -> bool {
    let Some(state) = app.try_state::<AivisState>() else {
        return false;
    };
    let mut slot = match state.child.lock() {
        Ok(slot) => slot,
        // 実プロセスの解放を優先し、poison 後も保持中のハンドルを確認する。
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(process) = slot.as_mut() {
        if matches!(process.try_wait(), Ok(None)) {
            return true;
        }
        if let Some(mut child) = slot.take() {
            terminate_child(&mut child);
        }
    }
    false
}

/// 接続されているディスプレイアダプタ名を列挙する。
///
/// 以前は PowerShell から `Get-CimInstance Win32_VideoController` を叩いていたが、
/// GPU の有無を見るだけのためにシェルと WMI を起動するのは重すぎる。
/// `EnumDisplayDevicesW` は COM も要らず、同じ用途に十分な情報（アダプタ名）を返す。
#[cfg(windows)]
fn gpu_adapter_names() -> Vec<String> {
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};

    let mut names = Vec::new();
    for index in 0..16u32 {
        let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // 安全性: cb を正しく埋めた自前のバッファだけを渡し、戻り値が 0 なら打ち切る。
        let ok = unsafe { EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) };
        if ok == 0 {
            break;
        }
        let end = device
            .DeviceString
            .iter()
            .position(|&ch| ch == 0)
            .unwrap_or(device.DeviceString.len());
        let name = String::from_utf16_lossy(&device.DeviceString[..end]);
        let name = name.trim().to_string();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(windows)]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    let adapters = gpu_adapter_names();
    if adapters.is_empty() {
        let _ = append_log(
            app,
            "GPU候補確認に失敗したため、auto は CPU 起動を選びます。",
        );
        return false;
    }
    let has_gpu = adapters.iter().any(|adapter| {
        let name = adapter.trim().to_lowercase();
        !name.is_empty()
            && !name.contains("microsoft basic")
            && !name.contains("basic render")
            && !name.contains("remote display")
            && (name.contains("nvidia")
                || name.contains("geforce")
                || name.contains("rtx")
                || name.contains("gtx")
                || name.contains("quadro")
                || name.contains("radeon")
                || name.contains("vega")
                || name.contains("graphics")
                || name.contains("intel")
                || name.contains("arc"))
    });
    let _ = append_log(
        app,
        if has_gpu {
            "GPU候補確認: GPU候補が見つかったため auto は GPU 起動を選びます。"
        } else {
            "GPU候補確認: GPU候補が見つからないため auto は CPU 起動を選びます。"
        },
    );
    has_gpu
}

#[cfg(target_os = "macos")]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    // macOS は Metal が常に使えるため、事前判定を行わず GPU 起動を選ぶ。
    let _ = append_log(
        app,
        "GPU候補確認: このOSでは事前判定を行わず auto は GPU 起動を選びます。",
    );
    true
}

#[cfg(not(any(windows, target_os = "macos")))]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    let _ = append_log(
        app,
        "GPU候補確認: このOSは対応対象外のため CPU 起動を選びます。",
    );
    false
}

fn auto_launch_device(app: &tauri::AppHandle) -> LaunchDevice {
    if gpu_candidate_available(app) {
        LaunchDevice::Gpu
    } else {
        LaunchDevice::Cpu
    }
}

fn spawn_engine(
    path: PathBuf,
    app: &tauri::AppHandle,
    device: LaunchDevice,
    label: &str,
) -> anyhow::Result<()> {
    if !is_executable_file(&path) {
        anyhow::bail!("Aivis の実行ファイルが見つかりません: {}", path.display());
    }
    ensure_executable(&path)?;
    let work_dir = path.parent().map(Path::to_path_buf);
    let mut command = crate::tools::process::command_sync(path);
    if let Some(dir) = work_dir {
        command.current_dir(dir);
    }
    let Some(state) = app.try_state::<AppState>() else {
        anyhow::bail!("AppState を取得できません。");
    };
    let port = port(app, &state);
    command.args(["--host", "127.0.0.1", "--port"]);
    command.arg(port.to_string());
    if device.use_gpu() {
        command.arg("--use_gpu");
    } else {
        command.arg("--no-use_gpu");
    }
    command.args(["--disable_sentry", "--output_log_utf8"]);
    let log_path = log_path(app);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(windows)]
    {
        let aivis_user_root = crate::log_paths::data_dir(app).join("aivis-userdata");
        std::fs::create_dir_all(&aivis_user_root)?;
        command.env("APPDATA", &aivis_user_root);
        command.env("LOCALAPPDATA", &aivis_user_root);
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    writeln!(
        log,
        "\n==== Aivis 起動: port={} device={} ({}) ====",
        port,
        label,
        device.flag()
    )?;
    let err_log = log.try_clone()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log));
    // macOS: 子を独立したプロセスグループリーダーにする。
    // stop() 時に pgid 全体へ SIGKILL を送り、子孫プロセスも確実に終了させるため。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let Some(state) = app.try_state::<AivisState>() else {
        terminate_child(&mut child);
        anyhow::bail!("AivisState を取得できないため、起動した Aivis を停止しました");
    };
    let mut slot = match state.child.lock() {
        Ok(slot) => slot,
        // ハンドルを失わないよう、poison 後も格納済み状態を引き継ぐ。
        Err(poisoned) => poisoned.into_inner(),
    };
    if slot.is_some() {
        terminate_child(&mut child);
        anyhow::bail!("Aivis は既に起動中です");
    }
    *slot = Some(child);
    Ok(())
}

pub fn start_background(app: &tauri::AppHandle) -> anyhow::Result<()> {
    if has_running_child(app) {
        return Ok(());
    }
    migrate_legacy_install(app)?;
    let path = match runtime_engine_path(app) {
        Some(path) => {
            if let Some(resource) = resource_engine_path(app) {
                if resource_is_newer(&resource, &path) {
                    // フォールバック許可: 同梱リソース更新に失敗しても既存の実行時配置が使える場合は起動を優先する。
                    copy_resource_to_install(app)?.unwrap_or(path)
                } else {
                    path
                }
            } else {
                path
            }
        }
        None => copy_resource_to_install(app)?.ok_or_else(|| anyhow::anyhow!(path_hint()))?,
    };
    let mode = app
        .try_state::<AppState>()
        .map(|state| device_mode(&state))
        .unwrap_or(DeviceMode::Auto);
    let device = match mode {
        DeviceMode::Cpu => LaunchDevice::Cpu,
        DeviceMode::Gpu => LaunchDevice::Gpu,
        DeviceMode::Auto => auto_launch_device(app),
    };
    let label = match mode {
        DeviceMode::Auto => {
            if device == LaunchDevice::Gpu {
                "auto/gpu"
            } else {
                "auto/cpu"
            }
        }
        _ => device.label(),
    };
    spawn_engine(path, app, device, label)
}

// 同梱 Aivis が応答可能（/version が 200）かを返す。接続不可は false に正規化する。
pub async fn is_ready(app: &tauri::AppHandle, state: &AppState) -> bool {
    is_up(&endpoint(app, state)).await
}

pub async fn ensure_started(app: &tauri::AppHandle) -> anyhow::Result<()> {
    // 終了処理が始まっていたら spawn しない（インストール完了後のレースコンディション対策）。
    if app
        .try_state::<AivisState>()
        .map(|s| s.stopping.load(Ordering::SeqCst))
        .unwrap_or(true)
    {
        return Ok(());
    }
    let app_state = app.state::<AppState>();
    let endpoint = endpoint(app, &app_state);
    if is_up(&endpoint).await {
        return Ok(());
    }
    let mode = device_mode(&app_state);
    let path = ensure_installed(app).await?;

    if has_running_child(app) {
        match wait_until_ready(app, &endpoint).await {
            StartupWait::Ready => return Ok(()),
            StartupWait::Exited(status) if mode != DeviceMode::Auto => {
                anyhow::bail!(
                    "AivisSpeech Engine が起動直後に終了しました: {}\n{}",
                    status,
                    log_tail(app)
                );
            }
            StartupWait::Exited(_) | StartupWait::Timeout => {
                stop(app);
            }
            StartupWait::Failed(error) => {
                stop(app);
                anyhow::bail!("{error}\n{}", log_tail(app));
            }
        }
    }

    match mode {
        DeviceMode::Cpu => {
            spawn_engine(path, app, LaunchDevice::Cpu, "cpu")?;
            wait_ready_or_bail(app, &endpoint, "AivisSpeech Engine のCPU起動").await
        }
        DeviceMode::Gpu => {
            spawn_engine(path, app, LaunchDevice::Gpu, "gpu")?;
            wait_ready_or_bail(app, &endpoint, "AivisSpeech Engine のGPU起動").await
        }
        DeviceMode::Auto => {
            let device = auto_launch_device(app);
            if device == LaunchDevice::Gpu {
                spawn_engine(path, app, LaunchDevice::Gpu, "auto/gpu")?;
                wait_ready_or_bail(app, &endpoint, "AivisSpeech Engine のGPU起動").await
            } else {
                spawn_engine(path, app, LaunchDevice::Cpu, "auto/cpu")?;
                wait_ready_or_bail(app, &endpoint, "AivisSpeech Engine のCPU起動").await
            }
        }
    }
}

async fn wait_until_ready(app: &tauri::AppHandle, endpoint: &str) -> StartupWait {
    for _ in 0..STARTUP_CHECKS {
        if is_up(endpoint).await {
            return StartupWait::Ready;
        }
        match child_exit_status(app) {
            Ok(Some(status)) => return StartupWait::Exited(status),
            Ok(None) => {}
            Err(error) => return StartupWait::Failed(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    StartupWait::Timeout
}

async fn wait_ready_or_bail(
    app: &tauri::AppHandle,
    endpoint: &str,
    label: &str,
) -> anyhow::Result<()> {
    match wait_until_ready(app, endpoint).await {
        StartupWait::Ready => Ok(()),
        StartupWait::Exited(status) => anyhow::bail!(
            "{label}が起動直後に終了しました: {}\n{}",
            status,
            log_tail(app)
        ),
        StartupWait::Failed(error) => {
            stop(app);
            anyhow::bail!("{label}の監視に失敗しました: {error}\n{}", log_tail(app))
        }
        StartupWait::Timeout => {
            stop(app);
            anyhow::bail!("{label}がタイムアウトしました\n{}", log_tail(app))
        }
    }
}

/// Aivis 子プロセスを停止する（アプリ終了時）。
/// `stopping` フラグを先に立てて `ensure_started` の後続 spawn を防ぐ。
/// Windows: `taskkill /T /F` でプロセスツリーごと終了させる。
/// macOS: `process_group(0)` で生成したプロセスグループに SIGKILL を送る。
pub fn stop(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AivisState>() {
        state.stopping.store(true, Ordering::SeqCst);
        terminate_managed_child(app);
    }
}

fn terminate_child(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = crate::tools::process::command_sync("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // spawn_engine で process_group(0) を設定しているため、子の PID = PGID。
        let pgid = child.id() as libc::pid_t;
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
    // プロセスグループの停止に失敗しても、直接の子プロセスは必ず停止を試みる。
    let _ = child.kill();
    let _ = child.wait();
}

fn terminate_managed_child(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AivisState>() else {
        return;
    };
    let child = match state.child.lock() {
        Ok(mut slot) => slot.take(),
        // 停止処理は Mutex の poison 後も中のハンドルを回収する。
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(mut child) = child {
        terminate_child(&mut child);
    }
}
