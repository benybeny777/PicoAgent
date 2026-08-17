// xtask/src/bundled.rs - 同梱物と開発用制作ツール（推論エンジン / Whisper / Live2D / THA3）の取得。
//
// 旧 `scripts/fetch-engine.mjs` / `fetch-whisper.mjs` / `fetch-live2d.mjs` の置き換え。
// 実体は巨大なのでリポジトリには含めず、各環境で取得する。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::util::{
    cleanup_dir, cleanup_file, command_exists, copy_dir, download_to, env_or, exe_name, extract,
    flag, get_json, list_files, log, make_executable, mb, repo_root, warn,
};

// ── 共通 ────────────────────────────────────────────────────────────────

/// 展開ツリーから実行ファイルと共有ライブラリを集めて `out` 直下へ集約する。
///
/// 配布アーカイブのディレクトリ階層はリリースごとに変わるため、必要なものだけ拾う。
fn flatten(root: &Path, out: &Path, exe_stem: &str) -> Result<bool> {
    fs::create_dir_all(out)?;
    let wanted_exe = exe_name(exe_stem);
    let mut found_exe = false;

    for file in list_files(root) {
        let Some(name) = file.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let is_exe = name == wanted_exe;
        let is_lib = name.ends_with(".dll")
            || name.ends_with(".dylib")
            || name.contains(".so.")
            || name.ends_with(".so");
        if !is_exe && !is_lib {
            continue;
        }
        let dest = out.join(&name);
        if file != dest {
            // rename が失敗する（別デバイス等）場合はコピーで代替する。
            if fs::rename(&file, &dest).is_err() {
                fs::copy(&file, &dest)
                    .with_context(|| format!("配置できません: {}", file.display()))?;
            }
        }
        if is_exe {
            make_executable(&dest);
            found_exe = true;
        }
    }
    Ok(found_exe)
}

/// 検証済み staging を最終配置先へ移す。
///
/// staging は一時ディレクトリ配下に作るため、TEMP がリポジトリと別ドライブ（Windows）や
/// tmpfs（Linux コンテナ）にあると `rename` が EXDEV で必ず失敗する。`flatten` と同じく
/// コピーで代替し、失敗したときは中途半端な配置先を残さない。
///
/// **配置が済んだあとの後処理失敗で `Err` を返してはいけない。** 呼び出し側は `Err` を
/// 「配置できなかった」と解釈して旧版を同じパスへ `rename` し戻すため、配置先が埋まった
/// まま `Err` を返すと Windows では復元も失敗し、旧版が `*-old-*` に取り残される
/// （AGENTS.md の「取得・更新・ビルドの正常終了後は旧版・退避を残さない」に反する）。
/// staging は一時ディレクトリ配下で、呼び出し側が作業フォルダごと掃除する。
fn place_dir(staged: &Path, dest: &Path) -> Result<()> {
    place_dir_inner(staged, dest, cleanup_dir)
}

/// `place_dir` の本体。staging の掃除だけテストから差し替えられるようにしている。
///
/// 掃除失敗を実ファイルシステムで再現する手が無い（root では権限で削除を阻めず、
/// `remove_dir_all` はシンボリックリンクを追わずに消して成功する）ため、
/// 「配置後の掃除失敗で巻き戻さない」という肝心の分岐を注入で検査する。
fn place_dir_inner(
    staged: &Path,
    dest: &Path,
    cleanup_staged: fn(&Path, &str) -> Result<()>,
) -> Result<()> {
    if fs::rename(staged, dest).is_ok() {
        return Ok(());
    }
    if let Err(error) = copy_dir(staged, dest, &|_| false) {
        cleanup_dir(dest, "配置に失敗したフォルダ")?;
        return Err(error);
    }
    if let Err(error) = cleanup_staged(staged, "配置済みstagingフォルダ") {
        warn(
            "setup",
            format!("配置済みstagingフォルダを掃除できません（配置は完了しています）: {error:#}"),
        );
    }
    Ok(())
}

/// GitHub リリースのアセットから、名前が条件に合う最初のものを選ぶ。
struct ReleaseAsset {
    name: String,
    url: String,
    size: u64,
    sha256: String,
}

fn pick_asset(release: &Value, matches: impl Fn(&str) -> bool) -> Result<ReleaseAsset> {
    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("リリース情報に assets がありません"))?;
    for asset in assets {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or("");
        if matches(name) {
            let url = asset
                .get("browser_download_url")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("アセットの URL がありません: {name}"))?;
            let size = asset
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("アセットサイズがありません: {name}"))?;
            // GitHubがdigestを返すのは比較的新しいアセットだけ。古いタグを指定すると
            // ここで止まるが、検証できない配布物を同梱するより安全なので必須のままにする。
            // 原因が分かるよう、digest付きのタグを選び直すことを案内する。
            let sha256 = asset
                .get("digest")
                .and_then(Value::as_str)
                .and_then(|value| value.strip_prefix("sha256:"))
                .filter(|value| value.len() == 64)
                .ok_or_else(|| {
                    anyhow!(
                        "アセットのSHA-256 digestがありません: {name}。\
                         GitHubがdigestを公開していない古いリリースは検証できないため配置しません。\
                         digestのある新しいタグを指定してください"
                    )
                })?;
            return Ok(ReleaseAsset {
                name: name.to_string(),
                url: url.to_string(),
                size,
                sha256: sha256.to_string(),
            });
        }
    }
    let available: Vec<&str> = assets
        .iter()
        .filter_map(|a| a.get("name").and_then(Value::as_str))
        .collect();
    bail!(
        "一致するアセットが見つかりません。利用可能: {}",
        available.join(", ")
    )
}

fn verify_release_asset(path: &Path, asset: &ReleaseAsset) -> Result<()> {
    let actual_size = fs::metadata(path)?.len();
    anyhow::ensure!(
        actual_size == asset.size,
        "{} のサイズが一致しません: {actual_size}/{}",
        asset.name,
        asset.size
    );
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual_sha256 = hex::encode(digest.finalize());
    anyhow::ensure!(
        actual_sha256 == asset.sha256,
        "{} のSHA-256が一致しません",
        asset.name
    );
    Ok(())
}

