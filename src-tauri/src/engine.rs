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
    if let Err(error) = app.emit("engine:status", value) {
        crate::log_paths::append_app_log(
            app,
            &format!("ローカルAIの状態を画面へ通知できません: {error}"),
        );
    }
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
    let name = local::preset_by_url(&url).map_or_else(
        || {
            url.split('/')
                .next_back()
                .filter(|s| !s.is_empty())
                .unwrap_or("model.gguf")
        },
        |preset| preset.file_name,
    );
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

fn model_storage_roots(app: &AppHandle) -> Vec<PathBuf> {
    let mut roots = vec![
        models_dir(app),
        crate::log_paths::data_dir(app).join("models"),
    ];
    if let Ok(dir) = app.path().app_data_dir() {
        roots.push(dir.join("models"));
    }
    roots
}

async fn preset_model_matches(
    path: &Path,
    preset: &local::LocalModelPreset,
) -> anyhow::Result<bool> {
    model_file_matches(path, preset.size_bytes, preset.sha256).await
}

async fn model_file_matches(path: &Path, size_bytes: u64, sha256: &str) -> anyhow::Result<bool> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;

    if tokio::fs::metadata(path).await?.len() != size_bytes {
        return Ok(false);
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()) == sha256)
}

async fn migrate_legacy_model(app: &AppHandle, dest: &PathBuf) -> anyhow::Result<()> {
    if dest.exists() {
        return Ok(());
    }
    let url = model_url(app);
    let expected = local::preset_by_url(&url);
    let mut selected = None;
    for path in legacy_model_paths(app) {
        if path == *dest || !path.exists() {
            continue;
        }
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
    let names = local::obsolete_model_file_names();
    let candidates = model_storage_roots(app)
        .into_iter()
        .flat_map(|root| names.iter().map(move |name| root.join(name)))
        .collect();
    remove_obsolete_model_files(dest, candidates, preset.size_bytes, preset.sha256).await
}

// モデルをストリーミングダウンロード（進捗を emit）。
async fn download_model(app: &AppHandle, dest: &PathBuf) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let url = model_url(app);
    let expected = local::preset_by_url(&url);
    set_status(app, json!({ "phase": "download", "percent": 0 }));

    let mut res = crate::tools::http::client().get(&url).send().await?;
    if !res.status().is_success() {
        anyhow::bail!("モデル取得に失敗: {} {}", res.status(), url);
    }
    let total = res.content_length().unwrap_or(0);
    let tmp = dest.with_extension("part");
    let result: anyhow::Result<()> = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        let mut received: u64 = 0;
        let mut digest = Sha256::new();
        let mut last_emit: i64 = -1;

        while let Some(chunk) = res.chunk().await? {
            file.write_all(&chunk).await?;
            digest.update(&chunk);
            received += chunk.len() as u64;
            if let Some(pct) = received.saturating_mul(100).checked_div(total) {
                let pct = pct as i64;
                if pct != last_emit {
                    last_emit = pct;
                    // 数GBの取得は10分以上かかることがある。割合だけだと「どれくらい待つのか」
                    // が分からず、止まっているようにも見えるので実サイズも渡す。
                    set_status(
                        app,
                        json!({
                            "phase": "download",
                            "percent": pct,
                            "receivedBytes": received,
                            "totalBytes": total
                        }),
                    );
                }
            }
        }
        file.flush().await?;
        drop(file);
        if let Some(preset) = expected {
            let actual_sha256 = hex::encode(digest.finalize());
            anyhow::ensure!(
                received == preset.size_bytes && actual_sha256 == preset.sha256,
                "モデル検証に失敗: size={received}/{} sha256={actual_sha256}/{}",
                preset.size_bytes,
                preset.sha256
            );
        }
        tokio::fs::rename(&tmp, dest).await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        match tokio::fs::remove_file(&tmp).await {
            Ok(()) => return Err(error),
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => return Err(error),
            Err(cleanup) => {
                anyhow::bail!(
                    "モデル取得に失敗し、一時ファイルも削除できません: {} ({cleanup}); 元のエラー: {error}",
                    tmp.display()
                );
            }
        }
    }
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
/// Windows では WMI を短時間だけ呼ぶ。共通の非同期プロセス層を通すことで、
/// 起動停止中や WMI 不調時にも10秒で打ち切り、子プロセスを残さない。
async fn gpu_vram_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        let mut command = crate::tools::process::command("powershell");
        command.args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty AdapterRAM",
        ]);
        let output = crate::tools::process::output(command, 10, "GPUメモリ量の確認")
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
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

