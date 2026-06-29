// engine.rs - 同梱推論エンジン（llama-server）とモデル GGUF のライフサイクル。
//
// 既定プロバイダ "local" のために、初回はモデルをダウンロードし、llama-server を spawn して
// OpenAI 互換 API をローカルに立てる。進捗・状態は "engine:status" イベントで UI へ通知する。

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::Map;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::AppState;
use crate::providers::local;

pub struct EngineState {
    pub status: Mutex<Value>,
    pub child: Mutex<Option<Child>>,
    /// `stop()` が呼ばれたら true に設定し、`ensure_and_start` がその後に spawn しないようにする。
    pub stopping: AtomicBool,
}

impl EngineState {
    pub fn new() -> Self {
        EngineState {
            status: Mutex::new(json!({ "phase": "idle" })),
            child: Mutex::new(None),
            stopping: AtomicBool::new(false),
        }
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}

fn set_status(app: &AppHandle, value: Value) {
    if let Some(engine) = app.try_state::<EngineState>() {
        *engine.status.lock().unwrap() = value.clone();
    }
    let _ = app.emit("engine:status", value);
}

// 同梱した llama-server 実行ファイルを解決する（バンドル時と dev で場所が異なるため候補を探す）。
fn server_path(app: &AppHandle) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(crate::log_paths::bundled_dir(app).join("engine").join(exe));
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("resources/bundled/engine").join(exe));
        candidates.push(res.join("bundled/engine").join(exe));
        candidates.push(res.join("resources/engine").join(exe));
        candidates.push(res.join("engine").join(exe));
    }
    // dev（tauri dev / cargo run）のフォールバック: 作業ディレクトリ直下。
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("resources/bundled/engine").join(exe));
        candidates.push(cwd.join("src-tauri/resources/bundled/engine").join(exe));
        candidates.push(cwd.join("resources/engine").join(exe));
        candidates.push(cwd.join("src-tauri/resources/engine").join(exe));
    }
    candidates.into_iter().find(|p| p.exists())
}

fn model_url(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    local::model_url(&state.config)
}

fn model_path(app: &AppHandle) -> PathBuf {
    let url = model_url(app);
    let name = url
        .split('/')
        .next_back()
        .filter(|s| !s.is_empty())
        .unwrap_or("model.gguf");
    crate::log_paths::bundled_dir(app).join("models").join(name)
}

fn legacy_model_paths(app: &AppHandle) -> Vec<PathBuf> {
    let url = model_url(app);
    let name = url
        .split('/')
        .next_back()
        .filter(|s| !s.is_empty())
        .unwrap_or("model.gguf");
    let mut paths = vec![crate::log_paths::data_dir(app).join("models").join(name)];
    if let Ok(dir) = app.path().app_data_dir() {
        paths.push(dir.join("models").join(name));
    }
    paths
}

async fn migrate_legacy_model(app: &AppHandle, dest: &PathBuf) -> anyhow::Result<()> {
    if dest.exists() {
        return Ok(());
    }
    let Some(old_path) = legacy_model_paths(app)
        .into_iter()
        .find(|path| path.exists())
    else {
        return Ok(());
    };
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match tokio::fs::rename(&old_path, dest).await {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            tokio::fs::copy(&old_path, dest)
                .await
                .map_err(|copy_error| {
                    anyhow::anyhow!(
                        "旧モデルの移行に失敗: rename={rename_error}; copy={copy_error}"
                    )
                })?;
            let _ = tokio::fs::remove_file(&old_path).await;
            Ok(())
        }
    }
}

// モデルをストリーミングダウンロード（進捗を emit）。
async fn download_model(app: &AppHandle, dest: &PathBuf) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let url = model_url(app);
    set_status(app, json!({ "phase": "download", "percent": 0 }));

    let mut res = reqwest::Client::new().get(&url).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("モデル取得に失敗: {} {}", res.status(), url);
    }
    let total = res.content_length().unwrap_or(0);
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut received: u64 = 0;
    let mut last_emit: i64 = -1;

    while let Some(chunk) = res.chunk().await? {
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        if let Some(pct) = received.saturating_mul(100).checked_div(total) {
            let pct = pct as i64;
            if pct != last_emit {
                last_emit = pct;
                set_status(app, json!({ "phase": "download", "percent": pct }));
            }
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

fn total_memory_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb = text.lines().find_map(|line| {
            line.strip_prefix("MemTotal:")
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok())
        })?;
        return Some(kb.saturating_mul(1024));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

fn gpu_vram_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty AdapterRAM",
            ])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u64>().ok())
            .max()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn auto_model_preset() -> &'static str {
    let vram = gpu_vram_bytes().unwrap_or(0);
    let ram = total_memory_bytes().unwrap_or(0);
    let gib = 1024_u64.pow(3);
    if vram >= 8 * gib && ram >= 24 * gib {
        "large"
    } else if vram >= 4 * gib || ram >= 16 * gib {
        "balanced"
    } else {
        "small"
    }
}