fn work_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("picoagent-{label}"))
}

// ── THA3（キャラクター制作専用。通常の開発ビルド用に準備する） ────

const THA3_REVISION: &str = "8946939ec7b417f443d7b0e3fcd97384313fcdb8";
const THA3_SOURCE_URL: &str = "https://github.com/pkhungurn/talking-head-anime-3-demo/archive/8946939ec7b417f443d7b0e3fcd97384313fcdb8.zip";
// 上流 README の「Download the Models」が案内する Dropbox 配布物。`dl=1` は直接取得用。
const THA3_MODELS_URL: &str =
    "https://www.dropbox.com/s/y7b8jl4n2euv8xe/talking-head-anime-3-models.zip?dl=1";
const THA3_PYTHON_VERSION: &str = "3.11";
const THA3_TORCH_VERSION: &str = "2.12.1";
const THA3_REQUIRED_MODELS: [&str; 3] = ["standard_float", "standard_half", "separable_half"];
const UV_VERSION: &str = "0.12.5";
const WHISPER_SMALL_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const WHISPER_SMALL_SIZE: u64 = 487_601_967;
const WHISPER_SMALL_SHA256: &str =
    "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tha3Backend {
    Cuda,
    Mps,
}

fn select_tha3_backend(
    is_windows: bool,
    is_macos_arm: bool,
    has_nvidia: bool,
) -> Result<Tha3Backend> {
    if is_windows {
        anyhow::ensure!(
            has_nvidia,
            "WindowsのTHA3生成にはCUDA対応NVIDIA GPUが必要です。AMD／Intel環境ではCUDA版PyTorchを取得しません"
        );
        return Ok(Tha3Backend::Cuda);
    }
    if is_macos_arm {
        return Ok(Tha3Backend::Mps);
    }
    bail!("THA3生成の対応環境はCUDA対応NVIDIA GPU搭載WindowsとApple Silicon Macです")
}

fn tha3_backend() -> Result<Tha3Backend> {
    select_tha3_backend(
        cfg!(windows),
        cfg!(all(target_os = "macos", target_arch = "aarch64")),
        nvidia_gpu_available(),
    )
}

fn nvidia_gpu_available() -> bool {
    if !cfg!(windows) || !command_exists("nvidia-smi") {
        return false;
    }
    Command::new("nvidia-smi")
        .arg("-L")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn tha3_source_root(extracted: &Path) -> Option<PathBuf> {
    list_files(extracted)
        .into_iter()
        .find(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some("load_poser.py")
                && path
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .is_some_and(|root| root.join("README.md").is_file())
        })
        .and_then(|path| {
            path.parent()?
                .parent()?
                .parent()?
                .parent()
                .map(Path::to_path_buf)
        })
}

fn tha3_models_root(extracted: &Path) -> Option<PathBuf> {
    list_files(extracted)
        .into_iter()
        .find(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some("editor.pt")
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    == Some("standard_float")
        })
        .and_then(|path| path.parent()?.parent().map(Path::to_path_buf))
}

fn tha3_model_ready(dir: &Path) -> bool {
    THA3_REQUIRED_MODELS.iter().all(|model| {
        [
            "editor.pt",
            "eyebrow_decomposer.pt",
            "eyebrow_morphing_combiner.pt",
            "face_morpher.pt",
            "two_algo_face_body_rotator.pt",
        ]
        .iter()
        .all(|name| dir.join("data/models").join(model).join(name).is_file())
    })
}

fn tha3_runtime_python(dir: &Path) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["python.exe"]
    } else {
        &["python3", "python3.11"]
    };
    list_files(&dir.join("runtime/python"))
        .into_iter()
        .find(|path| {
            let unfinished = path
                .components()
                .any(|component| component.as_os_str() == ".temp");
            let template = path
                .components()
                .any(|component| component.as_os_str() == "venv");
            !unfinished
                && !template
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| names.contains(&name))
        })
}

fn tha3_runtime_ready(dir: &Path) -> bool {
    tha3_runtime_python(dir).is_some()
        && [
            "torch/__init__.py",
            "numpy/__init__.py",
            "PIL/__init__.py",
            "matplotlib/__init__.py",
        ]
        .iter()
        .all(|name| dir.join("runtime/packages").join(name).is_file())
}

fn cleanup_tha3_uv_cache(root: &Path) -> Result<()> {
    let cache = root.join("temp/tha3-uv-cache");
    cleanup_dir(&cache, "完了済みTHA3 uvキャッシュ")
}

fn tha3_bootstrap_ready(root: &Path) -> bool {
    !cfg!(windows)
        || root
            .join("src-tauri/resources/bundled/tha3-bootstrap/uv.exe")
            .is_file()
}

fn fetch_tha3_bootstrap(root: &Path, work: &Path) -> Result<()> {
    if !cfg!(windows) || tha3_bootstrap_ready(root) {
        return Ok(());
    }
    let archive = work.join("uv.zip");
    let url = format!(
        "https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-x86_64-pc-windows-msvc.zip"
    );
    download_to(&url, &archive)?;
    let extracted = work.join("uv");
    extract(&archive, &extracted)?;
    let uv = list_files(&extracted)
        .into_iter()
        .find(|path| path.file_name().and_then(|name| name.to_str()) == Some("uv.exe"))
        .context("uv.exeを配布アーカイブから見つけられません")?;
    let destination = root.join("src-tauri/resources/bundled/tha3-bootstrap/uv.exe");
    fs::create_dir_all(destination.parent().context("uv配置先の親がありません")?)?;
    fs::copy(&uv, &destination)
        .with_context(|| format!("THA3取得用uvを配置できません: {}", destination.display()))?;
    log(
        "setup:tha3",
        format!("installer bootstrap ready: {}", destination.display()),
    );
    Ok(())
}

fn run_uv(args: &[String], cache: &Path) -> Result<()> {
    let status = Command::new("uv")
        .args(args)
        .env("UV_CACHE_DIR", cache)
        .status()
        .context("THA3開発用Pythonを用意するuvを起動できません")?;
    if !status.success() {
        bail!(
            "uvによるTHA3開発用Pythonの準備に失敗しました（終了コード {:?}）",
            status.code()
        );
    }
    Ok(())
}

