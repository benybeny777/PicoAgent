// aivis.rs - 同梱 AivisSpeech Engine の接続確認と裏起動を管理する。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use std::time::Duration;

use serde_json::Value;
use tauri::Manager;

use crate::commands::AppState;

const DEFAULT_PORT: u16 = match crate::config::ints::TTS_PORT.default {
    Some(value) => value as u16,
    None => panic!("音声ポートには整数の既定が必要です"),
};
const LEGACY_DEFAULT_PORT: u16 = 10102;
const DEFAULT_DEVICE: &str = "auto";
const STARTUP_CHECKS: usize = 360;
const MODEL_INSTALL_TIMEOUT: Duration = Duration::from_secs(900);
static AIVIS_INSTALL_FS_LOCK: Mutex<()> = Mutex::new(());
static AIVIS_INSTALL_NONCE: AtomicU64 = AtomicU64::new(1);

struct AivisInstallCleanup {
    app: tauri::AppHandle,
    path: PathBuf,
}

impl Drop for AivisInstallCleanup {
    fn drop(&mut self) {
        if self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".installing-aivis-"))
        {
            if let Some(parent) = self.path.parent() {
                match crate::installer::swap::has_pending(parent, "aivis") {
                    Ok(false) => {}
                    Ok(true) => return,
                    Err(error) => {
                        crate::log_paths::append_app_log(
                            &self.app,
                            &format!("Aivis復旧記録を確認できないため準備先を保持します: {error}"),
                        );
                        return;
                    }
                }
            }
        }
        // 正常・失敗・中断時のPicoAgent管理下一時物だけを掃除する。
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::log_paths::append_app_log(
                    &self.app,
                    &format!(
                        "Aivis導入の一時フォルダを削除できません: {} ({error})",
                        self.path.display()
                    ),
                );
            }
        }
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

fn configured_port(
    config: &crate::config::Config,
    migrate: impl FnOnce(String) -> Result<(), String>,
) -> Result<u16, String> {
    let port = config.int(&crate::config::ints::TTS_PORT)? as u16;
    if port == LEGACY_DEFAULT_PORT {
        // docs/SETTINGS.md「PICOAGENT_TTS_PORT」の既存移行。保存成功後だけ新ポートを使う。
        migrate(DEFAULT_PORT.to_string())?;
        return Ok(DEFAULT_PORT);
    }
    Ok(port)
}

pub fn port(app: &tauri::AppHandle, state: &AppState) -> Result<u16, String> {
    configured_port(&state.config, |next| {
        state
            .config
            .set_string("PICOAGENT_TTS_PORT", next.clone())
            .map_err(|error| format!("Aivis ポートの旧既定値移行に失敗しました: {error}"))?;
        append_log_best_effort(
            app,
            &format!("Aivis ポートを旧既定値 {LEGACY_DEFAULT_PORT} から {next} へ移行しました。"),
        );
        Ok(())
    })
}

pub fn endpoint(app: &tauri::AppHandle, state: &AppState) -> Result<String, String> {
    let external = state.config.get("PICOAGENT_TTS_ENDPOINT", "");
    if !external.trim().is_empty() {
        return Ok(external.trim().trim_end_matches('/').to_string());
    }
    Ok(format!("http://127.0.0.1:{}", port(app, state)?))
}

pub fn uses_external_endpoint(state: &AppState) -> bool {
    !state
        .config
        .get("PICOAGENT_TTS_ENDPOINT", "")
        .trim()
        .is_empty()
}

fn validate_external_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|error| anyhow::anyhow!("外部VOICEVOX互換APIのURLが不正です: {error}"))?;
    anyhow::ensure!(
        url.scheme() == "http",
        "外部VOICEVOX互換APIはローカルの http URLだけ指定できます"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "外部VOICEVOX互換APIのURLに認証情報・クエリ・フラグメントは指定できません"
    );
    let host = url.host_str().unwrap_or_default();
    anyhow::ensure!(
        matches!(host, "127.0.0.1" | "localhost" | "::1"),
        "外部VOICEVOX互換APIはこのPC上のURLだけ指定できます"
    );
    Ok(())
}

#[cfg(windows)]
fn managed_user_profile_paths(data_dir: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    // AivisSpeech Engine は Windows の利用者プロファイルから保存先を決める。
    // APPDATA だけでは Known Folder の参照先が変わらず、手動導入した Aivis と
    // Models・辞書・キャッシュを共有していた。専用プロファイル4変数を同時に渡し、
    // すべての実データを PicoAgent 管理下へ閉じ込める。
    let profile = crate::app_paths::path_at(data_dir, crate::app_paths::DataPath::AivisProfile);
    let roaming = profile.join("AppData").join("Roaming");
    let local = profile.join("AppData").join("Local");
    let aivis_data = roaming.join("AivisSpeech-Engine");
    (profile, roaming, local, aivis_data)
}

#[cfg(windows)]
fn configure_managed_user_profile(
    command: &mut std::process::Command,
    data_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let (profile, roaming, local, aivis_data) = managed_user_profile_paths(data_dir);
    std::fs::create_dir_all(&roaming)?;
    std::fs::create_dir_all(&local)?;
    command.env("USERPROFILE", &profile);
    command.env("HOME", &profile);
    command.env("APPDATA", &roaming);
    command.env("LOCALAPPDATA", &local);
    Ok(aivis_data)
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
        let body = crate::tools::http::response_text(response)
            .await
            .map_err(anyhow::Error::msg)?;
        anyhow::bail!(
            "{}",
            crate::tools::http::service_error(
                "読み上げエンジン（AivisSpeech）",
                "音声モデルの導入",
                status,
                &body
            )
        );
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
        "音声モデルを導入しましたが、読み上げエンジンの音声一覧に現れませんでした。\
         設定の「追加機能」から「公式配布から再導入」を試してください（内部ID: {speaker_uuid}）。"
    );
}