async fn auto_model_preset() -> &'static str {
    let vram = gpu_vram_bytes().await.unwrap_or(0);
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

async fn ensure_local_model_selection(app: &AppHandle) {
    let state = app.state::<AppState>();
    let requested = state.config.get("PICOAGENT_LOCAL_MODEL_PRESET", "auto");
    let should_auto = requested == "auto"
        || (!state.config.has_value("PICOAGENT_LOCAL_MODEL_PRESET")
            && !state.config.has_value("PICOAGENT_MODEL_URL"));
    let selected = if should_auto {
        auto_model_preset().await
    } else if local::preset(&requested).is_some() || requested == "custom" {
        local::canonical_preset_id(&requested)
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
    let configured_model = state.config.get("PICOAGENT_MODEL", "");
    let canonical_model = local::canonical_preset_id(&configured_model);
    if configured_model.is_empty() || configured_model != canonical_model {
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
        if let Err(error) = state.config.set_all(patch) {
            crate::log_paths::append_app_log(
                app,
                &format!("ローカルモデルの自動選択を保存できません: {error}"),
            );
            return;
        }
        *state.model.lock().unwrap() = preset.id.to_string();
    }
}

// llama-server を起動する。
fn spawn_server(app: &AppHandle, model: &PathBuf) -> anyhow::Result<()> {
    let server = server_path(app).ok_or_else(|| {
        anyhow::anyhow!(
            "ローカルAIエンジンの実行ファイルが見つかりません。設定の「診断・修復」から\
             再インストールを試すか、公式サイトから入れ直してください（開発環境では\
             `cargo xtask setup engine` でも用意できます）。"
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
    if slot.is_some() {
        terminate_child(app, &mut child);
        anyhow::bail!("ローカルエンジンは既に起動中です");
    }
    *slot = Some(child);
    Ok(())
}

// モデル取得→エンジン起動→ready 検知までを通しで行う（async タスクから呼ぶ）。
pub async fn ensure_and_start(app: AppHandle) {
    ensure_local_model_selection(&app).await;
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
        // フォールバック許可: 旧ファイルの掃除失敗だけで検証済み現行モデルの起動を止めない。失敗はログへ残す。
        Err(error) => crate::log_paths::append_app_log(
            &app,
            &format!("旧ローカルモデルの掃除に失敗しました: {error}"),
        ),
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
                    "message": "前に使っていたローカルAIがまだ動いています。設定の「診断・修復」から「ローカルAIを再起動」を押してください。直らないときは PicoAgent を終了して開き直してください"
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
        None => {
            terminate_managed_child(&app);
            set_status(
                &app,
                json!({ "phase": "error", "message": "ローカルAIを利用できません。設定の「AIモデル・連携」から使うAIを選び直してください" }),
            );
            return;
        }
    };
    for _ in 0..300 {
        if provider.is_up(&state.config).await {
            set_status(&app, json!({ "phase": "ready" }));
            return;
        }
        match managed_child_exit_status(&app) {
            Ok(Some(status)) => {
                set_status(
                    &app,
                    json!({
                        "phase": "error",
                        "message": format!("エンジンが起動直後に終了しました: {status}（engine.log を確認してください）")
                    }),
                );
                return;
            }
            Ok(None) => {}
            Err(error) => {
                terminate_managed_child(&app);
                set_status(&app, json!({ "phase": "error", "message": error }));
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    terminate_managed_child(&app);
    set_status(
        &app,
        json!({ "phase": "error", "message": "ローカルAIが時間内に起動しませんでした。設定の「診断・修復」から「ローカルAIを再起動」を試してください" }),
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
    use super::{migration_file_names, model_file_matches, remove_obsolete_model_files};
    use crate::providers::local;

    #[test]
    fn revision保存名への移行候補に旧配布名も含める() {
        let preset = local::preset("small").unwrap();
        let names = migration_file_names(preset.url);
        assert_eq!(names[0], preset.file_name);
        assert_eq!(names[1], "gemma-4-E2B-it-Q4_K_M.gguf");
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
