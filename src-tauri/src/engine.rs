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

static VISION_READY: AtomicBool = AtomicBool::new(false);

/// 画像を扱う全経路で、管理エンジンの起動完了を確かめる。
pub fn ensure_vision_available(cfg: &crate::config::Config) -> anyhow::Result<()> {
    anyhow::ensure!(
        local::vision_enabled(cfg),
        "ローカル画像入力が無効です。設定で有効にしてローカルAIを再起動してください"
    );
    local::validate_vision_model(cfg)?;
    anyhow::ensure!(VISION_READY.load(Ordering::SeqCst), "画像入力用エンジンの起動が完了していません。追加資産を導入してローカルAIを再起動してください");
    Ok(())
}

async fn vision_projector(app: &AppHandle) -> anyhow::Result<Option<PathBuf>> {
    let state = app.state::<AppState>();
    if !local::vision_enabled(&state.config) {
        return Ok(None);
    }
    local::validate_vision_model(&state.config)?;
    let name = local::vision_asset::FILE_NAME;
    // 追加導入した資産、このビルドの同梱物の順。別の置き場所は探さない。
    let mut candidates = vec![crate::app_paths::bundled_dir(app)
        .join("local-vision")
        .join(name)];
    if let Some(packaged) = crate::app_paths::packaged_bundled_dir(app) {
        candidates.push(packaged.join("local-vision").join(name));
    }
    select_vision_projector(
        &candidates,
        local::vision_asset::SIZE,
        local::vision_asset::SHA256,
    )
    .await
    .map(Some)
}

async fn select_vision_projector(
    candidates: &[PathBuf],
    size: u64,
    sha256: &str,
) -> anyhow::Result<PathBuf> {
    for candidate in candidates {
        if candidate.exists() {
            anyhow::ensure!(
                model_file_matches(candidate, size, sha256).await?,
                "画像入力用資産が破損または非対応です: {}。cargo xtask setup local-vision で再導入してください",
                candidate.display()
            );
            return Ok(candidate.clone());
        }
    }
    anyhow::bail!("画像入力用資産がありません。cargo xtask setup local-vision で約986MBの追加資産を明示導入してください。取得元: {}", local::vision_asset::URL)
}

pub struct EngineState {
    pub status: Mutex<Value>,
    pub child: Mutex<Option<Child>>,
    /// `stop()` が呼ばれたら true に設定し、`ensure_and_start` がその後に spawn しないようにする。
    pub stopping: AtomicBool,
    start_operation: tokio::sync::Mutex<()>,
    start_generation: tokio::sync::watch::Sender<u64>,
}

impl EngineState {
    pub fn new() -> Self {
        EngineState {
            status: Mutex::new(json!({ "phase": "idle" })),
            child: Mutex::new(None),
            stopping: AtomicBool::new(false),
            start_operation: tokio::sync::Mutex::new(()),
            start_generation: tokio::sync::watch::channel(0).0,
        }
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn set_status(app: &AppHandle, value: Value) {
    if let Some(engine) = app.try_state::<EngineState>() {
        *engine.status.lock().unwrap() = value.clone();
    }
    if let Err(error) = app.emit("engine:status", value) {
        crate::log_paths::append_app_log(
            app,
            &format!("ローカルAIの状態を画面へ通知できません: {error}"),
        );
    }
}

// 同梱した llama-server 実行ファイルを解決する（バンドル時と dev で場所が異なるため候補を探す）。
fn server_path(app: &AppHandle) -> PathBuf {
    let exe = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    // 差し替え用に取得したエンジン、このビルドの同梱物の順。作業フォルダや旧配置は探さない。
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(crate::app_paths::bundled_dir(app).join("engine").join(exe));
    if let Some(packaged) = crate::app_paths::packaged_bundled_dir(app) {
        candidates.push(packaged.join("engine").join(exe));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        // WindowsのCUDA/ROCm/SYCL管理版は、この共通版が未配置でも候補解決側で
        // 先に検査・準備できる。存在しない既定パスはフォールバック候補として渡し、
        // 実在候補が1件も無い場合だけ呼び出し側で未導入エラーにする。
        .unwrap_or_else(|| crate::app_paths::bundled_dir(app).join("engine").join(exe))
}

fn existing_backend_candidates(
    candidates: Vec<crate::engine_backend::BackendCandidate>,
) -> Vec<crate::engine_backend::BackendCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| candidate.server.is_file())
        .collect()
}

fn model_url(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    local::model_url(&state.config)
}

fn model_path(app: &AppHandle) -> PathBuf {
    let url = model_url(app);
    let name = local::preset_by_url(&url).map_or_else(
        || {
            url.split('/')
                .next_back()
                .filter(|s| !s.is_empty())
                .unwrap_or("model.gguf")
        },
        |preset| preset.file_name,
    );
    crate::app_paths::data_path(app, crate::app_paths::DataPath::Models).join(name)
}

/// ダウンロード済みモデルの保存ディレクトリ（`app_data_dir/bundled/models`）。
pub fn models_dir(app: &AppHandle) -> PathBuf {
    crate::app_paths::data_path(app, crate::app_paths::DataPath::Models)
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
    let port = local::port(&state.config).map_err(anyhow::Error::msg)?;
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
            crate::log_paths::append_app_log(
                app,
                &format!("既存ローカルエンジンのモデル確認に失敗しました: {error}"),
            );
            false
        }
    }
}