/// `/speakers` を取得する。
async fn fetch_speakers(endpoint: &str) -> anyhow::Result<Vec<Value>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;
    let response = client
        .get(format!("{endpoint}/speakers"))
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    if !response.status().is_success() {
        anyhow::bail!(
            "読み上げエンジン（AivisSpeech）が音声一覧を返しませんでした（HTTP {}）。設定の「追加機能」から再導入を試してください。",
            response.status()
        );
    }
    let speakers: Value = response.json().await.map_err(reqwest::Error::without_url)?;
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
/// 指定スタイルが無ければ失敗する。未指定のときだけ先頭を使う。
pub async fn resolve_style_id(
    endpoint: &str,
    speaker_uuid: &str,
    style_name: Option<&str>,
) -> anyhow::Result<String> {
    let speakers = fetch_speakers(endpoint).await?;
    resolve_style_id_in(&speakers, speaker_uuid, style_name)
}

fn resolve_style_id_in(
    speakers: &[Value],
    speaker_uuid: &str,
    style_name: Option<&str>,
) -> anyhow::Result<String> {
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
    let style = match style_name {
        Some(name) => styles.iter().find(|style| style.get("name").and_then(Value::as_str) == Some(name))
            .ok_or_else(|| anyhow::anyhow!("speaker_uuid={speaker_uuid} に指定スタイル「{name}」がありません。利用可能: {}",
                styles.iter().filter_map(|style| style.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join("、")))?,
        None => styles.first().ok_or_else(|| anyhow::anyhow!("speaker_uuid={speaker_uuid} に利用可能なstyleがありません。"))?,
    };
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

fn validate_runtime_files(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        path.is_file(),
        "Aivis の実行ファイルがありません: {}",
        path.display()
    );
    let root = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Aivis の配置先を取得できません: {}", path.display()))?;
    anyhow::ensure!(
        root.join("engine_manifest.json").is_file(),
        "Aivis の管理情報がありません: {}",
        root.display()
    );
    #[cfg(windows)]
    {
        use anyhow::Context as _;
        let internal = root.join("engine_internal");
        // `run.exe` だけが残った配置ではこのフォルダ自体が無い。OSエラー文だけでは
        // どの配置が欠けているか分からないため、パス付きの理由にする。
        anyhow::ensure!(
            internal.is_dir(),
            "Aivis の内部フォルダがありません: {}",
            internal.display()
        );
        let python_dll = std::fs::read_dir(&internal)
            .with_context(|| format!("Aivis の内部フォルダを読めません: {}", internal.display()))?
            .filter_map(Result::ok)
            .any(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.strip_prefix("python")
                    .and_then(|tail| tail.strip_suffix(".dll"))
                    .is_some_and(|version| {
                        version.len() >= 2 && version.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    && entry.path().is_file()
            });
        anyhow::ensure!(
            python_dll,
            "Aivis の Python DLL がありません: {}",
            internal.display()
        );
    }
    Ok(())
}

/// 導入先に Aivis があれば完全性を確かめて返す。無ければ未導入で、別の場所は探さない。
fn find_runtime(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    validate_runtime_files(path)?;
    Ok(Some(path.to_path_buf()))
}

fn runtime_engine_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "run.exe" } else { "run" };
    // 導入先は `bundled/aivis` だけ。旧配置（利用者データ直下の `aivis/`）は探さない。
    match find_runtime(&crate::app_paths::bundled_dir(app).join("aivis").join(exe)) {
        Ok(path) => path,
        Err(error) => {
            append_log_best_effort(
                app,
                &format!("不完全な Aivis を使用せず、入れ直します: {error}"),
            );
            None
        }
    }
}

/// このビルドに同梱した Aivis の唯一の置き場所。作業フォルダや旧配置は探さない。
fn resource_candidates(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let exe = if cfg!(windows) { "run.exe" } else { "run" };
    crate::app_paths::packaged_bundled_dir(app)
        .map(|packaged| vec![packaged.join("aivis").join(exe)])
        .unwrap_or_default()
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
    "読み上げエンジン（AivisSpeech）が見つかりません。設定の「追加機能」から\
     「音声読み上げ」を導入してください（開発環境では `cargo xtask setup aivis` でも用意できます）。"
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

fn append_log_best_effort(app: &tauri::AppHandle, message: &str) {
    if let Err(error) = append_log(app, message) {
        crate::log_paths::append_app_log(
            app,
            &format!("aivis.logへの書き込みに失敗しました: {error}; 元メッセージ: {message}"),
        );
    }
}

fn log_tail(app: &tauri::AppHandle) -> String {
    let path = log_path(app);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return format!("ログ: {}（読み取り失敗: {error}）", path.display());
        }
    };
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
    Ok(crate::app_paths::bundled_dir(app).join("aivis"))
}

fn release_asset_pattern() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "AivisSpeech-Engine-Windows-x64-",
        ("macos", "aarch64") => "AivisSpeech-Engine-macOS-arm64-",
        ("macos", _) => "AivisSpeech-Engine-macOS-x64-",
        _ => "",
    }
}

#[derive(Clone)]
struct ReleasePart {
    name: String,
    url: String,
    size: u64,
    sha256: String,
}

