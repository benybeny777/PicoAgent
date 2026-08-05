// xtask/src/bundled.rs - 同梱物（推論エンジン / Whisper / Live2D）の取得。
//
// 旧 `scripts/fetch-engine.mjs` / `fetch-whisper.mjs` / `fetch-live2d.mjs` の置き換え。
// 実体は巨大なのでリポジトリには含めず、各環境で取得する。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::util::{
    download_to, env_or, exe_name, extract, flag, get_json, list_files, log, make_executable, mb,
    repo_root,
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

/// GitHub リリースのアセットから、名前が条件に合う最初のものを選ぶ。
fn pick_asset(release: &Value, matches: impl Fn(&str) -> bool) -> Result<(String, String)> {
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
            return Ok((name.to_string(), url.to_string()));
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

fn work_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("picoagent-{label}"))
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
        fs::remove_dir_all(&dir).ok();
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
    let (name, url) = pick_asset(&release, engine_asset_matcher())?;
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or("?");

    let work = work_dir("engine");
    fs::remove_dir_all(&work).ok();
    fs::create_dir_all(&work)?;
    let archive = work.join(&name);
    let bytes = download_to(&url, &archive)?;
    log(
        "setup:engine",
        format!("fetched {name} ({}), tag {tag}", mb(bytes)),
    );

    let extracted = work.join("x");
    extract(&archive, &extracted)?;
    let ok = flatten(&extracted, &dir, "llama-server")?;
    fs::remove_dir_all(&work).ok();
    if !ok {
        bail!("llama-server が展開物に見つかりません");
    }
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
    let (name, url) = pick_asset(&release, whisper_asset_matcher())?;

    let work = work_dir("whisper");
    fs::remove_dir_all(&work).ok();
    fs::create_dir_all(&work)?;
    let archive = work.join(&name);
    let bytes = download_to(&url, &archive)?;
    log("setup:whisper", format!("fetched {name} ({})", mb(bytes)));

    let extracted = work.join("x");
    extract(&archive, &extracted)?;
    let ok = flatten(&extracted, dir, "whisper-cli")?;
    fs::remove_dir_all(&work).ok();
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

fn fetch_whisper_model(dir: &Path) -> Result<()> {
    let dest = dir.join("models").join(whisper_model_file());
    if dest.exists() && !flag("WHISPER_FORCE") {
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
    } else {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            whisper_model_file()
        )
    };
    let bytes = download_to(&url, &dest)?;
    log(
        "setup:whisper",
        format!("fetched {} ({})", whisper_model_file(), mb(bytes)),
    );
    Ok(())
}

// ── Live2D（ランタイム + 公式サンプルモデル） ──────────────────────────

struct Sample {
    id: &'static str,
    label: &'static str,
    /// raw: ベース URL からファイル単位で取得 / zip: 配布 zip の runtime を使う
    zip: Option<(&'static str, &'static str)>,
    base: &'static str,
    model_json: &'static str,
    preview_url: &'static str,
}

/// ランタイム3点とサンプルモデルを取得する（`LIVE2D_FORCE=1` で再取得）。
pub fn fetch_live2d() -> Result<()> {
    let root = repo_root();
    let vendor = root.join("ui/vendor/live2d");
    let characters = root.join("characters");

    // 読み込み順もこの順（core → pixi → display）。
    let runtime = [
        (
            "live2dcubismcore.min.js",
            "https://cubism.live2d.com/sdk-web/cubismcore/live2dcubismcore.min.js",
        ),
        (
            "pixi.min.js",
            "https://cdn.jsdelivr.net/npm/pixi.js@6.5.10/dist/browser/pixi.min.js",
        ),
        (
            "cubism4.min.js",
            "https://cdn.jsdelivr.net/npm/pixi-live2d-display@0.4.0/dist/cubism4.min.js",
        ),
    ];
    let mut fetched = 0u64;
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
            id: "mark",
            label: "マーク",
            zip: None,
            base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Mark/",
            model_json: "Mark.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-mark.jpg",
        },
        Sample {
            id: "natori",
            label: "なとり",
            zip: None,
            base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Natori/",
            model_json: "Natori.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-natori.jpg",
        },
        Sample {
            id: "ren",
            label: "レン",
            zip: None,
            base: "https://raw.githubusercontent.com/Live2D/CubismWebSamples/develop/Samples/Resources/Ren/",
            model_json: "Ren.model3.json",
            preview_url: "https://www.live2d.com/wp-content/uploads/2026/06/sample-img-ren.jpg",
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
        "注意: Cubism Core とサンプルモデルは Live2D 社のライセンス条件で利用してください。",
    );
    log(
        "setup:live2d",
        "  https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_jp.html",
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
    fs::remove_dir_all(&work).ok();
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
    fs::remove_dir_all(&model_dir).ok();
    crate::util::copy_dir(&runtime, &model_dir, &|_| false)?;
    fs::remove_dir_all(&work).ok();
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