fn migration_file_names(url: &str) -> Vec<&str> {
    let mut names = Vec::new();
    if let Some(preset) = local::preset_by_url(url) {
        names.push(preset.file_name);
    }
    let source_name = url
        .split('/')
        .next_back()
        .filter(|name| !name.is_empty())
        .unwrap_or("model.gguf");
    if !names.contains(&source_name) {
        names.push(source_name);
    }
    names
}

fn legacy_model_paths(app: &AppHandle) -> Vec<PathBuf> {
    let url = model_url(app);
    let names = migration_file_names(&url);
    model_storage_roots(app)
        .into_iter()
        .flat_map(|root| names.iter().map(move |name| root.join(name)))
        .collect()
}

/// モデルの置き場所は `bundled/models` だけ。旧配置（`models/` や旧識別子フォルダ）は探さない。
fn model_storage_roots(app: &AppHandle) -> Vec<PathBuf> {
    vec![models_dir(app)]
}

async fn preset_model_matches(
    path: &Path,
    preset: &local::LocalModelPreset,
) -> anyhow::Result<bool> {
    model_file_matches(path, preset.size_bytes, preset.sha256).await
}

async fn model_file_matches(path: &Path, size_bytes: u64, sha256: &str) -> anyhow::Result<bool> {
    if tokio::fs::metadata(path).await?.len() != size_bytes {
        return Ok(false);
    }
    let path = path.to_owned();
    let actual =
        tokio::task::spawn_blocking(move || crate::installer::fs::sha256_file(&path)).await??;
    Ok(actual.eq_ignore_ascii_case(sha256))
}

async fn migrate_legacy_model(app: &AppHandle, dest: &PathBuf) -> anyhow::Result<()> {
    if dest.exists() {
        return Ok(());
    }
    let url = model_url(app);
    let expected = local::preset_by_url(&url);
    let mut selected = None;
    let candidates: Vec<PathBuf> = legacy_model_paths(app)
        .into_iter()
        .filter(|path| path != dest && path.exists())
        .collect();
    // 数GBのSHA-256照合は分単位かかる。無表示のままだと起動が固まったように見える。
    if !candidates.is_empty() {
        set_status(app, json!({ "phase": "verify" }));
    }
    for path in candidates {
        if let Some(preset) = expected {
            match preset_model_matches(&path, preset).await {
                Ok(true) => {}
                Ok(false) => {
                    crate::log_paths::append_app_log(
                        app,
                        &format!(
                            "旧ローカルモデルは現在の固定版と一致しないため再利用しません: {}",
                            path.display()
                        ),
                    );
                    continue;
                }
                Err(error) => {
                    crate::log_paths::append_app_log(
                        app,
                        &format!(
                            "旧ローカルモデルを検証できません: {} ({error})",
                            path.display()
                        ),
                    );
                    continue;
                }
            }
        }
        selected = Some(path);
        break;
    }
    let Some(old_path) = selected else {
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
            if let Err(error) = tokio::fs::remove_file(&old_path).await {
                crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "移行済みの旧モデルを削除できません: {} ({error})",
                        old_path.display()
                    ),
                );
            }
            Ok(())
        }
    }
}

async fn remove_obsolete_model_files(
    dest: &Path,
    candidates: Vec<PathBuf>,
    size_bytes: u64,
    sha256: &str,
) -> anyhow::Result<usize> {
    let obsolete: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|path| path != dest && path.is_file())
        .collect();
    if obsolete.is_empty() {
        return Ok(0);
    }
    anyhow::ensure!(
        model_file_matches(dest, size_bytes, sha256).await?,
        "現行ローカルモデルの検証前に旧ファイルを削除しようとしました"
    );
    for path in &obsolete {
        tokio::fs::remove_file(path).await.map_err(|error| {
            anyhow::anyhow!(
                "旧ローカルモデルを削除できません: {} ({error})",
                path.display()
            )
        })?;
    }
    Ok(obsolete.len())
}