fn release_parts(data: &Value, prefix: &str) -> anyhow::Result<Vec<ReleasePart>> {
    let assets = data
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Aivis配布物一覧がありません"))?;
    let first: Vec<_> = assets
        .iter()
        .filter_map(|asset| asset.get("name").and_then(Value::as_str))
        .filter(|name| name.starts_with(prefix) && name.ends_with(".7z.001"))
        .collect();
    anyhow::ensure!(first.len() == 1, "Aivis配布物の先頭を一意に確定できません");
    let stem = first[0]
        .strip_suffix("001")
        .ok_or_else(|| anyhow::anyhow!("分割名が不正です"))?;
    let mut indexed = Vec::new();
    for asset in assets {
        let Some(name) = asset.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(stem) else {
            continue;
        };
        anyhow::ensure!(
            !name.contains(['/', '\\', ':']) && !name.chars().any(char::is_control),
            "Aivis配布名が不正です"
        );
        anyhow::ensure!(
            suffix.len() == 3 && suffix.bytes().all(|x| x.is_ascii_digit()),
            "Aivis分割番号が不正です"
        );
        let index: usize = suffix.parse()?;
        let size = asset
            .get("size")
            .and_then(Value::as_u64)
            .filter(|x| *x > 0 && *x <= 2 * 1024 * 1024 * 1024)
            .ok_or_else(|| anyhow::anyhow!("Aivis分割サイズが不正です"))?;
        let hash = asset
            .get("digest")
            .and_then(Value::as_str)
            .and_then(|x| x.strip_prefix("sha256:"))
            .filter(|x| x.len() == 64 && x.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| anyhow::anyhow!("Aivis公式配布のSHA-256がありません"))?;
        indexed.push((
            index,
            ReleasePart {
                name: name.to_owned(),
                url: asset
                    .get("browser_download_url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Aivis取得URLがありません"))?
                    .to_owned(),
                size,
                sha256: hash.to_ascii_lowercase(),
            },
        ));
    }
    indexed.sort_by_key(|x| x.0);
    anyhow::ensure!(
        !indexed.is_empty()
            && indexed.len() <= 16
            && indexed.iter().enumerate().all(|(i, x)| x.0 == i + 1),
        "Aivis分割配布物が欠損または重複しています"
    );
    anyhow::ensure!(
        indexed
            .iter()
            .try_fold(0_u64, |sum, x| sum.checked_add(x.1.size))
            .is_some_and(|x| x <= 8_000_000_000),
        "Aivis配布物の総量が上限を超えています"
    );
    Ok(indexed.into_iter().map(|x| x.1).collect())
}

async fn aivis_release_parts(target_version: &str) -> anyhow::Result<Vec<ReleasePart>> {
    let prefix = release_asset_pattern();
    anyhow::ensure!(
        !prefix.is_empty(),
        "このOS/CPU向けのAivis配布物を自動選択できません"
    );
    let url = format!("https://api.github.com/repos/Aivis-Project/AivisSpeech-Engine/releases/tags/{target_version}");
    let data: Value = crate::installer::http::client()
        .map_err(anyhow::Error::msg)?
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    release_parts(&data, prefix)
}

fn aivis_limits() -> crate::installer::Limits {
    crate::installer::Limits {
        max_entries: 20_000,
        max_total_bytes: 8_000_000_000,
        max_file_bytes: 4_000_000_000,
        file_limit: None,
    }
}

fn extraction_tool() -> anyhow::Result<(PathBuf, bool)> {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot")
            .ok_or_else(|| anyhow::anyhow!("Windowsシステムディレクトリを取得できません"))?;
        let path = PathBuf::from(root).join("System32/tar.exe");
        anyhow::ensure!(
            path.is_file(),
            "Windows標準のtar.exeがありません。Aivisの取得は開始していません"
        );
        Ok((path, true))
    }
    #[cfg(not(windows))]
    {
        // フォールバック許可: 取得前に7z/7zaの一つを確定し、失敗後に切り替えない（根拠: PR #472の利用者決定）。
        let path = ["7z", "7za"]
            .into_iter()
            .find_map(crate::tools::process::resolve_executable)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Aivis展開用7-Zipがありません。p7zipを導入してください。取得は開始していません"
                )
            })?;
        Ok((path, false))
    }
}

fn parse_7z_entries(text: &str) -> anyhow::Result<Vec<(String, bool, u64)>> {
    let mut entries = Vec::new();
    let mut fields = std::collections::HashMap::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if fields.is_empty() {
                continue;
            }
            let path: &str = fields
                .get("Path")
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Aivis一覧にPathがありません"))?;
            let size: u64 = fields
                .get("Size")
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Aivis一覧にSizeがありません"))?
                .parse()?;
            let attributes = fields
                .get("Attributes")
                .copied()
                .ok_or_else(|| anyhow::anyhow!("Aivis一覧に属性がありません"))?;
            let directory = fields
                .get("Folder")
                .copied()
                .map(|value| value == "+")
                .unwrap_or_else(|| attributes.starts_with('D'));
            anyhow::ensure!(
                !fields.contains_key("Symbolic Link") && !fields.contains_key("Hard Link"),
                "Aivis書庫にリンクがあります"
            );
            for token in attributes.split_whitespace() {
                if token.len() >= 10 && token.chars().all(|c| "-drwxstSTlbcps".contains(c)) {
                    anyhow::ensure!(
                        token.starts_with('-') || token.starts_with('d'),
                        "Aivis書庫にリンクまたは特殊ファイルがあります"
                    );
                }
            }
            anyhow::ensure!(!directory || size == 0, "Aivisフォルダのサイズが不正です");
            entries.push((path.to_owned(), directory, size));
            fields.clear();
            continue;
        }
        let (key, value) = line
            .split_once(" = ")
            .ok_or_else(|| anyhow::anyhow!("Aivis一覧に読めない続行行があります"))?;
        anyhow::ensure!(
            !value.chars().any(char::is_control)
                && [
                    "Path",
                    "Size",
                    "Packed Size",
                    "Modified",
                    "Created",
                    "Accessed",
                    "Attributes",
                    "Encrypted",
                    "Comment",
                    "CRC",
                    "Method",
                    "Characteristics",
                    "Host OS",
                    "Version",
                    "Volume Index",
                    "Offset",
                    "Block",
                    "Folder",
                    "Symbolic Link",
                    "Hard Link"
                ]
                .contains(&key),
            "Aivis一覧の項目が不正です"
        );
        anyhow::ensure!(
            fields.insert(key, value).is_none(),
            "Aivis一覧の項目が重複しています"
        );
    }
    anyhow::ensure!(!entries.is_empty(), "Aivis書庫が空です");
    crate::installer::archive::validate_external_entries(&entries, aivis_limits())?;
    Ok(entries)
}

