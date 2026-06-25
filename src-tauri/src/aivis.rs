// aivis.rs - AivisSpeech Engine の接続確認と任意の裏起動を管理する。

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Mutex;

use tauri::Manager;

use crate::commands::AppState;

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

pub fn endpoint(state: &AppState) -> String {
    state
        .config
        .get("CODA_TTS_ENDPOINT", "http://127.0.0.1:10101")
        .trim_end_matches('/')
        .to_string()
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

fn configured_path(state: &AppState) -> Option<PathBuf> {
    let raw = state.config.get("CODA_TTS_ENGINE_PATH", "");
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
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
    #[cfg(windows)]
    {
        candidates.push(PathBuf::from(
            r"C:\Program Files\AivisSpeech\AivisSpeech-Engine\run.exe",
        ));
        candidates.push(PathBuf::from(
            r"C:\Program Files\AivisSpeech-Engine\run.exe",
        ));
    }
    candidates
}

fn resolve_engine_path(state: &AppState, app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Some(path) = configured_path(state) {
        return existing(path);
    }
    bundled_candidates(app).into_iter().find_map(existing)
}

fn path_hint() -> &'static str {
    "Aivis の実行ファイルが見つかりません。設定で CODA_TTS_ENGINE_PATH を指定するか、src-tauri/resources/aivis/ に配置してください。"
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn spawn_engine(path: PathBuf, app: &tauri::AppHandle) -> anyhow::Result<()> {
    if !is_executable_file(&path) {
        anyhow::bail!("Aivis の実行ファイルが見つかりません: {}", path.display());
    }
    let mut command = std::process::Command::new(path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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

pub async fn ensure_started(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let app_state = app.state::<AppState>();
    let endpoint = endpoint(&app_state);
    if is_up(&endpoint).await {
        return Ok(());
    }
    if !app_state.config.get_bool("CODA_TTS_AUTOSTART", false) {
        anyhow::bail!("AivisSpeech Engine が起動していません");
    }
    stop(app);
    let path = resolve_engine_path(&app_state, app).ok_or_else(|| anyhow::anyhow!(path_hint()))?;
    spawn_engine(path, app)?;
    for _ in 0..30 {
        if is_up(&endpoint).await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    stop(app);
    anyhow::bail!("AivisSpeech Engine の起動がタイムアウトしました")
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
