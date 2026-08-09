// tha3_creator.rs - 設定画面からTHA3自作キャラクターを生成してユーザー領域へ導入する。

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::oneshot;

use crate::{character_pack, permissions, store, tools, user_characters};

const GENERATOR_SCRIPT: &str = include_str!("../../scripts/build-tha3-layers.py");
const ANIMATION_SCRIPT: &str = include_str!("../../scripts/build-character-animations.py");
const MODEL_FILES: [&str; 5] = [
    "editor.pt",
    "eyebrow_decomposer.pt",
    "eyebrow_morphing_combiner.pt",
    "face_morpher.pt",
    "two_algo_face_body_rotator.pt",
];
const THA3_ROOT_KEY: &str = "PICOAGENT_THA3_ROOT";
const REQUIRED_MODELS: [&str; 3] = ["standard_float", "standard_half", "separable_half"];

pub struct Tha3CreatorState {
    control: Mutex<GenerationControl>,
    finished: Condvar,
}

#[derive(Default)]
struct GenerationControl {
    running: bool,
    cancel: Option<oneshot::Sender<()>>,
}

impl Tha3CreatorState {
    pub fn new() -> Self {
        Self {
            control: Mutex::new(GenerationControl::default()),
            finished: Condvar::new(),
        }
    }
}

impl Drop for Tha3CreatorState {
    fn drop(&mut self) {
        if let Ok(control) = self.control.get_mut() {
            if let Some(sender) = control.cancel.take() {
                let _ = sender.send(());
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tha3Status {
    pub ready: bool,
    pub source_ready: bool,
    pub model_ready: bool,
    pub runtime_ready: bool,
    pub root: String,
    pub install_hint: String,
}

fn development_tha3_roots() -> Vec<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            starts.push(parent.to_path_buf());
        }
    }
    if let Ok(current) = std::env::current_dir() {
        starts.push(current);
    }
    let mut roots = Vec::new();
    for start in starts {
        for ancestor in start.ancestors() {
            for relative in [
                "src-tauri/resources/bundled/tha3",
                "resources/bundled/tha3",
                "tools/tha3",
            ] {
                let candidate = ancestor.join(relative);
                if source_ready(&candidate) && !roots.contains(&candidate) {
                    roots.push(candidate);
                }
            }
        }
    }
    roots
}

fn candidate_roots(app: &AppHandle) -> Result<Vec<PathBuf>> {
    let app_data = app
        .path()
        .app_data_dir()
        .context("アプリデータフォルダを取得できません")?;
    let mut roots = Vec::new();
    roots.push(crate::log_paths::bundled_dir(app).join("tha3"));
    if let Ok(resources) = app.path().resource_dir() {
        roots.push(resources.join("resources/bundled/tha3"));
    }
    roots.extend(development_tha3_roots());
    // フォールバック許可: 旧版で手動配置した利用者データを移行期間中だけ探索する。
    roots.push(app_data.join("tools/tha3"));
    roots.dedup();
    Ok(roots)
}

fn source_ready(root: &Path) -> bool {
    root.join("tha3/poser/modes/load_poser.py").is_file()
}

fn model_ready(root: &Path) -> bool {
    REQUIRED_MODELS.iter().all(|model| {
        MODEL_FILES
            .iter()
            .all(|name| root.join("data/models").join(model).join(name).is_file())
    })
}

fn find_python(dir: &Path, depth: usize) -> Option<PathBuf> {
    if depth > 6 {
        return None;
    }
    let names: &[&str] = if cfg!(windows) {
        &["python.exe"]
    } else {
        &["python3", "python3.11"]
    };
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(".temp") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("venv") {
            continue;
        }
        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| names.contains(&name))
        {
            return Some(path);
        }
        if path.is_dir() {
            directories.push(path);
        }
    }
    directories
        .into_iter()
        .find_map(|directory| find_python(&directory, depth + 1))
}

fn development_runtime_root() -> Option<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            starts.push(parent.to_path_buf());
        }
    }
    if let Ok(current) = std::env::current_dir() {
        starts.push(current);
    }
    starts.into_iter().find_map(|start| {
        start.ancestors().find_map(|ancestor| {
            let candidate = ancestor.join("tools/tha3-runtime");
            local_runtime_ready(&candidate).then_some(candidate)
        })
    })
}

fn local_python(root: &Path) -> Option<PathBuf> {
    let installed_venv = if cfg!(windows) {
        root.join("runtime/venv/Scripts/python.exe")
    } else {
        root.join("runtime/venv/bin/python")
    };
    if installed_venv.is_file() {
        return Some(installed_venv);
    }
    if let Some(python) = find_python(&root.join("runtime/python"), 0) {
        return Some(python);
    }
    let candidates = if cfg!(windows) {
        vec![
            root.join(".venv/Scripts/python.exe"),
            root.join("venv/Scripts/python.exe"),
        ]
    } else {
        vec![root.join(".venv/bin/python"), root.join("venv/bin/python")]
    };
    candidates.into_iter().find(|path| path.is_file())
}