async fn cleanup_obsolete_preset_models(app: &AppHandle, dest: &Path) -> anyhow::Result<usize> {
    let url = model_url(app);
    let Some(preset) = local::preset_by_url(&url) else {
        return Ok(0);
    };
    let names = local::obsolete_model_file_names(Some(preset));
    let candidates: Vec<PathBuf> = model_storage_roots(app)
        .into_iter()
        .flat_map(|root| names.iter().map(move |name| root.join(name)))
        .filter(|path| path != dest && path.is_file())
        .collect();
    if candidates.is_empty() {
        return Ok(0);
    }
    // 削除前の現行モデル照合も数GBのSHA-256になるため、進行中であることを画面へ出す。
    set_status(app, json!({ "phase": "verify" }));
    remove_obsolete_model_files(dest, candidates, preset.size_bytes, preset.sha256).await
}

// モデルをストリーミングダウンロード（進捗を emit）。
async fn download_model(app: &AppHandle, dest: &PathBuf) -> anyhow::Result<()> {
    let url = model_url(app);
    let expected = local::preset_by_url(&url);
    // カスタムGGUFの互換性を任意の固定容量で制限せず、開始時の利用可能容量を上限にする。
    // 旧モデルは検証済み一時ファイルの公開まで保持するため、その分は空き容量に含めない。
    let max_size = crate::installer::disk::available_bytes(dest)?;
    let spec = crate::installer::download::FetchSpec {
        url: &url,
        destination: dest,
        expected_size: expected.map(|preset| preset.size_bytes),
        max_size,
        sha256: expected.map(|preset| preset.sha256),
    };
    let mut progress = |received: u64, total: u64| {
        let percent = if total == 0 {
            0
        } else {
            ((received as u128 * 100) / total as u128) as u64
        };
        set_status(
            app,
            json!({ "phase": "download", "percent": percent, "receivedBytes": received, "totalBytes": total }),
        );
    };
    crate::installer::download::fetch_fixed(spec, &mut progress).await?;
    Ok(())
}