fn install_tha3_runtime(dir: &Path, work: &Path) -> Result<()> {
    let backend = tha3_backend()?;
    if tha3_runtime_ready(dir) && !flag("THA3_FORCE") {
        return Ok(());
    }
    if !command_exists("uv") {
        bail!("THA3開発用Pythonの作成にuvが必要です。https://docs.astral.sh/uv/ からuvを導入してください");
    }

    let runtime = dir.join("runtime");
    let python_root = runtime.join("python");
    let packages = runtime.join("packages");
    // 大容量PyTorchの取得が中断されても再実行で続きを使えるよう、PID別作業領域の外へ置く。
    // 完成後にだけ削除し、未完成時は次回setupのため保持する。
    let cache = work
        .parent()
        .context("THA3一時フォルダの親を取得できません")?
        .join("tha3-uv-cache");
    fs::create_dir_all(&runtime)?;
    if tha3_runtime_python(dir).is_none() {
        let unfinished = python_root.join(".temp");
        if unfinished.exists() {
            fs::remove_dir_all(&unfinished).with_context(|| {
                format!(
                    "中断された開発用Pythonを掃除できません: {}",
                    unfinished.display()
                )
            })?;
        }
        run_uv(
            &[
                "python".into(),
                "install".into(),
                THA3_PYTHON_VERSION.into(),
                "--install-dir".into(),
                python_root.to_string_lossy().into_owned(),
                "--no-bin".into(),
                "--no-registry".into(),
            ],
            &cache,
        )?;
    }
    let python =
        tha3_runtime_python(dir).context("開発用Pythonの実行ファイルを見つけられません")?;
    if flag("THA3_FORCE") && packages.exists() {
        fs::remove_dir_all(&packages).with_context(|| {
            format!(
                "古いTHA3 Python依存物を掃除できません: {}",
                packages.display()
            )
        })?;
    }
    if !tha3_runtime_ready(dir) {
        let torch_backend = match backend {
            Tha3Backend::Cuda => "cu126",
            Tha3Backend::Mps => "auto",
        };
        run_uv(
            &[
                "pip".into(),
                "install".into(),
                "--python".into(),
                python.to_string_lossy().into_owned(),
                "--target".into(),
                packages.to_string_lossy().into_owned(),
                "--torch-backend".into(),
                torch_backend.into(),
                format!("torch=={THA3_TORCH_VERSION}"),
                "numpy==2.4.6".into(),
                "pillow==12.3.0".into(),
                "matplotlib==3.11.1".into(),
            ],
            &cache,
        )?;
    }
    if !tha3_runtime_ready(dir) {
        bail!("THA3開発用Pythonの必要ファイルが揃いませんでした");
    }
    log(
        "setup:tha3",
        format!("THA3 runtime ready: {}", runtime.display()),
    );
    Ok(())
}

/// Talking Head Anime 3 のコード・学習済みモデル・Pythonを開発用領域へ配置する。
///
/// 通常の開発ビルドでは利用できるが、販売用軽量版へは再配布しない。
/// `THA3_FORCE=1` で再取得できる。
pub fn fetch_tha3() -> Result<()> {
    // 数GBのコード・モデル・Pythonを取る前に、実際に生成できる環境か確認する。
    // AMD／Intel搭載WindowsへCUDAランタイムを取得しても利用できないため停止する。
    let _ = tha3_backend()?;
    let root = repo_root();
    let dir = root.join("src-tauri/resources/bundled/tha3");
    // WindowsのCUDA版PyTorchは約4.4GB展開されNSISの約2GB上限を超えるため、
    // 開発用だけtools配下へ置く。配布先ではインストーラが専用領域へ取得する。
    let runtime_owner = if cfg!(windows) {
        root.join("tools/tha3-runtime")
    } else {
        dir.clone()
    };
    let legacy = root.join("tools/tha3");
    fs::create_dir_all(&dir)?;

    if !dir.join("tha3/poser/modes/load_poser.py").is_file()
        && legacy.join("tha3/poser/modes/load_poser.py").is_file()
    {
        copy_dir(&legacy, &dir, &|path| {
            matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".venv" | "venv" | "runtime")
            )
        })?;
        log(
            "setup:tha3",
            format!(
                "旧tools/tha3から開発用領域へ移行しました: {}",
                dir.display()
            ),
        );
    }

    let source_ready = dir.join("tha3/poser/modes/load_poser.py").is_file();
    let model_ready = tha3_model_ready(&dir);
    if source_ready
        && model_ready
        && tha3_runtime_ready(&runtime_owner)
        && tha3_bootstrap_ready(&root)
        && !flag("THA3_FORCE")
    {
        cleanup_tha3_uv_cache(&root)?;
        log("setup:tha3", format!("THA3 ready: {}", dir.display()));
        return Ok(());
    }

    // AGENTS.md の一時生成物ルールに従い、システムtempではなくリポジトリ直下を使う。
    let work = root
        .join("temp")
        .join(format!("xtask-tha3-{}", std::process::id()));
    if work.exists() {
        fs::remove_dir_all(&work)
            .with_context(|| format!("古いTHA3一時フォルダを掃除できません: {}", work.display()))?;
    }
    fs::create_dir_all(&work)?;

    let result = (|| -> Result<()> {
        fetch_tha3_bootstrap(&root, &work)?;
        if !source_ready || flag("THA3_FORCE") {
            let archive = work.join("tha3-source.zip");
            let bytes = download_to(THA3_SOURCE_URL, &archive)?;
            let extracted = work.join("source");
            extract(&archive, &extracted)?;
            let source = tha3_source_root(&extracted)
                .ok_or_else(|| anyhow!("THA3 のソースルートを展開物から見つけられません"))?;
            copy_dir(&source, &dir, &|path| {
                path.file_name().and_then(|name| name.to_str()) == Some(".git")
            })?;
            fs::write(
                dir.join(".picoagent-tha3-revision"),
                format!("{THA3_REVISION}\n"),
            )?;
            log("setup:tha3", format!("THA3 source fetched ({})", mb(bytes)));
        }

        if tha3_model_ready(&dir) && !flag("THA3_FORCE") {
            install_tha3_runtime(&runtime_owner, &work)?;
            return Ok(());
        }

        let archive = work.join("tha3-models.zip");
        let model_result = (|| -> Result<()> {
            let bytes = download_to(THA3_MODELS_URL, &archive)?;
            let extracted = work.join("models");
            extract(&archive, &extracted)?;
            let models = tha3_models_root(&extracted)
                .ok_or_else(|| anyhow!("THA3 の data/models を展開物から見つけられません"))?;
            for model in THA3_REQUIRED_MODELS {
                copy_dir(
                    &models.join(model),
                    &dir.join("data/models").join(model),
                    &|_| false,
                )?;
            }
            if !tha3_model_ready(&dir) {
                bail!("VRAM自動調整に必要なTHA3学習済みモデルが揃いませんでした");
            }
            log("setup:tha3", format!("THA3 models fetched ({})", mb(bytes)));
            Ok(())
        })();

        if let Err(error) = model_result {
            // フォールバック許可: Dropbox は認証・配布形態の変更で自動取得できない場合がある。
            // コードを残し、上流指定の手動配置先を示してから不足として終了する。
            warn(
                "setup:tha3",
                format!("学習済みモデルを自動取得できません: {error:#}"),
            );
            warn(
                "setup:tha3",
                format!(
                    "上流 README の Download the Models からZIPを取得し、standard_float / standard_half / separable_halfを {} へ置いてください。",
                    dir.join("data/models").display()
                ),
            );
        }
        if !tha3_model_ready(&dir) {
            bail!("THA3制作に必要なモデルが不足しています");
        }
        install_tha3_runtime(&runtime_owner, &work)?;
        Ok(())
    })();
    match result {
        Ok(()) => cleanup_dir(&work, "完了したTHA3一時フォルダ")?,
        Err(error) => {
            cleanup_dir(&work, "失敗したTHA3一時フォルダ")
                .with_context(|| format!("元のTHA3準備エラー: {error:#}"))?;
            return Err(error);
        }
    }
    cleanup_tha3_uv_cache(&root)?;
    log(
        "setup:tha3",
        format!("THA3 bundle ready: {}", dir.display()),
    );
    Ok(())
}