fn python_packages_ready(packages: &Path) -> bool {
    [
        "torch/__init__.py",
        "numpy/__init__.py",
        "PIL/__init__.py",
        "matplotlib/__init__.py",
    ]
    .iter()
    .all(|name| packages.join(name).is_file())
}

fn local_runtime_ready(root: &Path) -> bool {
    let packages = root.join("runtime/packages");
    let bundled_ready =
        find_python(&root.join("runtime/python"), 0).is_some() && python_packages_ready(&packages);
    let installed_venv_packages = if cfg!(windows) {
        root.join("runtime/venv/Lib/site-packages")
    } else {
        root.join("runtime/venv/lib/python3.11/site-packages")
    };
    let installed_venv_python = if cfg!(windows) {
        root.join("runtime/venv/Scripts/python.exe")
    } else {
        root.join("runtime/venv/bin/python")
    };
    let installed_venv_ready = installed_venv_python.is_file()
        && root.join("runtime/picoagent-runtime-version.txt").is_file()
        && python_packages_ready(&installed_venv_packages);
    let manual_venv_roots = [root.join(".venv"), root.join("venv")];
    let manual_venv_ready = manual_venv_roots.iter().any(|venv| {
        let python = if cfg!(windows) {
            venv.join("Scripts/python.exe")
        } else {
            venv.join("bin/python")
        };
        let packages = if cfg!(windows) {
            venv.join("Lib/site-packages")
        } else {
            venv.join("lib/python3.11/site-packages")
        };
        python.is_file() && python_packages_ready(&packages)
    });
    bundled_ready || installed_venv_ready || manual_venv_ready
}

fn runtime_ready(root: &Path) -> bool {
    local_runtime_ready(root) || development_runtime_root().is_some()
}

fn python_command(tha3_root: &Path) -> Option<PathBuf> {
    local_python(tha3_root)
        .or_else(|| development_runtime_root().and_then(|root| local_python(&root)))
}

fn runtime_packages(tha3_root: &Path) -> Option<PathBuf> {
    for root in [Some(tha3_root.to_path_buf()), development_runtime_root()]
        .into_iter()
        .flatten()
    {
        let packages = root.join("runtime/packages");
        if packages.join("torch/__init__.py").is_file() {
            return Some(packages);
        }
        let venv_packages = if cfg!(windows) {
            root.join("runtime/venv/Lib/site-packages")
        } else {
            root.join("runtime/venv/lib/python3.11/site-packages")
        };
        if venv_packages.join("torch/__init__.py").is_file() {
            return Some(venv_packages);
        }
    }
    None
}

fn validate_tha3_root(root: PathBuf) -> Result<PathBuf> {
    if !source_ready(&root) {
        bail!("THA3本体がありません: {}", root.display());
    }
    if !model_ready(&root) {
        bail!("VRAM自動調整用モデルが不足しています: {}", root.display());
    }
    if !runtime_ready(&root) {
        bail!("THA3用Pythonランタイムが不足しています: {}", root.display());
    }
    Ok(root)
}

fn locate_tha3(app: &AppHandle, selected: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(selected) = selected {
        return validate_tha3_root(selected);
    }
    let roots = candidate_roots(app)?;
    if let Some(root) = roots
        .iter()
        .find(|root| source_ready(root) && model_ready(root) && runtime_ready(root))
    {
        return Ok(root.clone());
    }
    let expected = roots
        .first()
        .cloned()
        .context("THA3の確認先を決められません")?;
    bail!(
        "THA3本体・モデル・Pythonランタイムがありません。設定で作者配布から導入したTHA3フォルダを選んでください。開発環境では `cargo xtask setup tha3` でも準備できます（確認先: {}）。",
        expected.display()
    )
}

fn selected_root(
    config: &crate::config::Config,
    explicit: Option<&str>,
) -> Result<Option<PathBuf>> {
    let configured = explicit
        .map(str::to_owned)
        .unwrap_or_else(|| config.get(THA3_ROOT_KEY, ""));
    if configured.trim().is_empty() {
        return Ok(None);
    }
    permissions::resolve(config, &configured)
        .map_err(anyhow::Error::msg)
        .map(|path| Some(path.as_path().to_path_buf()))
}