async fn extraction_output(
    tool: &Path,
    args: &[&std::ffi::OsStr],
) -> anyhow::Result<std::process::Output> {
    let mut command = crate::tools::process::command(tool);
    command.args(args);
    let output = crate::tools::process::output(command, 900, "Aivis展開")
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        output.status.success(),
        "Aivis展開ツールが失敗しました: {}",
        String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(2000)
            .collect::<String>()
    );
    anyhow::ensure!(
        output.stdout.len() < 1024 * 1024,
        "Aivis配布物の一覧が検査上限を超えています"
    );
    Ok(output)
}

async fn extract_archive(
    archive: &Path,
    out_dir: &Path,
    tool: &Path,
    tar: bool,
) -> anyhow::Result<()> {
    use std::ffi::OsStr;
    if tar {
        let listing = extraction_output(tool, &[OsStr::new("-tf"), archive.as_os_str()]).await?;
        let names = std::str::from_utf8(&listing.stdout)?
            .lines()
            .collect::<Vec<_>>();
        let listing = extraction_output(tool, &[OsStr::new("-tvf"), archive.as_os_str()]).await?;
        let verbose = std::str::from_utf8(&listing.stdout)?
            .lines()
            .collect::<Vec<_>>();
        anyhow::ensure!(
            verbose.len() == names.len() && !names.is_empty(),
            "Aivis一覧の照合に失敗しました"
        );
        let mut entries = Vec::new();
        for (name, line) in names.into_iter().zip(verbose) {
            anyhow::ensure!(
                line.starts_with('-') || line.starts_with('d'),
                "Aivis書庫のリンクや特殊ファイルは展開できません"
            );
            let size: u64 = line
                .split_whitespace()
                .nth(4)
                .ok_or_else(|| anyhow::anyhow!("Aivis一覧のサイズがありません"))?
                .parse()?;
            entries.push((name.to_owned(), line.starts_with('d'), size));
        }
        crate::installer::archive::validate_external_entries(&entries, aivis_limits())?;
        std::fs::create_dir_all(out_dir)?;
        extraction_output(
            tool,
            &[
                OsStr::new("-xf"),
                archive.as_os_str(),
                OsStr::new("-C"),
                out_dir.as_os_str(),
            ],
        )
        .await?;
    } else {
        let listing = extraction_output(
            tool,
            &[
                OsStr::new("l"),
                OsStr::new("-slt"),
                OsStr::new("-ba"),
                archive.as_os_str(),
            ],
        )
        .await?;
        parse_7z_entries(std::str::from_utf8(&listing.stdout)?)?;
        std::fs::create_dir_all(out_dir)?;
        let output_arg = format!("-o{}", out_dir.display());
        extraction_output(
            tool,
            &[
                OsStr::new("x"),
                archive.as_os_str(),
                OsStr::new(&output_arg),
                OsStr::new("-y"),
            ],
        )
        .await?;
    }
    Ok(())
}

fn find_run_file(dir: &Path) -> anyhow::Result<Option<PathBuf>> {
    crate::installer::fs::find_file(dir, if cfg!(windows) { "run.exe" } else { "run" }, 32)
}
fn copy_dir_all(src: &Path, dest: &Path) -> anyhow::Result<()> {
    crate::installer::fs::copy_dir(src, dest, aivis_limits())
}

fn install_stage_path(install: &Path) -> PathBuf {
    let nonce = AIVIS_INSTALL_NONCE.fetch_add(1, Ordering::Relaxed);
    install.with_file_name(format!(".installing-aivis-{}-{nonce}", std::process::id()))
}

fn replace_install_tree(
    app: &tauri::AppHandle,
    stage: &Path,
    install: &Path,
) -> anyhow::Result<()> {
    crate::installer::swap::complete(
        app,
        crate::installer::swap::replace(stage, install, "aivis"),
    )
}

/// 前回の入れ替えが途中で落ちた残り（`.installing-aivis-<pid>-<n>` / `.backup-aivis-<pid>-<n>`）を片付ける。
///
/// 名前にプロセスIDを含むため、次の起動は同じ名前を見ない。拾わないと約0.8GBの旧版が残り続け、
/// 配置前に落ちた場合は Aivis が消えたように見えて取り直しになる。PicoAgent は同時に1つしか
/// 動かないので、別プロセスIDのものは終了済みとして扱える。
fn recover_interrupted_install(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let install = install_dir(app)?;
    let parent = install
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Aivis配置の親がありません"))?;
    if !parent.try_exists()? {
        return Ok(());
    }
    std::fs::read_dir(parent)?;
    let _guard = match AIVIS_INSTALL_FS_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    for (name, action) in interrupted_install_actions(parent, &install, std::process::id())? {
        let suffix = name
            .strip_prefix(".installing-aivis-")
            .or_else(|| name.strip_prefix(".backup-aivis-"))
            .ok_or_else(|| anyhow::anyhow!("Aivis復旧名が不正です"))?;
        let pid: u32 = suffix
            .split('-')
            .next()
            .ok_or_else(|| anyhow::anyhow!("Aivis復旧PIDがありません"))?
            .parse()?;
        if crate::installer::swap::process_alive(pid)? {
            continue;
        }
        let path = parent.join(&name);
        crate::installer::fs::checked_directory(&path)?;
        let (action, result) = apply_interrupted_install_action(&path, &install, action);
        let verb = match action {
            InterruptedAction::Remove => "片付けました",
            InterruptedAction::Restore => "元の場所へ戻しました",
        };
        match result {
            Ok(()) => append_log_best_effort(
                app,
                &format!("中断した Aivis の入れ替えの残りを{verb}: {name}"),
            ),
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "中断したAivis配置を復旧できません: {name} ({error})"
                ))
            }
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum InterruptedAction {
    Remove,
    Restore,
}

