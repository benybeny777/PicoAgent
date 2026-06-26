// aivis.rs - 同梱 AivisSpeech Engine の接続確認と裏起動を管理する。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::Manager;
use tokio::io::AsyncWriteExt;

use crate::commands::AppState;

const BUNDLED_PORT: u16 = 10102;
const AIVIS_RELEASE: &str = "1.2.0";
const AIVIS_API_URL: &str =
    "https://api.github.com/repos/Aivis-Project/AivisSpeech-Engine/releases/tags/1.2.0";
const DEFAULT_DEVICE: &str = "auto";
const STARTUP_CHECKS: usize = 360;
const GPU_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub struct AivisState {
    child: Mutex<Option<Child>>,
}

impl AivisState {
    pub fn new() -> Self {
        AivisState {
            child: Mutex::new(None),
        }
    }
}

impl Default for AivisState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn endpoint(_app: &tauri::AppHandle, _state: &AppState) -> String {
    format!("http://127.0.0.1:{BUNDLED_PORT}")
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

fn existing(path: PathBuf) -> Option<PathBuf> {
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn bundled_candidates(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let exe = if cfg!(windows) { "run.exe" } else { "run" };
    let mut candidates = Vec::new();
    if let Ok(data_dir) = app.path().app_data_dir() {
        candidates.push(data_dir.join("aivis").join(exe));
    }
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/aivis").join(exe));
        candidates.push(res.join("aivis").join(exe));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("resources/aivis").join(exe));
        candidates.push(cwd.join("src-tauri/resources/aivis").join(exe));
    }
    candidates
}

fn bundled_engine_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    bundled_candidates(app).into_iter().find_map(existing)
}

fn path_hint() -> &'static str {
    "Aivis の実行ファイルが見つかりません。ネットワーク接続を確認するか、npm run setup:aivis を実行してください。"
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
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
    Ok(app.path().app_data_dir()?.join("aivis"))
}

fn release_asset_pattern() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "AivisSpeech-Engine-Windows-x64-",
        ("macos", "aarch64") => "AivisSpeech-Engine-macOS-arm64-",
        ("macos", _) => "AivisSpeech-Engine-macOS-x64-",
        ("linux", "aarch64") => "AivisSpeech-Engine-Linux-arm64-",
        ("linux", _) => "AivisSpeech-Engine-Linux-x64-",
        _ => "",
    }
}

async fn aivis_release_asset_url() -> anyhow::Result<(String, String)> {
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
    Ok((name, url))
}

async fn download_aivis_archive(url: &str, dest: &Path) -> anyhow::Result<()> {
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
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(tmp, dest).await?;
    Ok(())
}