// ── 推論エンジン（llama-server） ────────────────────────────────────────

/// 推論エンジン llama.cpp（llama-server）を `resources/bundled/engine/` へ配置する。
///
/// 既定は Windows x64 の Vulkan(GPU) ビルド。別 OS / CPU ビルドは環境変数で上書きできる。
///   `LLAMACPP_ASSET`  … アセット名の一部を直接指定（最優先）
///   `LLAMACPP_FORCE=1` … 既存配置を再取得
pub fn fetch_engine() -> Result<()> {
    let dir = repo_root().join("src-tauri/resources/bundled/engine");
    let legacy = repo_root().join("src-tauri/resources/engine");
    fs::create_dir_all(&dir)?;

    let server = dir.join(exe_name("llama-server"));
    if server.exists() && !flag("LLAMACPP_FORCE") {
        make_executable(&server);
        log("setup:engine", format!("engine ready: {}", dir.display()));
        return Ok(());
    }

    // 旧配置（resources/engine）に取得済みならコピーで移行する。
    let legacy_server = legacy.join(exe_name("llama-server"));
    if legacy_server.exists() && !flag("LLAMACPP_FORCE") {
        cleanup_dir(&dir, "旧llama-server配置")?;
        crate::util::copy_dir(&legacy, &dir, &|_| false)?;
        fs::write(dir.join(".gitkeep"), "\n")?;
        make_executable(&server);
        log(
            "setup:engine",
            format!("旧 resources/engine から移行しました: {}", dir.display()),
        );
        return Ok(());
    }

    let release = get_json("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")?;
    let asset = pick_asset(&release, engine_asset_matcher())?;
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or("?");

    let work = work_dir("engine");
    cleanup_dir(&work, "前回のllama-server導入途中フォルダ")?;
    fs::create_dir_all(&work)?;
    let archive = work.join(&asset.name);
    let bytes = download_to(&asset.url, &archive)?;
    verify_release_asset(&archive, &asset)?;
    log(
        "setup:engine",
        format!("fetched {} ({}), tag {tag}", asset.name, mb(bytes)),
    );

    let extracted = work.join("x");
    extract(&archive, &extracted)?;
    let staged = work.join("staged");
    let ok = flatten(&extracted, &staged, "llama-server")?;
    if !ok {
        bail!("llama-server が展開物に見つかりません");
    }
    fs::write(staged.join(".gitkeep"), "\n")?;
    let old = dir.with_file_name(format!("engine-old-{}", std::process::id()));
    cleanup_dir(&old, "前回のllama-server退避フォルダ")?;
    if dir.exists() {
        fs::rename(&dir, &old).context("旧llama-serverを退避できません")?;
    }
    // stagingは一時ディレクトリなので、TEMPが別ドライブ・tmpfsのときrenameがEXDEVで必ず失敗する。
    // その場合はコピーで配置する（flatten と同じ方針）。
    if let Err(error) = place_dir(&staged, &dir) {
        if old.exists() {
            fs::rename(&old, &dir).with_context(|| {
                format!("llama-server配置失敗後に旧版を復元できません: {error}")
            })?;
        }
        return Err(error).context("検証済みllama-serverを配置できません");
    }
    cleanup_dir(&old, "置換済みllama-server退避フォルダ")?;
    cleanup_dir(&work, "完了したllama-server導入途中フォルダ")?;
    log("setup:engine", format!("engine ready: {}", dir.display()));
    Ok(())
}

