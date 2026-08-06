// engine.rs - 同梱推論エンジン（llama-server）とモデル GGUF のライフサイクル。
//
// 既定プロバイダ "local" のために、初回はモデルをダウンロードし、llama-server を spawn して
// OpenAI 互換 API をローカルに立てる。進捗・状態は "engine:status" イベントで UI へ通知する。

use std::io::Write;
use std::path::{Path, PathBuf};
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

/// ダウンロード済みモデルの保存ディレクトリ（`app_data_dir/bundled/models`）。
pub fn models_dir(app: &AppHandle) -> PathBuf {
    crate::log_paths::bundled_dir(app).join("models")
}

/// 現在の設定で使用するモデルの GGUF ファイル名（一覧で「使用中」を判定する用途）。
pub fn active_model_filename(app: &AppHandle) -> Option<String> {
    model_path(app)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

async fn running_server_model_ids(app: &AppHandle) -> anyhow::Result<Vec<String>> {
    let state = app.state::<AppState>();
    let port = local::port(&state.config);
    let url = format!("http://127.0.0.1:{port}/v1/models");
    let data: Value = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(data
        .get("data")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
        .map(|id| id.replace('\\', "/"))
        .collect())
}

async fn running_server_uses_model(app: &AppHandle, model: &Path) -> bool {
    let Some(file_name) = model.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let expected_path = model.to_string_lossy().replace('\\', "/");
    match running_server_model_ids(app).await {
        Ok(ids) => ids.iter().any(|id| {
            id == file_name || id == &expected_path || id.ends_with(&format!("/{file_name}"))
        }),
        Err(error) => {
            eprintln!("既存ローカルエンジンのモデル確認に失敗しました: {error}");
            false
        }
    }
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
        // WMI を PowerShell 経由で叩くのをやめ、OS の API を直接呼ぶ。
        // 起動が要らないぶん速く、実行ポリシーやプロファイルの影響も受けない。
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // 安全性: dwLength を正しく埋めた自前のバッファだけを渡す。
        if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
            return None;
        }
        Some(status.ullTotalPhys)
    }
    #[cfg(target_os = "macos")]
    {
        // sysctl の hw.memsize が搭載メモリ量（バイト）を返す。
        let mut size: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let name = c"hw.memsize";
        // 安全性: 出力先は自前の u64、長さもその size を渡す。失敗時は非ゼロが返る。
        let ok = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                &mut size as *mut u64 as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if ok != 0 || size == 0 {
            return None;
        }
        Some(size)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// GPU の VRAM 量（バイト）。ローカルモデルのプリセット自動選定にだけ使う。
///
/// ここだけ PowerShell + WMI が残っている。正確な値を取るには DXGI の
/// `IDXGIAdapter1::GetDesc1`（`DedicatedVideoMemory`）を読むのが本筋だが、
/// COM の呼び出しが必要で `windows-sys` では素の vtable を叩くことになり、
/// 高レベルな `windows` クレートを足すと依存が大きく増える。初回起動時に一度だけ
/// 動く経路なので、費用対効果を見て据え置いている（総メモリ量と GPU 名の取得は
/// ネイティブ API へ置き換え済み）。
fn gpu_vram_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        let output = crate::tools::process::command_sync("powershell")
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
            "同梱エンジン llama-server が見つかりません（cargo xtask setup engine を実行してください）"
        )
    })?;
    ensure_executable(&server)?;
    let state = app.state::<AppState>();
    let port = local::port(&state.config).to_string();
    let ctx = state.config.get("PICOAGENT_LOCAL_CTX", "8192");
    // GPU 優先: 既定で全レイヤを GPU へオフロード（999）。GPU 非対応ビルド/環境では
    // llama-server が自動で CPU にフォールバックする。CPU 強制は 0 を設定する。
    let ngl = state.config.get("PICOAGENT_LOCAL_NGL", "999");
    // 使用 GPU の指定（複数 GPU 環境向け）。空なら llama-server の既定（先頭デバイス）。
    // 例: Vulkan ビルドで discrete GPU を使う場合は "Vulkan1" のように指定する。
    let gpu_device = state.config.get("PICOAGENT_LOCAL_GPU_DEVICE", "");

    let mut cmd = crate::tools::process::command_sync(&server);
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
    if app
        .try_state::<EngineState>()
        .map(|e| e.stopping.load(Ordering::SeqCst))
        .unwrap_or(true)
    {
        return;
    }
    // すでに同梱エンジンが応答していれば（前回クラッシュ等で取り残されたプロセス）、
    // 要求モデルまで一致する場合だけ再利用する。モデル変更後の古いプロセスを誤用しないため。
    if let Some(provider) = crate::providers::get("local") {
        let state = app.state::<AppState>();
        if provider.is_up(&state.config).await {
            if running_server_uses_model(&app, &model).await {
                set_status(&app, json!({ "phase": "ready" }));
                return;
            }
            set_status(
                &app,
                json!({
                    "phase": "error",
                    "message": "別モデルのローカルエンジンが既に起動しています。PicoAgent を終了して残った llama-server を停止してから再起動してください"
                }),
            );
            return;
        }
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
            // Windows は taskkill /T で llama-server の子孫まで含めて確実に停止する（Aivis と同方針）。
            #[cfg(windows)]
            {
                let _ = crate::tools::process::command_sync("taskkill")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
            #[cfg(not(windows))]
            let _ = child.kill();
            let _ = child.wait(); // kill 後に wait してプロセスハンドルを回収する。
        }
    }
}

/// 設定の再初期化でエンジンを停止→再起動するときに使う。
/// `stop()` が立てた `stopping` フラグを下ろし、続く `ensure_and_start` が起動できるようにする。
/// （`stop()` はアプリ終了用にフラグを立てっぱなしにするため、再起動経路では明示的に下ろす必要がある。）
pub fn clear_stopping(app: &AppHandle) {
    if let Some(engine) = app.try_state::<EngineState>() {
        engine.stopping.store(false, Ordering::SeqCst);
    }
}