pub fn status(
    app: &AppHandle,
    config: &crate::config::Config,
    selected: Option<&str>,
) -> Result<Tha3Status> {
    let root = if let Some(selected) = selected_root(config, selected)? {
        selected
    } else {
        let roots = candidate_roots(app)?;
        roots
            .iter()
            .find(|root| source_ready(root) && model_ready(root) && runtime_ready(root))
            .or_else(|| {
                roots
                    .iter()
                    .find(|root| source_ready(root) || model_ready(root) || runtime_ready(root))
            })
            .cloned()
            .unwrap_or_else(|| roots[0].clone())
    };
    let source = source_ready(&root);
    let model = model_ready(&root);
    let runtime = runtime_ready(&root);
    Ok(Tha3Status {
        ready: source && model && runtime,
        source_ready: source,
        model_ready: model,
        runtime_ready: runtime,
        root: root.to_string_lossy().to_string(),
        install_hint: "販売版はTHA3を同梱しません。追加機能に表示する作者配布元から本体・モデル・Python環境を用意し、この画面でTHA3フォルダを選んで保存してください。開発環境は cargo xtask setup tha3 でも準備できます。".to_string(),
    })
}

fn validate_identity(id: &str, name: &str) -> Result<()> {
    if id.is_empty()
        || id.chars().count() > character_pack::MAX_ID_CHARS
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!(
            "IDは{}文字以内の英数字・ハイフン・アンダースコアにしてください。",
            character_pack::MAX_ID_CHARS
        );
    }
    if name.is_empty() || name.chars().count() > 80 {
        bail!("表示名は1〜80文字にしてください。");
    }
    Ok(())
}

async fn collect_lines<R>(reader: R, app: AppHandle, stream: &'static str) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut captured = Vec::new();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let _ = app.emit(
                    "tha3:progress",
                    json!({ "stream": stream, "message": line }),
                );
                captured.push(line);
                if captured.len() > 200 {
                    captured.remove(0);
                }
            }
            Ok(None) => break,
            Err(error) => {
                captured.push(format!("{stream}の読込に失敗しました: {error}"));
                break;
            }
        }
    }
    captured.join("\n")
}

enum ProcessResult {
    Completed(std::process::ExitStatus),
    Cancelled(Option<String>),
    WaitFailed(String),
}

async fn stop_and_reap(child: &mut tokio::process::Child) -> Option<String> {
    let kill_error = child.kill().await.err();
    match child.wait().await {
        Ok(_) => None,
        Err(wait_error) => Some(match kill_error {
            Some(kill_error) => format!("停止に失敗: {kill_error}、回収にも失敗: {wait_error}"),
            None => format!("回収に失敗: {wait_error}"),
        }),
    }
}

struct GeneratorRequest<'a> {
    python: &'a Path,
    generator: &'a Path,
    animation: &'a Path,
    source: &'a permissions::SafePath,
    pack: &'a Path,
    temp: &'a Path,
    tha3_root: &'a Path,
    id: &'a str,
    name: &'a str,
}

async fn run_generator(
    app: &AppHandle,
    receiver: oneshot::Receiver<()>,
    request: GeneratorRequest<'_>,
) -> Result<()> {
    let mut command = tools::process::command(request.python);
    command
        .arg(request.generator)
        .arg(request.id)
        .arg("--input")
        .arg(request.source)
        .arg("--name")
        .arg(request.name)
        .arg("--pack-dir")
        .arg(request.pack)
        .arg("--tha3-dir")
        .arg(request.tha3_root)
        .arg("--temp-dir")
        .arg(request.temp)
        .arg("--animation-script")
        .arg(request.animation)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONNOUSERSITE", "1");
    if let Some(bundled_packages) = runtime_packages(request.tha3_root) {
        command.env("PYTHONPATH", bundled_packages);
    }
    let mut child = command.spawn().context("THA3用Pythonを起動できません")?;
    let Some(stdout) = child.stdout.take() else {
        let release = stop_and_reap(&mut child).await.unwrap_or_default();
        bail!("THA3の標準出力を取得できません。{release}");
    };
    let Some(stderr) = child.stderr.take() else {
        let release = stop_and_reap(&mut child).await.unwrap_or_default();
        bail!("THA3の標準エラーを取得できません。{release}");
    };
    let stdout_task = tokio::spawn(collect_lines(stdout, app.clone(), "stdout"));
    let stderr_task = tokio::spawn(collect_lines(stderr, app.clone(), "stderr"));

    let result = tokio::select! {
        status = child.wait() => match status {
            Ok(status) => ProcessResult::Completed(status),
            Err(error) => ProcessResult::WaitFailed(format!("THA3プロセスを待機できません: {error}")),
        },
        _ = receiver => {
            ProcessResult::Cancelled(stop_and_reap(&mut child).await)
        }
    };
    let stdout = stdout_task
        .await
        .unwrap_or_else(|error| format!("出力処理に失敗しました: {error}"));
    let stderr = stderr_task
        .await
        .unwrap_or_else(|error| format!("エラー出力処理に失敗しました: {error}"));
    match result {
        ProcessResult::Completed(status) if status.success() => Ok(()),
        ProcessResult::Completed(status) => {
            let detail = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            bail!(
                "THA3生成に失敗しました（終了コード {:?}）: {}",
                status.code(),
                detail.trim()
            )
        }
        ProcessResult::Cancelled(None) => bail!("THA3生成をキャンセルしました。"),
        ProcessResult::Cancelled(Some(error)) => {
            bail!("THA3生成をキャンセルしましたが、子プロセスの解放に失敗しました: {error}")
        }
        ProcessResult::WaitFailed(error) => bail!("{error}"),
    }
}