fn engine_asset_matcher() -> impl Fn(&str) -> bool {
    let override_name = env_or("LLAMACPP_ASSET", "");
    move |name: &str| {
        if !override_name.is_empty() {
            return name.contains(&override_name);
        }
        if cfg!(windows) {
            // Windows Vulkan x64（GPU 優先・NVIDIA / Intel 兼用）
            name.contains("-bin-win-vulkan-x64") && name.ends_with(".zip")
        } else if cfg!(target_os = "macos") {
            let want = if cfg!(target_arch = "aarch64") {
                "-bin-macos-arm64"
            } else {
                "-bin-macos-x64"
            };
            name.contains(want) && name.ends_with(".tar.gz")
        } else {
            name.contains("-bin-ubuntu-x64") && name.ends_with(".tar.gz")
        }
    }
}

// ── Whisper（文字起こし） ──────────────────────────────────────────────

/// whisper.cpp の CLI と ggml モデルを `resources/bundled/whisper/` へ配置する。
///
///   `WHISPER_RELEASE` … リリースタグ（既定 latest）
///   `WHISPER_ASSET`   … アセット名の一部を直接指定
///   `WHISPER_MODEL`   … tiny/base/small/medium/large-v3-turbo または URL（既定 small）
///   `WHISPER_FORCE=1` … 既存配置を再取得
pub fn fetch_whisper() -> Result<()> {
    let dir = repo_root().join("src-tauri/resources/bundled/whisper");
    fs::create_dir_all(&dir)?;
    fetch_whisper_binary(&dir)?;
    fetch_whisper_model(&dir)?;

    // スキルが参照する起動定義を書き出す。
    let model_rel = format!("models/{}", whisper_model_file());
    let config = serde_json::json!({
        "program": exe_name("whisper-cli"),
        "model": model_rel,
        "args": "-m {model} -f {audio} -otxt -of {output_stem}",
        "diarizeArgs": "-tdrz",
    });
    fs::write(
        dir.join("whisper.json"),
        format!("{}\n", serde_json::to_string_pretty(&config)?),
    )?;
    fs::write(dir.join(".gitkeep"), "\n")?;
    log("setup:whisper", format!("whisper ready: {}", dir.display()));
    Ok(())
}

fn fetch_whisper_binary(dir: &Path) -> Result<()> {
    let exe = dir.join(exe_name("whisper-cli"));
    if exe.exists() && !flag("WHISPER_FORCE") {
        make_executable(&exe);
        log(
            "setup:whisper",
            format!("whisper binary ready: {}", exe.display()),
        );
        return Ok(());
    }

    let tag = env_or("WHISPER_RELEASE", "latest");
    let api = if tag == "latest" {
        "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest".to_string()
    } else {
        format!("https://api.github.com/repos/ggml-org/whisper.cpp/releases/tags/{tag}")
    };
    let release = get_json(&api)?;
    let asset = pick_asset(&release, whisper_asset_matcher())?;

    let work = work_dir("whisper");
    cleanup_dir(&work, "前回のWhisper導入途中フォルダ")?;
    fs::create_dir_all(&work)?;
    let archive = work.join(&asset.name);
    let bytes = download_to(&asset.url, &archive)?;
    verify_release_asset(&archive, &asset)?;
    log(
        "setup:whisper",
        format!("fetched {} ({})", asset.name, mb(bytes)),
    );

    let extracted = work.join("x");
    extract(&archive, &extracted)?;
    let ok = flatten(&extracted, dir, "whisper-cli")?;
    cleanup_dir(&work, "完了したWhisper導入途中フォルダ")?;
    if !ok {
        bail!("whisper-cli が展開物に見つかりません");
    }
    Ok(())
}

fn whisper_asset_matcher() -> impl Fn(&str) -> bool {
    let override_name = env_or("WHISPER_ASSET", "");
    move |name: &str| {
        if !override_name.is_empty() {
            return name.contains(&override_name);
        }
        let arm = cfg!(target_arch = "aarch64");
        if cfg!(windows) {
            name.ends_with(if arm {
                "whisper-bin-arm64.zip"
            } else {
                "whisper-bin-x64.zip"
            })
        } else if cfg!(target_os = "macos") {
            name.ends_with(if arm {
                "whisper-bin-macos-arm64.zip"
            } else {
                "whisper-bin-macos-x64.zip"
            })
        } else {
            name.ends_with(if arm {
                "whisper-bin-ubuntu-arm64.tar.gz"
            } else {
                "whisper-bin-ubuntu-x64.tar.gz"
            })
        }
    }
}

fn whisper_model_choice() -> String {
    env_or("WHISPER_MODEL", "small").trim().to_string()
}

fn whisper_model_file() -> String {
    let choice = whisper_model_choice();
    if choice.starts_with("http://") || choice.starts_with("https://") {
        return choice
            .rsplit('/')
            .next()
            .unwrap_or("ggml-model.bin")
            .to_string();
    }
    let normalized = if choice.starts_with("ggml-") {
        choice
    } else {
        format!("ggml-{choice}.bin")
    };
    if normalized.ends_with(".bin") {
        normalized
    } else {
        format!("{normalized}.bin")
    }
}

fn whisper_small_asset(url: String) -> ReleaseAsset {
    ReleaseAsset {
        name: "ggml-small.bin".to_string(),
        url,
        size: WHISPER_SMALL_SIZE,
        sha256: WHISPER_SMALL_SHA256.to_string(),
    }
}