pub(crate) fn total_memory_bytes() -> Option<u64> {
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
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| parse_meminfo_total_bytes(&text))
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_meminfo_total_bytes(text: &str) -> Option<u64> {
    let kb = text.lines().find_map(|line| {
        let rest = line.strip_prefix("MemTotal:")?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })?;
    // /proc/meminfo の kB は実質 KiB（1024バイト）として定義されている。
    kb.checked_mul(1_024)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn parse_nvidia_memory_bytes(text: &str) -> Option<u64> {
    text.lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()?
        .checked_mul(1_048_576)
}

/// GPU の専用VRAM総量（バイト）。正確に取得できる場合だけ返す。
///
/// `Win32_VideoController.AdapterRAM` はuint32で4GB超が飽和するため使わない。
/// Windows NVIDIAはドライバ同梱のnvidia-smiを短時間だけ呼び、他GPUは不明として
/// RAM側の候補判定へ回す。
async fn gpu_vram_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        let mut command = crate::tools::process::command("nvidia-smi.exe");
        command.args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"]);
        // nvidia-smi が無い、または3秒以内に応答しないPCでは
        // 専用VRAMを不明としてRAMだけで候補を案内する。
        let output = crate::tools::process::output(command, 3, "NVIDIA GPUメモリ量の確認")
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_nvidia_memory_bytes(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn decimal_gb(bytes: u64) -> f64 {
    ((bytes as f64 / 1_000_000_000.0) * 10.0).round() / 10.0
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalHardwareProfile {
    ram_gb: Option<f64>,
    vram_gb: Option<f64>,
    unified_memory: bool,
}

pub async fn local_hardware_profile() -> LocalHardwareProfile {
    LocalHardwareProfile {
        ram_gb: total_memory_bytes().map(decimal_gb),
        vram_gb: gpu_vram_bytes().await.map(decimal_gb),
        unified_memory: cfg!(target_os = "macos"),
    }
}

async fn ensure_local_model_selection(app: &AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let requested = state.config.get("PICOAGENT_LOCAL_MODEL_PRESET", "auto");
    let should_auto = requested == "auto"
        || (!state.config.has_value("PICOAGENT_LOCAL_MODEL_PRESET")
            && !state.config.has_value("PICOAGENT_MODEL_URL"));
    let selected = if should_auto {
        local::DEFAULT_LOCAL_MODEL_PRESET
    } else if local::preset(&requested).is_some() || requested == "custom" {
        local::canonical_preset_id(&requested)
    } else {
        local::DEFAULT_LOCAL_MODEL_PRESET
    };
    if selected == "custom" {
        return Ok(());
    }
    let Some(preset) = local::preset(selected) else {
        anyhow::bail!("ローカルモデルの既定プリセットが見つかりません: {selected}");
    };
    let mut patch = Map::new();
    if requested != selected {
        patch.insert(
            "PICOAGENT_LOCAL_MODEL_PRESET".to_string(),
            Value::String(selected.to_string()),
        );
    }
    let configured_model = state.config.get("PICOAGENT_MODEL", "");
    if configured_model != preset.id {
        patch.insert(
            "PICOAGENT_MODEL".to_string(),
            Value::String(preset.id.to_string()),
        );
    }
    // 保存済み値と環境変数を優先する。旧既定値でも手動指定と区別できないため変更しない。
    if !state.config.has_value("PICOAGENT_LOCAL_CTX") {
        patch.insert(
            "PICOAGENT_LOCAL_CTX".to_string(),
            Value::String(preset.ctx.to_string()),
        );
    }
    if !patch.is_empty() {
        state
            .config
            .set_all(patch)
            .map_err(|error| anyhow::anyhow!("ローカルモデルの選択を保存できません: {error}"))?;
        *state.model.lock().unwrap() = preset.id.to_string();
    }
    Ok(())
}

/// 保存済みGPU名が候補バックエンドで使える場合だけ返す。
/// llama.cppのデバイス名はバックエンド固有なので、たとえば`Vulkan1`をCUDA版へ
/// 渡すと起動前に終了する。未知の名前は後方互換のためそのまま渡す。
fn gpu_device_for_candidate<'a>(candidate_label: &str, configured: &'a str) -> Option<&'a str> {
    let device = configured.trim();
    if device.is_empty() || candidate_label.eq_ignore_ascii_case("CPU") {
        return None;
    }
    let known_backend = ["CUDA", "Vulkan", "ROCm", "SYCL", "Metal"]
        .into_iter()
        .find(|backend| device.starts_with(backend));
    match known_backend {
        Some(backend) if !candidate_label.eq_ignore_ascii_case(backend) => None,
        _ => Some(device),
    }
}

// llama-server を起動する。
fn spawn_server(
    app: &AppHandle,
    model: &PathBuf,
    candidate: &crate::engine_backend::BackendCandidate,
    projector: Option<&Path>,
    start_generation: u64,
) -> anyhow::Result<()> {
    let server = &candidate.server;
    crate::installer::fs::prepare_executable(app, server)?;
    let state = app.state::<AppState>();
    let port = local::port(&state.config)
        .map_err(anyhow::Error::msg)?
        .to_string();
    // 設定値を起動前に検証し、不正な文脈長のままサーバを起動しない。
    let ctx = local::context_length(&state.config)
        .map_err(anyhow::Error::msg)?
        .to_string();
    // GPU 優先: 既定で全レイヤを GPU へオフロード（999）。GPU 非対応ビルド/環境では
    // llama-server が自動で CPU にフォールバックする。CPU 強制は 0 を設定する。
    let ngl = &candidate.ngl;
    // 使用 GPU の指定（複数 GPU 環境向け）。空なら llama-server の既定（先頭デバイス）。
    // 例: Vulkan ビルドで discrete GPU を使う場合は "Vulkan1" のように指定する。
    let gpu_device = state.config.get("PICOAGENT_LOCAL_GPU_DEVICE", "");
    let candidate_gpu_device = gpu_device_for_candidate(candidate.label, &gpu_device);

    let mut cmd = crate::tools::process::command_sync(server);
    cmd.arg("-m")
        .arg(model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(&port)
        .arg("-c")
        .arg(&ctx)
        .arg("-ngl")
        .arg(ngl)
        .arg("--jinja");
    if let Some(projector) = projector {
        cmd.arg("--mmproj").arg(projector);
        if ngl.trim() == "0" {
            cmd.arg("--no-mmproj-offload");
        }
    }
    // Qwen3.5 は既定で推論過程を生成する。キャラクターの発話へ内部推論を混ぜず、
    // llama.cpp のツール呼び出し解析を安定させるため、非推論モードで起動する。
    if model
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("Qwen3.5-"))
    {
        cmd.arg("--chat-template-kwargs")
            .arg(r#"{"enable_thinking":false}"#)
            .arg("--reasoning-budget")
            .arg("0");
    }
    // 候補と異なる方式の保存済みデバイス名を渡して、起動前に壊さない。
    if ngl.trim() != "0" {
        if let Some(device) = candidate_gpu_device {
            cmd.arg("--device").arg(device);
        }
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
        "\n==== llama-server 起動: backend={} model={} port={} ctx={} ngl={} device={} ====",
        candidate.label,
        model.display(),
        port,
        ctx,
        ngl,
        candidate_gpu_device.unwrap_or("(default)")
    )?;
    let err_log = log.try_clone()?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log));
    let mut child = cmd.spawn()?;
    let Some(engine) = app.try_state::<EngineState>() else {
        terminate_child(app, &mut child);
        anyhow::bail!("EngineState を取得できないため、起動したエンジンを停止しました");
    };
    let mut slot = match engine.child.lock() {
        Ok(slot) => slot,
        // ハンドルを失わないよう、poison 後も格納済み状態を引き継ぐ。
        Err(poisoned) => poisoned.into_inner(),
    };
    if !start_generation_is_current(engine.inner(), start_generation) {
        terminate_child(app, &mut child);
        anyhow::bail!("停止または再起動済みの処理のため、起動したエンジンを停止しました");
    }
    if slot.is_some() {
        terminate_child(app, &mut child);
        anyhow::bail!("ローカルエンジンは既に起動中です");
    }
    *slot = Some(child);
    Ok(())
}