fn ensure_local_model_selection(app: &AppHandle) {
    let state = app.state::<AppState>();
    let requested = state.config.get("PICOAGENT_LOCAL_MODEL_PRESET", "auto");
    let should_auto = requested == "auto"
        || (!state.config.has_value("PICOAGENT_LOCAL_MODEL_PRESET")
            && !state.config.has_value("PICOAGENT_MODEL_URL"));
    let selected = if should_auto {
        auto_model_preset()
    } else if local::preset(&requested).is_some() || requested == "custom" {
        requested.as_str()
    } else {
        "balanced"
    };
    if selected == "custom" {
        return;
    }
    let Some(preset) = local::preset(selected) else {
        return;
    };
    let mut patch = Map::new();
    if requested != selected {
        patch.insert(
            "PICOAGENT_LOCAL_MODEL_PRESET".to_string(),
            Value::String(selected.to_string()),
        );
    }
    if !state.config.has_value("PICOAGENT_MODEL") {
        patch.insert(
            "PICOAGENT_MODEL".to_string(),
            Value::String(preset.id.to_string()),
        );
    }
    if !state.config.has_value("PICOAGENT_LOCAL_CTX") {
        patch.insert(
            "PICOAGENT_LOCAL_CTX".to_string(),
            Value::String(preset.ctx.to_string()),
        );
    }
    if !patch.is_empty() {
        let _ = state.config.set_all(patch);
        *state.model.lock().unwrap() = preset.id.to_string();
    }
}

// llama-server を起動する。
fn spawn_server(app: &AppHandle, model: &PathBuf) -> anyhow::Result<()> {
    let server = server_path(app).ok_or_else(|| {
        anyhow::anyhow!(
            "同梱エンジン llama-server が見つかりません（npm run setup:engine を実行してください）"
        )
    })?;
    let state = app.state::<AppState>();
    let port = local::port(&state.config).to_string();
    let ctx = state.config.get("PICOAGENT_LOCAL_CTX", "8192");
    // GPU 優先: 既定で全レイヤを GPU へオフロード（999）。GPU 非対応ビルド/環境では
    // llama-server が自動で CPU にフォールバックする。CPU 強制は 0 を設定する。
    let ngl = state.config.get("PICOAGENT_LOCAL_NGL", "999");
    // 使用 GPU の指定（複数 GPU 環境向け）。空なら llama-server の既定（先頭デバイス）。
    // 例: Vulkan ビルドで discrete GPU を使う場合は "Vulkan1" のように指定する。
    let gpu_device = state.config.get("PICOAGENT_LOCAL_GPU_DEVICE", "");

    let mut cmd = std::process::Command::new(&server);
    cmd.arg("-m")
        .arg(model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(&port)
        .arg("-c")
        .arg(&ctx)
        .arg("-ngl")
        .arg(&ngl)
        .arg("--jinja");
    if !gpu_device.trim().is_empty() {
        cmd.arg("--device").arg(gpu_device.trim());
    }
    let log_path = crate::log_paths::log_file(app, "engine.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    writeln!(
        log,
        "\n==== llama-server 起動: model={} port={} ctx={} ngl={} device={} ====",
        model.display(),
        port,
        ctx,
        ngl,
        if gpu_device.trim().is_empty() {
            "(default)"
        } else {
            gpu_device.trim()
        }
    )?;
    let err_log = log.try_clone()?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let child = cmd.spawn()?;
    if let Some(engine) = app.try_state::<EngineState>() {
        *engine.child.lock().unwrap() = Some(child);
    }
    Ok(())
}

// モデル取得→エンジン起動→ready 検知までを通しで行う（async タスクから呼ぶ）。
pub async fn ensure_and_start(app: AppHandle) {
    ensure_local_model_selection(&app);
    let model = model_path(&app);
    if let Err(e) = migrate_legacy_model(&app, &model).await {
        set_status(&app, json!({ "phase": "error", "message": e.to_string() }));
        return;
    }
    if !model.exists() {
        if let Err(e) = download_model(&app, &model).await {
            set_status(&app, json!({ "phase": "error", "message": e.to_string() }));
            return;
        }
    }
    // 終了処理が始まっていたら spawn しない（ダウンロード完了後のレースコンディション対策）。
    if app.try_state::<EngineState>().map(|e| e.stopping.load(Ordering::SeqCst)).unwrap_or(true) {
        return;
    }
    set_status(&app, json!({ "phase": "starting" }));
    if let Err(e) = spawn_server(&app, &model) {
        set_status(&app, json!({ "phase": "error", "message": e.to_string() }));
        return;
    }

    // /health が 200 を返すまで待ち、ready を通知する（最大 ~5 分）。
    let state = app.state::<AppState>();
    let provider = match crate::providers::get("local") {
        Some(p) => p,
        None => return,
    };
    for _ in 0..300 {
        if provider.is_up(&state.config).await {
            set_status(&app, json!({ "phase": "ready" }));
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    set_status(
        &app,
        json!({ "phase": "error", "message": "エンジンの起動がタイムアウトしました" }),
    );
}

/// 子プロセスを停止する（アプリ終了時）。
/// `stopping` フラグを先に立てて `ensure_and_start` の後続 spawn を防ぐ。
pub fn stop(app: &AppHandle) {
    if let Some(engine) = app.try_state::<EngineState>() {
        engine.stopping.store(true, Ordering::SeqCst);
        if let Some(mut child) = engine.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait(); // kill 後に wait してプロセスハンドルを回収する。
        }
    }
}