fn fetch_whisper_model(dir: &Path) -> Result<()> {
    let dest = dir.join("models").join(whisper_model_file());
    let default_small = whisper_model_choice() == "small";
    let mut reuse = dest.exists() && !flag("WHISPER_FORCE");
    if reuse && default_small {
        // 一致しない既存ファイルを残したまま失敗すると、以降の setup が毎回同じ検証で止まる。
        // 上流の固定版更新や前回の破損取得から自力で復帰できるよう、消して取り直す。
        if let Err(error) = verify_release_asset(&dest, &whisper_small_asset(String::new())) {
            warn(
                "setup:whisper",
                format!("既存のggml-small.binが固定版と一致しないため取得し直します: {error:#}"),
            );
            cleanup_file(&dest, "固定版と一致しないWhisperモデル")?;
            reuse = false;
        }
    }
    if reuse {
        let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        log(
            "setup:whisper",
            format!("whisper model ready: {} ({})", dest.display(), mb(size)),
        );
        return Ok(());
    }
    let choice = whisper_model_choice();
    let url = if choice.starts_with("http://") || choice.starts_with("https://") {
        choice
    } else if default_small {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/{WHISPER_SMALL_REVISION}/ggml-small.bin"
        )
    } else {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            whisper_model_file()
        )
    };
    let bytes = download_to(&url, &dest)?;
    if default_small {
        // 検証に落ちたファイルを置いたままにすると、次回は「既存あり」経路で同じ失敗を繰り返す。
        if let Err(error) = verify_release_asset(&dest, &whisper_small_asset(url)) {
            cleanup_file(&dest, "検証に失敗したWhisperモデル")
                .with_context(|| format!("元の検証エラー: {error:#}"))?;
            return Err(error);
        }
    }
    log(
        "setup:whisper",
        format!("fetched {} ({})", whisper_model_file(), mb(bytes)),
    );
    Ok(())
}

// ── Live2D（通常はランタイムだけ、公式サンプルは明示取得） ──────────────

struct Sample {
    id: &'static str,
    label: &'static str,
    /// raw: ベース URL からファイル単位で取得 / zip: 配布 zip の runtime を使う
    zip: Option<(&'static str, &'static str)>,
    base: &'static str,
    model_json: &'static str,
    preview_url: &'static str,
}

/// Live2D描画ランタイムだけを取得する（`LIVE2D_FORCE=1` で再取得）。
/// サンプルモデルは利用者・開発者が配布元の条件を確認して自分で取得する。
pub fn fetch_live2d_runtime() -> Result<()> {
    let root = repo_root();
    let vendor = root.join("ui/vendor/live2d");

    // 公式ホスティング版Coreをモデル世代別に固定する。Cubism 5.3用Core（URLの06）は
    // 5.3 MOCを読める一方、従来のpixi-live2d-displayでは旧モデルの一部（ひより）の
    // 口ArtMeshが消える。Cubism 4と5.0以前は互換Core（URLの05）で描き分ける。
    // どちらもSHA-256まで照合し、Latestの無言更新を受けない。
    let cores = [
        (
            "live2dcubismcore-5.3.min.js",
            "https://cubism.live2d.com/sdk-web/core/06/live2dcubismcore.min.js",
            "8741f739779b5d5210872bd3d7d99f0f1e56e6c87409e7d26d6bb4b80aa1ef47",
        ),
        (
            "live2dcubismcore-compat.min.js",
            "https://cubism.live2d.com/sdk-web/core/05/live2dcubismcore.min.js",
            "25ae938cb4fe282ce189b357bcc97e603d1e1f7ec78bf04150d401c23cdc792f",
        ),
    ];
    let mut fetched = 0;
    for (name, url, sha256) in cores {
        fetched += ensure_compatible_live2d_core(url, &vendor.join(name), sha256)?;
    }
    // 旧単一Coreは描画経路から外れた生成物。配布へ混ざらないようsetup時に掃除する。
    cleanup_file(&vendor.join("live2dcubismcore.min.js"), "旧Live2D Core")?;

    // 残りも読み込み順どおり（core → pixi → display）に固定版を置く。
    let runtime = [
        (
            "pixi.min.js",
            "https://cdn.jsdelivr.net/npm/pixi.js@6.5.10/dist/browser/pixi.min.js",
        ),
        (
            "cubism4.min.js",
            "https://cdn.jsdelivr.net/npm/pixi-live2d-display@0.4.0/dist/cubism4.min.js",
        ),
    ];
    for (name, url) in runtime {
        fetched += ensure_file(url, &vendor.join(name))?;
    }
    log(
        "setup:live2d",
        if fetched > 0 {
            format!(
                "live2d runtime fetched: {} ({})",
                vendor.display(),
                mb(fetched)
            )
        } else {
            format!("live2d runtime ready: {}", vendor.display())
        },
    );
    log(
        "setup:live2d",
        "注意: Cubism Core は Live2D社のライセンス条件で利用してください。",
    );
    log(
        "setup:live2d",
        "  https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_jp.html",
    );
    Ok(())
}

/// 開発時の動作確認用に公式サンプルを明示取得する。
/// 通常のsetup・dev・build・販売ビルドからは呼ばない。
pub fn fetch_live2d_samples() -> Result<()> {
    fetch_live2d_runtime()?;
    let characters = repo_root().join("characters");

    let samples = [
        Sample {
            id: "hiyori",
            label: "ひより",
            zip: None,
            base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Hiyori/",
            model_json: "Hiyori.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-hiyori.jpg",
        },
        Sample {
            id: "haru",
            label: "はる",
            zip: Some(("https://cubism.live2d.com/sample-data/bin/haru/haru_ja.zip", "runtime")),
            base: "",
            model_json: "haru.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-haru.jpg",
        },
        Sample {
            id: "epsilon",
            label: "イプシロン",
            zip: Some((
                "https://cubism.live2d.com/sample-data/bin/epsilon/epsilon_ja.zip",
                "epsilon_free/runtime",
            )),
            base: "",
            model_json: "Epsilon_free.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-epsillon.jpg",
        },
        // ここから下は Live2D 公式リポジトリ（CubismWebSamples）の LICENSE.md が
        // 「Free Material License」で提供すると明記しているモデル。モデル実体は配布物へ
        // 同梱せず、この setup が実行時に取得する（Git 管理外）。
        Sample {
            id: "mao",
            label: "まお",
            zip: None,
            base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Mao/",
            model_json: "Mao.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-mao.jpg",
        },
        Sample {
            id: "rice",
            label: "ライス",
            zip: None,
            base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Rice/",
            model_json: "Rice.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-rice.jpg",
        },
        Sample {
            id: "wanko",
            label: "わんこ",
            zip: None,
            base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Wanko/",
            model_json: "Wanko.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-wanko.jpg",
        },
    ];

    for sample in samples {
        let ext = sample
            .preview_url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4)
            .unwrap_or("jpg");
        // プレビュー画像は設定画面のアイコンと起動待ち画面に使うだけで、モデルの動作には
        // 関わらない。配布ページの URL は将来変わりうるので、取れなくても警告に留めて
        // 共通のプレースホルダーで代替する（ここで失敗すると setup 全体が止まってしまう）。
        let preview = characters.join(sample.id).join(format!("preview.{ext}"));
        if let Err(error) = ensure_file(sample.preview_url, &preview) {
            log(
                "setup:live2d",
                format!(
                    "警告: {} のプレビュー画像を取得できませんでした（表示は代替画像で続行します）: {error}",
                    sample.label
                ),
            );
        }
        // 1体取得できなくても他のモデルの取得は続ける。配布元が落ちている・その host へ
        // 到達できないネットワークでも、取れたモデルだけで起動できる方が実用的で、
        // アプリ側もモデルが無いキャラは選択時にエラーを出すだけで済む。
        let result = match sample.zip {
            None => fetch_live2d_raw(&sample, &characters),
            Some((url, runtime_dir)) => fetch_live2d_zip(&sample, url, runtime_dir, &characters),
        };
        if let Err(error) = result {
            log(
                "setup:live2d",
                format!(
                    "警告: {} のモデルを取得できませんでした（このキャラは選べません）: {error}",
                    sample.label
                ),
            );
        }
    }

    log(
        "setup:live2d",
        "注意: サンプルモデルはLive2D社の配布条件を確認した開発者自身の環境だけで利用してください。",
    );
    log(
        "setup:live2d",
        "  https://www.live2d.com/eula/live2d-free-material-license-agreement_jp.html",
    );
    Ok(())
}