// モデル取得→エンジン起動→ready 検知までを通しで行う（async タスクから呼ぶ）。
pub async fn ensure_and_start(app: AppHandle) {
    let Some(engine) = app.try_state::<EngineState>() else {
        return;
    };
    let generation = engine.start_generation.subscribe();
    let start_generation = *generation.borrow();
    run_start_operation(
        engine.inner(),
        generation,
        ensure_and_start_inner(app.clone(), start_generation),
    )
    .await;
}

/// 停止世代の通知で旧処理を取り消し、その解放後にだけ次の取得・起動を進める。
async fn run_start_operation(
    engine: &EngineState,
    mut generation: tokio::sync::watch::Receiver<u64>,
    operation: impl std::future::Future<Output = ()>,
) {
    let _operation = tokio::select! {
        biased;
        _ = generation.changed() => return,
        guard = engine.start_operation.lock() => guard,
    };
    if engine.stopping.load(Ordering::SeqCst) || generation.has_changed().unwrap_or(true) {
        return;
    }
    tokio::select! {
        biased;
        _ = generation.changed() => {},
        _ = operation => {},
    }
}

fn start_generation_is_current(engine: &EngineState, generation: u64) -> bool {
    !engine.stopping.load(Ordering::SeqCst) && *engine.start_generation.borrow() == generation
}