fn apply_interrupted_install_action(
    path: &Path,
    install: &Path,
    planned: InterruptedAction,
) -> (InterruptedAction, std::io::Result<()>) {
    // 先行候補を戻せたかは実際の配置で判定する。復元失敗を成功と見なして
    // 後続の退避先まで消すと、利用可能な旧版を失ってしまう。
    let action = if planned == InterruptedAction::Restore && !install.exists() {
        InterruptedAction::Restore
    } else {
        InterruptedAction::Remove
    };
    let result = match action {
        InterruptedAction::Remove => std::fs::remove_dir_all(path),
        InterruptedAction::Restore => std::fs::rename(path, install),
    };
    (action, result)
}

/// 片付け対象と処理を決める（ファイル操作をしない純粋な判定）。
/// 本体が無ければ全退避先を復元候補にし、実際に戻せてから後続を消す。
fn interrupted_install_actions(
    parent: &Path,
    install: &Path,
    current_pid: u32,
) -> anyhow::Result<Vec<(String, InterruptedAction)>> {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let entries = entries.collect::<std::io::Result<Vec<_>>>()?;
    for entry in &entries {
        entry.file_type()?;
    }
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| {
            let Some(suffix) = name
                .strip_prefix(".installing-aivis-")
                .or_else(|| name.strip_prefix(".backup-aivis-"))
            else {
                return false;
            };
            let Some((pid, nonce)) = suffix.split_once('-') else {
                return false;
            };
            pid.parse::<u32>().is_ok_and(|pid| pid != current_pid) && nonce.parse::<u64>().is_ok()
        })
        .collect();
    names.sort();
    let installed = install.exists();
    Ok(names
        .into_iter()
        .map(|name| {
            if name.starts_with(".backup-aivis-") && !installed {
                (name, InterruptedAction::Restore)
            } else {
                (name, InterruptedAction::Remove)
            }
        })
        .collect())
}

fn stage_and_replace(app: &tauri::AppHandle, source: &Path, install: &Path) -> anyhow::Result<()> {
    let stage = install_stage_path(install);
    if stage.exists() {
        std::fs::remove_dir_all(&stage)?;
    }
    let _cleanup = AivisInstallCleanup {
        app: app.clone(),
        path: stage.clone(),
    };
    copy_dir_all(source, &stage)?;
    let run = find_run_file(&stage)?
        .ok_or_else(|| anyhow::anyhow!("準備した Aivis に run 実行ファイルが見つかりません。"))?;
    validate_runtime_files(&run)?;
    crate::installer::fs::prepare_executable(app, &run)?;
    replace_install_tree(app, &stage, install)
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
    stage_and_replace(app, &resource_root, &install)?;
    let Some(path) = runtime_engine_path(app) else {
        anyhow::bail!("Aivis 同梱リソースをコピーしましたが、実行ファイルを見つけられません。");
    };
    crate::installer::fs::prepare_executable(app, &path)?;
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

async fn ensure_installed_inner(
    app: &tauri::AppHandle,
    force_repair: bool,
) -> anyhow::Result<PathBuf> {
    let path = ensure_installed_ready(app, force_repair).await?;
    // 新しい配置が使える状態になった後でだけ、旧配置を片付ける。
    crate::log_paths::remove_legacy_layout_dir(app, "aivis");
    Ok(path)
}

async fn ensure_installed_ready(
    app: &tauri::AppHandle,
    force_repair: bool,
) -> anyhow::Result<PathBuf> {
    crate::installer::swap::recover(&crate::app_paths::bundled_dir(app))?;
    recover_interrupted_install(app)?;
    if !force_repair {
        if let Some(path) = runtime_engine_path(app) {
            if let Some(resource) = resource_engine_path(app) {
                if resource_is_newer(&resource, &path) {
                    if let Some(copied) = copy_resource_to_install(app)? {
                        return Ok(copied);
                    }
                }
            }
            crate::installer::fs::prepare_executable(app, &path)?;
            return Ok(path);
        }
    }
    if let Some(path) = copy_resource_to_install(app)? {
        crate::installer::fs::prepare_executable(app, &path)?;
        return Ok(path);
    }
    append_log(
        app,
        "Aivis が見つからないため、公式リリースから自動取得します。",
    )?;
    let target_version = crate::components::managed_target_version(app, "aivis")?;
    let (tool, tar) = extraction_tool()?;
    let parts = aivis_release_parts(&target_version).await?;
    let nonce = AIVIS_INSTALL_NONCE.fetch_add(1, Ordering::Relaxed);
    let work = crate::app_paths::bundled_dir(app).join(format!(
        "picoagent-aivis-runtime-{}-{nonce}",
        std::process::id()
    ));
    if let Err(error) = tokio::fs::remove_dir_all(&work).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            crate::log_paths::append_app_log(
                app,
                &format!("古いAivis導入一時フォルダを削除できません: {error}"),
            );
        }
    }
    tokio::fs::create_dir_all(&work).await?;
    let _cleanup = AivisInstallCleanup {
        app: app.clone(),
        path: work.clone(),
    };
    let archive = work.join("verified-runtime.7z");
    for part in &parts {
        let path = work.join(&part.name);
        append_log(app, &format!("Aivis ダウンロード開始: {}", part.name))?;
        crate::installer::download::fetch_fixed(
            crate::installer::download::FetchSpec {
                url: &part.url,
                destination: &path,
                expected_size: Some(part.size),
                max_size: part.size,
                sha256: Some(&part.sha256),
            },
            &mut |_, _| {},
        )
        .await?;
    }
    // 検査済みの分割を順番通りに連結する。Windows標準tarは分割先頭単体を読めない。
    let mut joined = std::fs::File::create(&archive)?;
    for part in &parts {
        std::io::copy(
            &mut std::fs::File::open(work.join(&part.name))?,
            &mut joined,
        )?;
    }
    joined.flush()?;
    drop(joined);
    append_log(app, "Aivis ダウンロード完了。展開します。")?;
    let extract_dir = work.join("extract");
    extract_archive(&archive, &extract_dir, &tool, tar).await?;
    let run_path = find_run_file(&extract_dir)?
        .ok_or_else(|| anyhow::anyhow!("展開したAivisに実行ファイルがありません"))?;
    let root = run_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Aivis 実行ファイルの親ディレクトリを取得できません。"))?
        .to_path_buf();
    let install = install_dir(app)?;
    let root_for_copy = root.clone();
    let install_for_copy = install.clone();
    let app_for_copy = app.clone();
    tokio::task::spawn_blocking(move || {
        stage_and_replace(&app_for_copy, &root_for_copy, &install_for_copy)
    })
    .await??;
    let installed = runtime_engine_path(app).ok_or_else(|| {
        anyhow::anyhow!("Aivis を配置しましたが、実行ファイルを見つけられません。")
    })?;
    crate::installer::fs::prepare_executable(app, &installed)?;
    append_log(app, &format!("Aivis 自動取得完了: {}", installed.display()))?;
    Ok(installed)
}

