// aivis.rs - 同梱 AivisSpeech Engine の接続確認と裏起動を管理する。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::Mutex;

use tauri::Manager;

use crate::commands::AppState;

const BUNDLED_PORT: u16 = 10102;
const DEFAULT_DEVICE: &str = "auto";

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
    "同梱 Aivis の実行ファイルが見つかりません。npm run setup:aivis を実行してください。"
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn use_gpu(state: &AppState) -> bool {
    match state
        .config
        .get("PICOAGENT_TTS_DEVICE", DEFAULT_DEVICE)
        .to_lowercase()
        .as_str()
    {
        "gpu" => true,
        "cpu" => false,
        _ => true,
    }
}

fn log_path(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_log_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("aivis.log")
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

fn spawn_engine(path: PathBuf, app: &tauri::AppHandle) -> anyhow::Result<()> {
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
    if let Some(app_state) = app.try_state::<AppState>() {
        if use_gpu(&app_state) {
            command.arg("--use_gpu");
        } else {
            command.arg("--no-use_gpu");
        }
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
        "\n==== Aivis 起動: device={} ====",
        if let Some(app_state) = app.try_state::<AppState>() {
            app_state.config.get("PICOAGENT_TTS_DEVICE", DEFAULT_DEVICE)
        } else {
            DEFAULT_DEVICE.to_string()
        }
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
    spawn_engine(path, app)
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
    stop(app);
    start_background(app)?;
    for _ in 0..360 {
        if is_up(&endpoint).await {
            return Ok(());
        }
        if let Some(status) = child_exit_status(app) {
            anyhow::bail!(
                "AivisSpeech Engine が起動直後に終了しました: {}\n{}",
                status,
                log_tail(app)
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    stop(app);
    anyhow::bail!(
        "AivisSpeech Engine の起動がタイムアウトしました\n{}",
        log_tail(app)
    )
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