fn ensure_file(url: &str, dest: &Path) -> Result<u64> {
    if dest.exists() && !flag("LIVE2D_FORCE") {
        return Ok(0);
    }
    download_to(url, dest)
}

fn ensure_compatible_live2d_core(url: &str, dest: &Path, sha256: &str) -> Result<u64> {
    let compatible = dest.exists()
        && fs::read(dest)
            .map(|bytes| is_compatible_live2d_core(&bytes, sha256))
            .unwrap_or(false);
    if compatible && !flag("LIVE2D_FORCE") {
        return Ok(0);
    }
    let bytes = download_to(url, dest)?;
    let downloaded = fs::read(dest)?;
    if !is_compatible_live2d_core(&downloaded, sha256) {
        bail!("取得した Cubism Core が検証済みの公式版と一致しません");
    }
    Ok(bytes)
}

fn is_compatible_live2d_core(bytes: &[u8], sha256: &str) -> bool {
    hex::encode(Sha256::digest(bytes)) == sha256
}

fn fetch_live2d_raw(sample: &Sample, characters: &Path) -> Result<()> {
    let model_dir = characters.join(sample.id).join("live2d");
    let manifest = model_dir.join(sample.model_json);
    if !manifest.exists() || flag("LIVE2D_FORCE") {
        download_to(&format!("{}{}", sample.base, sample.model_json), &manifest)?;
    }
    let model3: Value = serde_json::from_str(&fs::read_to_string(&manifest)?)?;
    let mut fetched = 0u64;
    for file in referenced_files(&model3) {
        fetched += ensure_file(&format!("{}{}", sample.base, file), &model_dir.join(&file))?;
    }
    log(
        "setup:live2d",
        if fetched > 0 {
            format!("live2d model fetched: {} ({})", sample.id, mb(fetched))
        } else {
            format!("live2d model ready: {}", sample.id)
        },
    );
    Ok(())
}

fn fetch_live2d_zip(
    sample: &Sample,
    zip_url: &str,
    runtime_dir: &str,
    characters: &Path,
) -> Result<()> {
    let model_dir = characters.join(sample.id).join("live2d");
    if model_dir.join(sample.model_json).exists() && !flag("LIVE2D_FORCE") {
        log("setup:live2d", format!("live2d model ready: {}", sample.id));
        return Ok(());
    }
    let work = work_dir(&format!("live2d-{}", sample.id));
    cleanup_dir(&work, "前回のLive2Dモデル導入途中フォルダ")?;
    fs::create_dir_all(&work)?;
    let name = zip_url.rsplit('/').next().unwrap_or("model.zip");
    let archive = work.join(name);
    let bytes = download_to(zip_url, &archive)?;
    let extracted = work.join("x");
    extract(&archive, &extracted)?;

    let runtime = extracted.join(runtime_dir);
    if !runtime.join(sample.model_json).exists() {
        bail!(
            "{} の runtime に {} が見つかりません",
            sample.label,
            sample.model_json
        );
    }
    cleanup_dir(&model_dir, "旧Live2Dモデル配置")?;
    crate::util::copy_dir(&runtime, &model_dir, &|_| false)?;
    cleanup_dir(&work, "完了したLive2Dモデル導入途中フォルダ")?;
    log(
        "setup:live2d",
        format!("live2d model fetched: {} ({})", sample.id, mb(bytes)),
    );
    Ok(())
}