async fn ensure_installed(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    ensure_installed_inner(app, false).await
}

/// 追加機能画面からAivis本体を公式配布物で再導入する。
/// 読み上げ設定は変更せず、有効な場合だけ入れ替え後にエンジンを再起動する。
pub async fn install_component(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    let should_restart = app
        .try_state::<AppState>()
        .map(|state| state.config.get_bool("PICOAGENT_TTS_ENABLED", false))
        .unwrap_or(false);
    let was_running = has_running_child(app);
    if was_running {
        stop(app);
        clear_stopping(app);
    }
    let result = ensure_installed_inner(app, true).await;
    if should_restart {
        clear_stopping(app);
        if let Err(error) = ensure_started(app).await {
            append_log_best_effort(app, &format!("Aivis修復後の再起動に失敗しました: {error}"));
        }
    }
    result
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
                terminate_child(app, &mut child);
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
            terminate_child(app, &mut child);
        }
    }
    false
}

#[cfg(windows)]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    let adapters = crate::gpu::adapter_names();
    if adapters.is_empty() {
        append_log_best_effort(
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
    append_log_best_effort(
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
    append_log_best_effort(
        app,
        "GPU候補確認: このOSでは事前判定を行わず auto は GPU 起動を選びます。",
    );
    true
}

#[cfg(not(any(windows, target_os = "macos")))]
fn gpu_candidate_available(app: &tauri::AppHandle) -> bool {
    append_log_best_effort(
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
    crate::installer::fs::prepare_executable(app, &path)?;
    let work_dir = path.parent().map(Path::to_path_buf);
    let mut command = crate::tools::process::command_sync(path);
    if let Some(dir) = work_dir {
        command.current_dir(dir);
    }
    let Some(state) = app.try_state::<AppState>() else {
        anyhow::bail!("AppState を取得できません。");
    };
    let port = port(app, &state).map_err(anyhow::Error::msg)?;
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
    let managed_user_data =
        configure_managed_user_profile(&mut command, &crate::app_paths::data_dir(app))?;
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
    #[cfg(windows)]
    writeln!(log, "Aivis 利用者データ: {}", managed_user_data.display())?;
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
        terminate_child(app, &mut child);
        anyhow::bail!("AivisState を取得できないため、起動した Aivis を停止しました");
    };
    let mut slot = match state.child.lock() {
        Ok(slot) => slot,
        // ハンドルを失わないよう、poison 後も格納済み状態を引き継ぐ。
        Err(poisoned) => poisoned.into_inner(),
    };
    if slot.is_some() {
        terminate_child(app, &mut child);
        anyhow::bail!("Aivis は既に起動中です");
    }
    *slot = Some(child);
    Ok(())
}

pub fn start_background(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let Some(state) = app.try_state::<AppState>() else {
        anyhow::bail!("AppState を取得できません。");
    };
    if uses_external_endpoint(&state) {
        validate_external_endpoint(&endpoint(app, &state).map_err(anyhow::Error::msg)?)?;
        return Ok(());
    }
    port(app, &state).map_err(anyhow::Error::msg)?;
    if has_running_child(app) {
        return Ok(());
    }
    let path = match runtime_engine_path(app) {
        Some(path) => {
            if let Some(resource) = resource_engine_path(app) {
                if resource_is_newer(&resource, &path) {
                    // 更新の失敗は上位へ返す。同梱リソースが無いときだけ更新せず既存配置を使う。
                    match copy_resource_to_install(app)? {
                        Some(updated) => updated,
                        None => path,
                    }
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
pub async fn is_ready(app: &tauri::AppHandle, state: &AppState) -> Result<bool, String> {
    Ok(is_up(&endpoint(app, state)?).await)
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
    let endpoint = endpoint(app, &app_state).map_err(anyhow::Error::msg)?;
    if uses_external_endpoint(&app_state) {
        validate_external_endpoint(&endpoint)?;
        anyhow::ensure!(
            is_up(&endpoint).await,
            "明示設定した外部VOICEVOX互換APIへ接続できません: {endpoint}"
        );
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

    anyhow::ensure!(
        !is_up(&endpoint).await,
        "Aivis用ポートがPicoAgent管理外のプロセスに使われています: {endpoint}。外部サービスを使う場合はPICOAGENT_TTS_ENDPOINTへ明示設定してください"
    );

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

/// 設定変更や修復後に再起動するときだけ、終了防止フラグを戻す。
pub fn clear_stopping(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AivisState>() {
        state.stopping.store(false, Ordering::SeqCst);
    }
}

fn terminate_child(app: &tauri::AppHandle, child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let taskkill = crate::tools::process::command_sync("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(error) = taskkill {
            crate::log_paths::append_app_log(
                app,
                &format!("Aivisのプロセスツリー停止に失敗しました: {error}"),
            );
        }
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
    if let Err(error) = child.kill() {
        if error.kind() != std::io::ErrorKind::InvalidInput {
            crate::log_paths::append_app_log(app, &format!("Aivisを停止できませんでした: {error}"));
        }
    }
    if let Err(error) = child.wait() {
        crate::log_paths::append_app_log(
            app,
            &format!("Aivisの終了ハンドルを回収できませんでした: {error}"),
        );
    }
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
        terminate_child(app, &mut child);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn 分割配布の全番号とサイズとハッシュを取得前に確定する() {
        let prefix = "AivisSpeech-Engine-Windows-x64-";
        let asset = |index: usize| serde_json::json!({"name":format!("{prefix}1.0.7z.{index:03}"),"size":12,"digest":format!("sha256:{}","a".repeat(64)),"browser_download_url":"https://example.invalid/a"});
        let parts =
            super::release_parts(&serde_json::json!({"assets":[asset(2),asset(1)]}), prefix)
                .unwrap();
        assert!(parts[0].name.ends_with("001"));
        assert!(parts[1].name.ends_with("002"));
        for assets in [
            vec![asset(1), asset(3)],
            vec![asset(1), asset(1)],
            vec![
                asset(1),
                serde_json::json!({"name":format!("{prefix}1.0.7z.002"),"size":12}),
            ],
        ] {
            assert!(super::release_parts(&serde_json::json!({"assets":assets}), prefix).is_err());
        }
    }
    #[test]
    fn 展開一覧の続行やリンクと正規化後重複を拒否する() {
        let valid = "Path = runtime/file\nSize = 3\nAttributes = A\n\n";
        assert!(super::parse_7z_entries(valid).is_ok());
        for invalid in [
            "Path = runtime/file\n/../../outside\nSize = 3\nAttributes = A",
            "Path = runtime/file\nSize = 3\nAttributes = A\nSymbolic Link = outside",
            "Path = runtime/file\nSize = 3\nSize = 3\nAttributes = A",
        ] {
            assert!(super::parse_7z_entries(invalid).is_err());
        }
        assert!(super::parse_7z_entries(
            &(valid.to_owned() + "Path = runtime/./file\nSize = 3\nAttributes = A\n")
        )
        .is_err());
    }
    #[test]
    fn 音声ポート不正値は移行や既定ポート接続へ進めない() {
        let cfg = crate::config::Config::new();
        for value in ["65536", "bad", "1.5", "-1"] {
            cfg.set("PICOAGENT_TTS_PORT", value);
            let error =
                super::configured_port(&cfg, |_| panic!("不正値を移行してはいけない")).unwrap_err();
            assert!(error.contains("PICOAGENT_TTS_PORT"));
        }
        cfg.set("PICOAGENT_TTS_PORT", "12400");
        assert_eq!(
            super::configured_port(&cfg, |_| panic!("利用者のポートを移行してはいけない")).unwrap(),
            12400
        );
    }

    #[test]
    fn 旧音声ポートは保存に成功した場合だけ移行する() {
        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_TTS_PORT", "10102");
        let mut saves = 0;
        let port = super::configured_port(&cfg, |next| {
            saves += 1;
            assert_eq!(next, "11436");
            cfg.set("PICOAGENT_TTS_PORT", &next);
            Ok(())
        })
        .unwrap();
        assert_eq!(port, 11436);
        assert_eq!(saves, 1);
        assert_eq!(
            super::configured_port(&cfg, |_| panic!("移行済みは再保存しない")).unwrap(),
            11436
        );
        cfg.set("PICOAGENT_TTS_PORT", "10102");
        assert_eq!(
            super::configured_port(&cfg, |_| Err("保存先へ書き込めません".to_string()))
                .unwrap_err(),
            "保存先へ書き込めません"
        );
        assert_eq!(cfg.int(&crate::config::ints::TTS_PORT).unwrap(), 10102);
    }

    #[test]
    fn 話者の指定スタイルが無いときだけエラーで未指定は先頭を使う() {
        let speakers = serde_json::json!([{"speaker_uuid":"speaker", "styles":[{"name":"ノーマル","id":123},{"name":"おちつき","id":456}]}]);
        for (name, expected) in [
            (None, Some("123")),
            (Some("おちつき"), Some("456")),
            (Some("無いスタイル"), None),
            (Some(""), None),
        ] {
            let result = super::resolve_style_id_in(speakers.as_array().unwrap(), "speaker", name);
            if let Some(expected) = expected {
                assert_eq!(result.unwrap(), expected);
            } else {
                let error = result.unwrap_err().to_string();
                assert!(error.contains("ノーマル") && error.contains("おちつき"));
            }
        }
        assert!(super::resolve_style_id_in(&[], "speaker", None).is_err());
    }

    use super::{interrupted_install_actions, validate_external_endpoint, InterruptedAction};
    // 実行ファイルと管理情報の判定は全OS共通。Python DLLだけWindows版の配置を検査する。
    use super::find_runtime;
    #[cfg(windows)]
    use super::validate_runtime_files;

    #[cfg(not(windows))]
    #[test]
    fn 管理情報のないaivisは不完全と判定し未導入とも区別する() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        // 導入先に無ければ未導入（別の場所は探さない）。
        assert!(find_runtime(&run).unwrap().is_none());
        std::fs::write(&run, []).unwrap();
        let missing = find_runtime(&run).unwrap_err().to_string();
        assert!(missing.contains("管理情報"), "{missing}");
        std::fs::write(dir.path().join("engine_manifest.json"), b"{}").unwrap();
        assert_eq!(find_runtime(&run).unwrap(), Some(run));
    }

    #[cfg(windows)]
    #[test]
    fn 実行ファイルだけ残ったaivisを不完全と判定する() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run.exe");
        std::fs::write(&run, []).unwrap();
        std::fs::write(dir.path().join("engine_manifest.json"), b"{}").unwrap();
        let internal = dir.path().join("engine_internal");
        let missing = validate_runtime_files(&run).unwrap_err().to_string();
        assert!(missing.contains("engine_internal"), "{missing}");
        std::fs::create_dir_all(&internal).unwrap();
        std::fs::write(internal.join("python3.dll"), []).unwrap();
        assert!(validate_runtime_files(&run).is_err());
        std::fs::write(internal.join("python311.dll"), []).unwrap();
        assert!(validate_runtime_files(&run).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn 導入先のaivisだけを確認し不完全なら使わない() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run.exe");
        assert!(find_runtime(&run).unwrap().is_none());
        std::fs::create_dir_all(dir.path().join("engine_internal")).unwrap();
        std::fs::write(&run, []).unwrap();
        std::fs::write(dir.path().join("engine_manifest.json"), b"{}").unwrap();
        std::fs::write(dir.path().join("engine_internal/python311.dll"), []).unwrap();
        assert_eq!(find_runtime(&run).unwrap(), Some(run.clone()));
        std::fs::remove_file(dir.path().join("engine_internal/python311.dll")).unwrap();
        assert!(find_runtime(&run).is_err());
    }

    /// 前回の起動で入れ替え中に落ちた残りを、本体の有無に応じて戻すか消す。
    #[test]
    fn 中断した入れ替えは本体が無ければ旧版を戻し残りを消す() {
        let parent = std::env::temp_dir().join(format!(
            "picoagent-aivis-recover-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let install = parent.join("aivis");
        for name in [
            ".backup-aivis-111-0",
            ".installing-aivis-111-1",
            ".backup-aivis-999-0",
        ] {
            std::fs::create_dir_all(parent.join(name)).unwrap();
        }

        let actions = interrupted_install_actions(&parent, &install, 999).unwrap();
        assert_eq!(
            actions,
            vec![
                (
                    ".backup-aivis-111-0".to_string(),
                    InterruptedAction::Restore
                ),
                (
                    ".installing-aivis-111-1".to_string(),
                    InterruptedAction::Remove
                ),
            ],
            "この起動（pid 999）の入れ替え中のものには触らない"
        );

        std::fs::create_dir_all(&install).unwrap();
        let actions = interrupted_install_actions(&parent, &install, 999).unwrap();
        assert!(
            actions
                .iter()
                .all(|(_, action)| *action == InterruptedAction::Remove),
            "本体があるなら旧版で上書きしない: {actions:?}"
        );
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn 復元失敗時は後続の退避先を削除せず成功後だけ片付ける() {
        let parent = std::env::temp_dir().join(format!(
            "picoagent-aivis-recover-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&parent).unwrap();
        let blocked = parent.join("blocked");
        std::fs::write(&blocked, "復元先の親を作れない状態").unwrap();
        let install = blocked.join("aivis");
        let names = [".backup-aivis-111-0", ".backup-aivis-222-0"];
        for name in names {
            std::fs::create_dir_all(parent.join(name)).unwrap();
        }
        let actions = interrupted_install_actions(&parent, &install, 999).unwrap();
        assert_eq!(actions.len(), 2);
        for (name, planned) in &actions {
            let path = parent.join(name);
            let (action, result) =
                super::apply_interrupted_install_action(&path, &install, *planned);
            assert_eq!(action, InterruptedAction::Restore);
            assert!(result.is_err());
            assert!(path.is_dir(), "復元できない退避先を削除してはいけません");
        }
        std::fs::remove_file(&blocked).unwrap();
        std::fs::create_dir(&blocked).unwrap();
        for (index, (name, planned)) in actions.iter().enumerate() {
            let (action, result) =
                super::apply_interrupted_install_action(&parent.join(name), &install, *planned);
            result.unwrap();
            assert_eq!(
                action,
                if index == 0 {
                    InterruptedAction::Restore
                } else {
                    InterruptedAction::Remove
                }
            );
        }
        assert!(install.is_dir());
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn aivisの途中名は数値のpidと連番が揃うものだけを処理する() {
        let parent = std::env::temp_dir().join(format!(
            "picoagent-aivis-recover-names-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for name in [
            ".backup-aivis-user-data",
            ".installing-aivis-111",
            ".backup-aivis-111-2-extra",
            ".backup-aivis-999-1",
            ".backup-aivis-111-999",
        ] {
            std::fs::create_dir_all(parent.join(name)).unwrap();
        }
        assert_eq!(
            interrupted_install_actions(&parent, &parent.join("aivis"), 999).unwrap(),
            vec![(
                ".backup-aivis-111-999".to_string(),
                InterruptedAction::Restore
            )]
        );
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn 外部音声apiはローカルhttpだけを許可する() {
        assert!(validate_external_endpoint("http://127.0.0.1:10101").is_ok());
        assert!(validate_external_endpoint("http://localhost:50021").is_ok());
        assert!(validate_external_endpoint("https://127.0.0.1:10101").is_err());
        assert!(validate_external_endpoint("http://example.com:10101").is_err());
        assert!(validate_external_endpoint("http://user:pass@localhost:10101").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn aivisの利用者データはpicoagent専用プロファイルへ固定する() {
        let root = std::path::PathBuf::from(r"C:\PicoAgent");
        let (_profile, roaming, local, actual) = super::managed_user_profile_paths(&root);
        let profile = root.join("aivis-userdata").join("profile");
        assert_eq!(roaming, profile.join("AppData/Roaming"));
        assert_eq!(local, profile.join("AppData/Local"));
        assert_eq!(actual, profile.join("AppData/Roaming/AivisSpeech-Engine"));
    }

    #[test]
    fn aivis入れ替え失敗時は旧版を復元する() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-aivis-replace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let install = root.join("aivis");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("version.txt"), b"old").unwrap();

        assert!(
            crate::installer::swap::replace(&root.join("missing-stage"), &install, "aivis")
                .is_err()
        );
        assert_eq!(std::fs::read(install.join("version.txt")).unwrap(), b"old");

        let stage = root.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("version.txt"), b"new").unwrap();
        crate::installer::swap::replace(&stage, &install, "aivis").unwrap();
        assert_eq!(std::fs::read(install.join("version.txt")).unwrap(), b"new");
        std::fs::remove_dir_all(root).unwrap();
    }
}