fn command_exists(command: &str) -> bool {
    let checker = if cfg!(windows) { "where.exe" } else { "sh" };
    let mut cmd = std::process::Command::new(checker);
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
        let mut cmd = std::process::Command::new("7z");
        cmd.args([
            "x",
            archive.to_string_lossy().as_ref(),
            &format!("-o{}", out_dir.display()),
            "-y",
        ]);
        cmd
    } else if command_exists("7za") {
        let mut cmd = std::process::Command::new("7za");
        cmd.args([
            "x",
            archive.to_string_lossy().as_ref(),
            &format!("-o{}", out_dir.display()),
            "-y",
        ]);
        cmd
    } else {
        let mut cmd = std::process::Command::new("tar");
        cmd.args(["-xf", archive.to_string_lossy().as_ref(), "-C"]);
        cmd.arg(out_dir);
        cmd
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let status = command.status()?;
    if !status.success() {
        anyhow::bail!(
            "Aivis アーカイブの展開に失敗しました。7-Zip または tar を利用できる環境が必要です。"
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

async fn ensure_installed(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    if let Some(path) = bundled_engine_path(app) {
        return Ok(path);
    }
    append_log(
        app,
        "Aivis が見つからないため、公式リリースから自動取得します。",
    )?;
    let (asset_name, asset_url) = aivis_release_asset_url().await?;
    let work = std::env::temp_dir().join("picoagent-aivis-runtime");
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work).await?;
    let archive = work.join(&asset_name);
    append_log(app, &format!("Aivis ダウンロード開始: {asset_name}"))?;
    download_aivis_archive(&asset_url, &archive).await?;
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
    let _ = tokio::fs::remove_dir_all(&work).await;
    let installed = bundled_engine_path(app).ok_or_else(|| {
        anyhow::anyhow!("Aivis を配置しましたが、実行ファイルを見つけられません。")
    })?;
    append_log(app, &format!("Aivis 自動取得完了: {}", installed.display()))?;
    Ok(installed)
}

fn child_exit_status(app: &tauri::AppHandle) -> Option<ExitStatus> {
    let state = app.try_state::<AivisState>()?;
    let mut child = state.child.lock().unwrap();
    match child
        .as_mut()
        .and_then(|child| child.try_wait().ok())
        .flatten()
    {
        Some(status) => {
            *child = None;
            Some(status)
        }
        None => None,
    }
}

fn has_running_child(app: &tauri::AppHandle) -> bool {
    let Some(state) = app.try_state::<AivisState>() else {
        return false;
    };
    let mut child = state.child.lock().unwrap();
    if let Some(process) = child.as_mut() {
        if matches!(process.try_wait(), Ok(None)) {
            return true;
        }
        *child = None;
    }
    false
}

fn wait_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if started.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn gpu_probe_output() -> std::io::Result<String> {
    let script = "Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name }";
    let mut command = std::process::Command::new("powershell");
    command.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command.stdout(Stdio::piped()).spawn()?;
    match wait_child_with_timeout(&mut child, GPU_PROBE_TIMEOUT)? {
        Some(status) if status.success() => {
            let mut output = Vec::new();
            if let Some(mut stdout) = child.stdout.take() {
                stdout.read_to_end(&mut output)?;
            }
            let _ = child.wait();
            Ok(String::from_utf8_lossy(&output).to_string())
        }
        Some(_) => Ok(String::new()),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Ok(String::new())
        }
    }
}

#[cfg(windows)]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    let Ok(output) = gpu_probe_output() else {
        let _ = append_log(
            app,
            "GPU候補確認に失敗したため、auto は CPU 起動を選びます。",
        );
        return false;
    };
    let has_gpu = output.lines().any(|line| {
        let name = line.trim().to_lowercase();
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

#[cfg(target_os = "linux")]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    let has_gpu = std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::fs::read_dir("/dev/dri")
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
            })
            .unwrap_or(false);
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

#[cfg(all(not(windows), not(target_os = "linux")))]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    let _ = append_log(
        app,
        "GPU候補確認: このOSでは事前判定を行わず auto は GPU 起動を選びます。",
    );
    true
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
    let work_dir = path.parent().map(Path::to_path_buf);
    let mut command = std::process::Command::new(path);
    if let Some(dir) = work_dir {
        command.current_dir(dir);
    }
    command.args(["--host", "127.0.0.1", "--port"]);
    command.arg(BUNDLED_PORT.to_string());
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
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    writeln!(
        log,
        "\n==== Aivis 起動: device={} ({}) ====",
        label,
        device.flag()
    )?;
    let err_log = log.try_clone()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let child = command.spawn()?;
    if let Some(state) = app.try_state::<AivisState>() {
        *state.child.lock().unwrap() = Some(child);
    }
    Ok(())
}

pub fn start_background(app: &tauri::AppHandle) -> anyhow::Result<()> {
    if has_running_child(app) {
        return Ok(());
    }
    let path = bundled_engine_path(app).ok_or_else(|| anyhow::anyhow!(path_hint()))?;
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
        if is_up(&endpoint).await {
            return StartupWait::Ready;
        }
        if let Some(status) = child_exit_status(app) {
            return StartupWait::Exited(status);
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
        StartupWait::Timeout => {
            stop(app);
            anyhow::bail!("{label}がタイムアウトしました\n{}", log_tail(app))
        }
    }
}

pub fn stop(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AivisState>() {
        if let Some(mut child) = state.child.lock().unwrap().take() {
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            #[cfg(not(windows))]
            {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}