/// `model3.json` の `FileReferences` から参照ファイルを列挙する
/// （moc3・テクスチャ・物理・ポーズ・表情・モーション等）。
fn referenced_files(model3: &Value) -> Vec<String> {
    let mut files = Vec::new();
    let Some(refs) = model3.get("FileReferences") else {
        return files;
    };
    for key in ["Moc", "Physics", "Pose", "UserData", "DisplayInfo"] {
        if let Some(path) = refs.get(key).and_then(Value::as_str) {
            files.push(path.to_string());
        }
    }
    if let Some(textures) = refs.get("Textures").and_then(Value::as_array) {
        for texture in textures.iter().filter_map(Value::as_str) {
            files.push(texture.to_string());
        }
    }
    if let Some(expressions) = refs.get("Expressions").and_then(Value::as_array) {
        for expression in expressions {
            if let Some(file) = expression.get("File").and_then(Value::as_str) {
                files.push(file.to_string());
            }
        }
    }
    if let Some(motions) = refs.get("Motions").and_then(Value::as_object) {
        for group in motions.values().filter_map(Value::as_array) {
            for motion in group {
                for key in ["File", "Sound"] {
                    if let Some(file) = motion.get(key).and_then(Value::as_str) {
                        files.push(file.to_string());
                    }
                }
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "picoagent-place-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    /// `rename` が使えない状況（別ドライブ・tmpfs の EXDEV 相当）でもコピーで配置し、
    /// staging を残さないこと。ここでは配置先を非空にして `rename` を失敗させ、
    /// コピー経路を通す（EXDEV そのものは単体テストから再現できない）。
    #[test]
    fn renameできない場合もコピーで配置してstagingを残さない() {
        let base = test_dir("copy");
        let staged = base.join("staged");
        let dest = base.join("engine");
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("llama-server"), b"new").unwrap();
        // 非空の配置先があると rename は失敗する（ENOTEMPTY / Windows も同様）。
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("occupied"), b"old").unwrap();

        place_dir(&staged, &dest).unwrap();

        assert_eq!(fs::read(dest.join("llama-server")).unwrap(), b"new");
        assert!(!staged.exists(), "配置後に staging を残してはいけない");
        let _ = fs::remove_dir_all(&base);
    }

    /// 配置が済んだあとに staging の掃除だけ失敗しても `Ok` を返すこと。
    ///
    /// `Err` を返すと呼び出し側が「配置できなかった」と解釈して旧版を同じパスへ
    /// `rename` し戻し、配置先が埋まっているため Windows では復元も失敗して旧版が
    /// `engine-old-*` に取り残される。掃除できなかった staging は呼び出し側が
    /// 作業フォルダごと消すので、ここでは警告だけにする。
    ///
    /// 掃除失敗は注入で作る。権限を落とす方式は root で無視され、シンボリックリンク方式は
    /// `remove_dir_all` がリンクを追わずに消して成功するため、どちらも検査が空振りする。
    #[test]
    fn 配置後のstaging掃除失敗では配置を巻き戻さない() {
        fn 掃除は必ず失敗する(_path: &Path, label: &str) -> Result<()> {
            bail!("{label}を掃除できません（テスト用の注入）")
        }

        let base = test_dir("cleanup-fail");
        let staged = base.join("staged");
        let dest = base.join("engine");
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("llama-server"), b"new").unwrap();
        // 非空の配置先があると rename は失敗し、コピー経路（EXDEV 相当）へ入る。
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("occupied"), b"old").unwrap();

        let result = place_dir_inner(&staged, &dest, 掃除は必ず失敗する);

        assert!(
            result.is_ok(),
            "配置が済んだあとの掃除失敗で Err を返すと、呼び出し側が旧版を復元できない: {result:?}"
        );
        assert_eq!(
            fs::read(dest.join("llama-server")).unwrap(),
            b"new",
            "配置自体は完了していること"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn tha3は対応gpuにだけランタイムを選ぶ() {
        assert_eq!(
            select_tha3_backend(true, false, true).unwrap(),
            Tha3Backend::Cuda
        );
        assert!(select_tha3_backend(true, false, false).is_err());
        assert_eq!(
            select_tha3_backend(false, true, false).unwrap(),
            Tha3Backend::Mps
        );
        assert!(select_tha3_backend(false, false, false).is_err());
    }

    /// llama.cpp のリリース資産名からプラットフォーム用を選べること。
    #[test]
    fn エンジンのアセットを選ぶ() {
        let names = [
            "llama-b4321-bin-win-vulkan-x64.zip",
            "llama-b4321-bin-win-cpu-x64.zip",
            "llama-b4321-bin-macos-arm64.tar.gz",
            "llama-b4321-bin-macos-x64.tar.gz",
            "llama-b4321-bin-ubuntu-x64.tar.gz",
        ];
        let matcher = engine_asset_matcher();
        let picked: Vec<&str> = names.into_iter().filter(|name| matcher(name)).collect();
        assert_eq!(picked.len(), 1, "1つだけ選ばれること: {picked:?}");

        let expected = if cfg!(windows) {
            "llama-b4321-bin-win-vulkan-x64.zip"
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "llama-b4321-bin-macos-arm64.tar.gz"
            } else {
                "llama-b4321-bin-macos-x64.tar.gz"
            }
        } else {
            "llama-b4321-bin-ubuntu-x64.tar.gz"
        };
        assert_eq!(picked[0], expected);
    }

    #[test]
    fn whisperのアセットを選ぶ() {
        let names = [
            "whisper-bin-x64.zip",
            "whisper-bin-arm64.zip",
            "whisper-bin-macos-x64.zip",
            "whisper-bin-macos-arm64.zip",
            "whisper-bin-ubuntu-x64.tar.gz",
            "whisper-bin-ubuntu-arm64.tar.gz",
        ];
        let matcher = whisper_asset_matcher();
        let picked: Vec<&str> = names.into_iter().filter(|name| matcher(name)).collect();
        assert_eq!(picked.len(), 1, "1つだけ選ばれること: {picked:?}");
    }

    /// `WHISPER_MODEL` の指定形式（短縮名・ファイル名・URL）を同じ規則で解決すること。
    #[test]
    fn whisperのモデルファイル名を決める() {
        // 既定は small。
        assert_eq!(whisper_model_file(), "ggml-small.bin");
    }

    /// `model3.json` の参照ファイルを漏れなく列挙すること。
    #[test]
    fn live2dの参照ファイルを列挙する() {
        let model3: Value = serde_json::from_str(
            r#"{"FileReferences":{
                "Moc":"m.moc3",
                "Physics":"p.physics3.json",
                "Textures":["t/0.png","t/1.png"],
                "Expressions":[{"File":"e/smile.exp3.json"}],
                "Motions":{"Idle":[{"File":"m/idle.motion3.json","Sound":"s/idle.wav"}]}
            }}"#,
        )
        .unwrap();
        let files = referenced_files(&model3);
        for want in [
            "m.moc3",
            "p.physics3.json",
            "t/0.png",
            "t/1.png",
            "e/smile.exp3.json",
            "m/idle.motion3.json",
            "s/idle.wav",
        ] {
            assert!(
                files.contains(&want.to_string()),
                "{want} が漏れている: {files:?}"
            );
        }
    }
}