pub async fn generate(
    app: AppHandle,
    state: &Tha3CreatorState,
    config: &crate::config::Config,
    source_path: &str,
    tha3_root_path: Option<&str>,
    id: &str,
    name: &str,
) -> Result<(String, String)> {
    let id = id.trim();
    let name = name.trim();
    validate_identity(id, name)?;
    let source = permissions::resolve(config, source_path).map_err(anyhow::Error::msg)?;
    if source.extension_lossy() != "png" {
        bail!("入力画像はPNGを選んでください。");
    }
    if !source.as_path().is_file() {
        bail!("入力画像が見つかりません: {}", source.display());
    }
    if user_characters::root(&app).join(id).exists() {
        bail!("同じIDの追加キャラクターが既にあります: {id}");
    }
    let tha3_root = locate_tha3(&app, selected_root(config, tha3_root_path)?)?;
    let python = python_command(&tha3_root).context("THA3用Python実行ファイルがありません")?;

    let cache = app
        .path()
        .app_cache_dir()
        .context("アプリの一時フォルダを取得できません")?
        .join("tha3-creator")
        .join(format!("{id}-{}", std::process::id()));
    let (sender, receiver) = oneshot::channel();
    {
        let mut control = state.control.lock().unwrap();
        if control.running {
            bail!("別のキャラクターを生成中です。完了またはキャンセルを待ってください。");
        }
        control.running = true;
        control.cancel = Some(sender);
    }
    let result = async {
        if cache.exists() {
            std::fs::remove_dir_all(&cache).with_context(|| {
                format!("古い生成途中データを掃除できません: {}", cache.display())
            })?;
        }
        std::fs::create_dir_all(&cache)?;
        let generator = cache.join("build-tha3-layers.py");
        let animation = cache.join("build-character-animations.py");
        store::write_atomic(&generator, GENERATOR_SCRIPT.as_bytes())?;
        store::write_atomic(&animation, ANIMATION_SCRIPT.as_bytes())?;
        let pack = cache.join("pack");
        let temp = cache.join("temp");
        run_generator(
            &app,
            receiver,
            GeneratorRequest {
                python: &python,
                generator: &generator,
                animation: &animation,
                source: &source,
                pack: &pack,
                temp: &temp,
                tha3_root: &tha3_root,
                id,
                name,
            },
        )
        .await?;
        character_pack::validate(&pack)
            .context("生成したキャラクターパックの検証に失敗しました")?;
        user_characters::install(&app, &pack).context("生成したキャラクターを導入できません")
    }
    .await;
    {
        let mut control = state.control.lock().unwrap();
        control.running = false;
        control.cancel = None;
        state.finished.notify_all();
    }
    if cache.exists() {
        if let Err(error) = std::fs::remove_dir_all(&cache) {
            crate::log_paths::append_app_log(
                &app,
                &format!(
                    "THA3一時フォルダを掃除できません: {} ({error})",
                    cache.display()
                ),
            );
        }
    }
    result
}

pub fn cancel(state: &Tha3CreatorState) -> bool {
    state
        .control
        .lock()
        .unwrap()
        .cancel
        .take()
        .is_some_and(|sender| sender.send(()).is_ok())
}

/// アプリ終了時はキャンセルを送り、非同期側がkillとwaitを終えるまで待つ。
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<Tha3CreatorState>();
    let mut control = state.control.lock().unwrap();
    if let Some(sender) = control.cancel.take() {
        let _ = sender.send(());
    }
    let (control, timeout) = state
        .finished
        .wait_timeout_while(control, Duration::from_secs(15), |control| control.running)
        .unwrap();
    if timeout.timed_out() && control.running {
        crate::log_paths::append_app_log(
            app,
            "THA3生成プロセスの終了待機が15秒でタイムアウトしました。kill_on_dropで停止します。",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::validate_identity;

    #[test]
    fn 自作キャラクターのidと表示名を検証する() {
        assert!(validate_identity("my-character_1", "マイキャラ").is_ok());
        assert!(validate_identity("../escape", "マイキャラ").is_err());
        assert!(validate_identity("日本語", "マイキャラ").is_err());
        assert!(validate_identity("valid", "").is_err());
    }
}