async fn ensure_and_start_inner(app: AppHandle, start_generation: u64) {
    // 不正設定がある場合は、資産の移行・取得や既存サーバへの接続より前に止める。
    let config = &app.state::<AppState>().config;
    let validated = config.int(&crate::config::ints::LOCAL_NGL).and_then(|ngl| {
        local::port(config)?;
        local::context_length(config)?;
        Ok(ngl.to_string())
    });
    let configured_ngl = match validated {
        Ok(value) => value,
        Err(error) => {
            set_status(&app, json!({"phase":"error", "message":error}));
            return;
        }
    };
    if let Err(error) = ensure_local_model_selection(&app).await {
        set_status(
            &app,
            json!({ "phase": "error", "message": error.to_string() }),
        );
        return;
    }
    let projector = match vision_projector(&app).await {
        Ok(projector) => projector,
        Err(error) => {
            VISION_READY.store(false, Ordering::SeqCst);
            set_status(
                &app,
                json!({ "phase": "error", "message": error.to_string() }),
            );
            return;
        }
    };
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
    match cleanup_obsolete_preset_models(&app, &model).await {
        Ok(removed) if removed > 0 => crate::log_paths::append_app_log(
            &app,
            &format!("検証済み現行版へ移行し、旧ローカルモデル{removed}件を削除しました"),
        ),
        Ok(_) => {}
        // 旧ファイルの掃除失敗だけで検証済み現行モデルの起動を止めない。失敗はログへ残す。
        Err(error) => crate::log_paths::append_app_log(
            &app,
            &format!("旧ローカルモデルの掃除に失敗しました: {error}"),
        ),
    }
    // 現行モデルが揃った後でだけ、旧配置（利用者データ直下の `models/`）を片付ける。
    crate::log_paths::remove_legacy_layout_dir(&app, "models");
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
        if match provider.is_up(&state.config).await {
            Ok(ready) => ready,
            Err(error) => {
                set_status(&app, json!({"phase":"error", "message":error}));
                return;
            }
        } {
            if running_server_uses_model(&app, &model).await
                && (projector.is_none() || VISION_READY.load(Ordering::SeqCst))
            {
                set_status(&app, json!({ "phase": "ready" }));
                return;
            }
            set_status(
                &app,
                json!({
                    "phase": "error",
                    "message": "前に使っていたローカルAIがまだ動いています。設定の「診断・修復」から「ローカルAIを再起動」を押してください。直らないときは PicoAgent を終了して開き直してください"
                }),
            );
            return;
        }
    }
    let bundled_server = server_path(&app);
    let candidates = existing_backend_candidates(
        crate::engine_backend::candidates(&app, bundled_server, configured_ngl).await,
    );
    if candidates.is_empty() {
        set_status(
            &app,
            json!({ "phase": "error", "message": "ローカルAIエンジンの実行ファイルが見つかりません。設定の「診断・修復」から再インストールを試してください" }),
        );
        return;
    }

    // /health が 200 を返すまで待つ。CUDAが起動失敗した場合はVulkan、最後にCPUへ切り替える。
    let state = app.state::<AppState>();
    let provider = match crate::providers::get("local") {
        Some(p) => p,
        None => {
            terminate_managed_child(&app);
            set_status(
                &app,
                json!({ "phase": "error", "message": "ローカルAIを利用できません。設定の「AIモデル」から使うAIを選び直してください" }),
            );
            return;
        }
    };
    let mut failures = Vec::new();
    for candidate in candidates {
        set_status(
            &app,
            json!({ "phase": "starting", "message": format!("{}でローカルAIを起動しています", candidate.label) }),
        );
        VISION_READY.store(false, Ordering::SeqCst);
        if let Err(error) = spawn_server(
            &app,
            &model,
            &candidate,
            projector.as_deref(),
            start_generation,
        ) {
            failures.push(format!("{}: {error}", candidate.label));
            continue;
        }
        let mut failed = None;
        for _ in 0..300 {
            if match provider.is_up(&state.config).await {
                Ok(ready) => ready,
                Err(error) => {
                    set_status(&app, json!({"phase":"error", "message":error}));
                    return;
                }
            } {
                VISION_READY.store(projector.is_some(), Ordering::SeqCst);
                crate::log_paths::append_app_log(
                    &app,
                    &format!("ローカルAIを{}バックエンドで起動しました", candidate.label),
                );
                set_status(
                    &app,
                    json!({ "phase": "ready", "backend": candidate.label }),
                );
                return;
            }
            match managed_child_exit_status(&app) {
                Ok(Some(status)) => {
                    failed = Some(format!("起動直後に終了しました: {status}"));
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        terminate_managed_child(&app);
        failures.push(format!(
            "{}: {}",
            candidate.label,
            failed.unwrap_or_else(|| "時間内に起動しませんでした".to_string())
        ));
        crate::log_paths::append_app_log(
            &app,
            &format!(
                "ローカルAIのバックエンドを切り替えます: {}",
                failures.last().unwrap()
            ),
        );
    }
    set_status(
        &app,
        json!({ "phase": "error", "message": format!("ローカルAIを起動できませんでした（{}）。設定の「診断・修復」から再起動を試してください", failures.join(" / ")) }),
    );
}

fn managed_child_exit_status(app: &AppHandle) -> Result<Option<std::process::ExitStatus>, String> {
    let Some(engine) = app.try_state::<EngineState>() else {
        return Err("EngineState を取得できません".to_string());
    };
    let mut slot = match engine.child.lock() {
        Ok(slot) => slot,
        // 終了確認と解放を優先し、poison 後も保持中のハンドルを回収する。
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(child) = slot.as_mut() else {
        return Ok(None);
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            VISION_READY.store(false, Ordering::SeqCst);
            *slot = None;
            Ok(Some(status))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(format!("エンジンの終了状態を確認できません: {error}")),
    }
}

fn terminate_child(app: &AppHandle, child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let taskkill = crate::tools::process::command_sync("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Err(error) = taskkill {
            crate::log_paths::append_app_log(
                app,
                &format!("ローカルエンジンのプロセスツリー停止に失敗しました: {error}"),
            );
        }
    }
    // taskkill 自体が失敗した場合も、直接の子プロセスだけは必ず停止を試みる。
    if let Err(error) = child.kill() {
        if error.kind() != std::io::ErrorKind::InvalidInput {
            crate::log_paths::append_app_log(
                app,
                &format!("ローカルエンジンを停止できませんでした: {error}"),
            );
        }
    }
    if let Err(error) = child.wait() {
        crate::log_paths::append_app_log(
            app,
            &format!("ローカルエンジンの終了ハンドルを回収できませんでした: {error}"),
        );
    }
}

fn terminate_managed_child(app: &AppHandle) {
    VISION_READY.store(false, Ordering::SeqCst);
    let Some(engine) = app.try_state::<EngineState>() else {
        return;
    };
    let child = match engine.child.lock() {
        Ok(mut slot) => slot.take(),
        // 停止処理は Mutex の poison 後も中のハンドルを回収する。
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(mut child) = child {
        terminate_child(app, &mut child);
    }
}

/// 子プロセスを停止する（アプリ終了時）。
/// `stopping` フラグを先に立てて `ensure_and_start` の後続 spawn を防ぐ。
pub fn stop(app: &AppHandle) {
    if let Some(engine) = app.try_state::<EngineState>() {
        engine.stopping.store(true, Ordering::SeqCst);
        engine.start_generation.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        terminate_managed_child(app);
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

#[cfg(test)]
mod tests {
    use super::{
        decimal_gb, existing_backend_candidates, gpu_device_for_candidate, migration_file_names,
        model_file_matches, parse_meminfo_total_bytes, parse_nvidia_memory_bytes,
        remove_obsolete_model_files,
    };
    use crate::providers::local;

    #[test]
    fn 停止フラグ解除後も旧世代の子プロセス登録を拒否する() {
        let engine = super::EngineState::new();
        let old_generation = *engine.start_generation.borrow();
        assert!(super::start_generation_is_current(&engine, old_generation));
        engine
            .stopping
            .store(true, std::sync::atomic::Ordering::SeqCst);
        engine.start_generation.send_modify(|value| *value += 1);
        engine
            .stopping
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!super::start_generation_is_current(&engine, old_generation));
        assert!(super::start_generation_is_current(
            &engine,
            *engine.start_generation.borrow()
        ));
    }

    #[tokio::test]
    async fn 再起動は旧取得を取り消してから新取得を開始する() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let engine = std::sync::Arc::new(super::EngineState::new());
        let active = std::sync::Arc::new(AtomicUsize::new(0));
        struct Active(std::sync::Arc<AtomicUsize>);
        impl Drop for Active {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let (started, ready) = tokio::sync::oneshot::channel();
        let old_engine = engine.clone();
        let old_active = active.clone();
        let old_generation = engine.start_generation.subscribe();
        let old = tokio::spawn(async move {
            super::run_start_operation(&old_engine, old_generation, async move {
                old_active.fetch_add(1, Ordering::SeqCst);
                let _guard = Active(old_active);
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            })
            .await;
        });
        ready.await.unwrap();
        // stop直後にclear_stoppingしても、旧購読者への取消通知は消えない。
        engine.stopping.store(true, Ordering::SeqCst);
        engine.start_generation.send_modify(|value| *value += 1);
        engine.stopping.store(false, Ordering::SeqCst);
        super::run_start_operation(&engine, engine.start_generation.subscribe(), async {
            assert_eq!(active.load(Ordering::SeqCst), 0);
        })
        .await;
        old.await.unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn 画像資産は優先候補を検証し破損時に別候補へ降格しない() {
        use sha2::{Digest, Sha256};
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../temp")
            .join(format!(
                "vision-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first.gguf");
        let second = root.join("second.gguf");
        let candidates = [first.clone(), second.clone()];
        let hash = hex::encode(Sha256::digest(b"valid"));
        assert!(super::select_vision_projector(&candidates, 5, &hash)
            .await
            .is_err());
        std::fs::write(&second, b"valid").unwrap();
        assert_eq!(
            super::select_vision_projector(&candidates, 5, &hash)
                .await
                .unwrap(),
            second
        );
        std::fs::write(&first, b"valid").unwrap();
        assert_eq!(
            super::select_vision_projector(&candidates, 5, &hash)
                .await
                .unwrap(),
            first
        );
        std::fs::remove_file(&second).unwrap();
        assert_eq!(
            super::select_vision_projector(&candidates, 5, &hash)
                .await
                .unwrap(),
            first
        );
        std::fs::write(&second, b"valid").unwrap();
        std::fs::write(&first, b"wrong").unwrap();
        assert!(super::select_vision_projector(&candidates, 5, &hash)
            .await
            .is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn revision保存名への移行候補に旧配布名も含める() {
        let preset = local::preset("small").unwrap();
        let names = migration_file_names(preset.url);
        assert_eq!(names[0], preset.file_name);
        assert_eq!(names[1], "gemma-4-E2B-it-Q4_K_M.gguf");
    }

    #[test]
    fn gpu名は対応するバックエンド候補だけへ渡す() {
        assert_eq!(
            gpu_device_for_candidate("Vulkan", "Vulkan1"),
            Some("Vulkan1")
        );
        assert_eq!(gpu_device_for_candidate("CUDA", "Vulkan1"), None);
        assert_eq!(gpu_device_for_candidate("CUDA", "CUDA0"), Some("CUDA0"));
        assert_eq!(gpu_device_for_candidate("CPU", "CUDA0"), None);
        assert_eq!(
            gpu_device_for_candidate("CUDA", "  custom:0  "),
            Some("custom:0")
        );
        assert_eq!(gpu_device_for_candidate("CUDA", "  "), None);
    }

    #[test]
    fn 実在する管理バックエンドだけを起動候補に残す() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-backend-candidates-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let cuda = root.join("engine-cuda").join("llama-server.exe");
        std::fs::create_dir_all(cuda.parent().unwrap()).unwrap();
        std::fs::write(&cuda, b"test").unwrap();
        let candidates = vec![
            crate::engine_backend::BackendCandidate {
                label: "CUDA",
                server: cuda.clone(),
                ngl: "999".to_string(),
            },
            crate::engine_backend::BackendCandidate {
                label: "Vulkan",
                server: root.join("engine").join("llama-server.exe"),
                ngl: "999".to_string(),
            },
        ];
        let existing = existing_backend_candidates(candidates);
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].label, "CUDA");
        assert_eq!(existing[0].server, cuda);

        let generic = root.join("engine").join("llama-server.exe");
        std::fs::create_dir_all(generic.parent().unwrap()).unwrap();
        std::fs::write(&generic, b"test").unwrap();
        let fallback_only = existing_backend_candidates(vec![
            crate::engine_backend::BackendCandidate {
                label: "CUDA",
                server: root.join("engine-cuda-missing").join("llama-server.exe"),
                ngl: "999".to_string(),
            },
            crate::engine_backend::BackendCandidate {
                label: "Vulkan",
                server: generic.clone(),
                ngl: "999".to_string(),
            },
        ]);
        assert_eq!(fallback_only.len(), 1);
        assert_eq!(fallback_only[0].label, "Vulkan");
        assert_eq!(fallback_only[0].server, generic);

        let none = existing_backend_candidates(vec![crate::engine_backend::BackendCandidate {
            label: "CUDA",
            server: root.join("missing").join("llama-server.exe"),
            ngl: "999".to_string(),
        }]);
        assert!(none.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pcメモリ量を十進gbへ丸める() {
        assert_eq!(decimal_gb(8_550_000_000), 8.6);
        assert_eq!(decimal_gb(730_894_624), 0.7);
    }

    #[test]
    fn linuxとnvidiaのメモリ出力を総量へ変換する() {
        assert_eq!(
            parse_meminfo_total_bytes("MemTotal:       16384000 kB\nMemFree: 10 kB"),
            Some(16_777_216_000)
        );
        assert_eq!(
            parse_nvidia_memory_bytes("4096\n8151\n"),
            Some(8_546_942_976)
        );
        assert_eq!(parse_nvidia_memory_bytes("N/A\n"), None);
    }

    #[test]
    fn 旧モデルはサイズとsha256の両方が一致した場合だけ再利用する() {
        use sha2::{Digest, Sha256};

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let path = std::env::temp_dir().join(format!(
                "picoagent-model-migration-{}-{}.gguf",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            tokio::fs::write(&path, b"picoagent").await.unwrap();
            let sha256 = hex::encode(Sha256::digest(b"picoagent"));
            assert!(model_file_matches(&path, 9, &sha256).await.unwrap());
            assert!(!model_file_matches(&path, 8, &sha256).await.unwrap());
            assert!(!model_file_matches(&path, 9, &"0".repeat(64)).await.unwrap());
            tokio::fs::remove_file(path).await.unwrap();
        });
    }

    #[test]
    fn 現行モデルの検証成功後だけ旧ファイルを削除する() {
        use sha2::{Digest, Sha256};

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir().join(format!(
                "picoagent-model-cleanup-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            tokio::fs::create_dir_all(&root).await.unwrap();
            let dest = root.join("current.gguf");
            let old = root.join("old.gguf");
            tokio::fs::write(&dest, b"current").await.unwrap();
            tokio::fs::write(&old, b"old").await.unwrap();
            let sha256 = hex::encode(Sha256::digest(b"current"));
            let removed =
                remove_obsolete_model_files(&dest, vec![dest.clone(), old.clone()], 7, &sha256)
                    .await
                    .unwrap();
            assert_eq!(removed, 1);
            assert!(dest.is_file());
            assert!(!old.exists());
            tokio::fs::remove_dir_all(root).await.unwrap();
        });
    }
}
