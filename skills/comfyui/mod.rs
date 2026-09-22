// skills/comfyui/mod.rs - PicoAgent管理下のComfyUIで画像編集・動画生成を実行する。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::{Tool, ToolReg};
use crate::config::Config;

pub struct ComfyuiTool;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    QwenEdit,
    Wan,
    Ltx,
    Krea2,
    Ltx25,
}

impl Engine {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("qwen-edit") {
            "qwen-edit" => Ok(Self::QwenEdit),
            "wan-2.2-5b" => Ok(Self::Wan),
            "ltx-0.9.8-2b-fp8" => Ok(Self::Ltx),
            "krea-2-turbo-nvfp4" => Ok(Self::Krea2),
            "ltx-2.5-nvfp4" => Ok(Self::Ltx25),
            other => Err(format!("エラー: 未対応の生成エンジンです: {other}")),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::QwenEdit => "qwen-edit",
            Self::Wan => "wan-2.2-5b",
            Self::Ltx => "ltx-0.9.8-2b-fp8",
            Self::Krea2 => "krea-2-turbo-nvfp4",
            Self::Ltx25 => "ltx-2.5-nvfp4",
        }
    }

    fn component(self) -> &'static str {
        match self {
            Self::QwenEdit => "character-maker",
            Self::Wan => "video-wan",
            Self::Ltx => "video-ltx",
            Self::Krea2 => "image-krea2",
            Self::Ltx25 => "video-ltx25",
        }
    }

    fn extension(self) -> &'static str {
        if matches!(self, Self::QwenEdit | Self::Krea2) {
            "png"
        } else {
            "mp4"
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::QwenEdit => "画像編集・キャラクター作成基盤",
            Self::Wan => "Wan 2.2動画生成",
            Self::Ltx => "LTX軽量動画生成",
            Self::Krea2 => "Krea 2 Turbo画像生成（NVFP4）",
            Self::Ltx25 => "LTX 2.5動画生成（NVFP4）",
        }
    }
}

/// サイドカーへ渡す1回分の生成条件。
///
/// OOM後の再試行でも、ここから同じ引数列を作り直す。**変えてよいのは`--lowvram`だけ**で、
/// 入力・seed・モデル・workflow・出力先は必ず同じにする。
struct SidecarRequest<'a> {
    script: &'a std::path::Path,
    engine: Engine,
    prompt: &'a str,
    output: &'a std::path::Path,
    comfy: &'a std::path::Path,
    models: &'a std::path::Path,
    gpu_lock: &'a std::path::Path,
    seed: u64,
    width: u64,
    height: u64,
    frames: u64,
    input: Option<&'a std::path::Path>,
}

impl SidecarRequest<'_> {
    fn arguments(&self, lowvram: bool) -> Vec<std::ffi::OsString> {
        let mut arguments: Vec<std::ffi::OsString> = vec![
            self.script.into(),
            "--engine".into(),
            self.engine.id().into(),
            "--prompt".into(),
            self.prompt.into(),
            "--output".into(),
            self.output.into(),
            "--comfy".into(),
            self.comfy.into(),
            "--models".into(),
            self.models.into(),
            "--gpu-lock".into(),
            self.gpu_lock.into(),
            "--seed".into(),
            self.seed.to_string().into(),
            "--width".into(),
            self.width.to_string().into(),
            "--height".into(),
            self.height.to_string().into(),
            "--frames".into(),
            self.frames.to_string().into(),
        ];
        if let Some(input) = self.input {
            arguments.push("--input".into());
            arguments.push(input.into());
        }
        if lowvram {
            arguments.push("--lowvram".into());
        }
        arguments
    }
}

fn prepare_generation_environment(bundled: &std::path::Path) -> Result<PathBuf, String> {
    let root = bundled
        .parent()
        .unwrap_or(bundled)
        .join("generation-runtime-data");
    for directory in [root.join("home"), root.join("cache"), root.join("temp")] {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("生成ランタイムの管理領域を作成できません: {error}"))?;
    }
    Ok(root)
}

fn configure_generation_environment(command: &mut tokio::process::Command, root: &std::path::Path) {
    let home = root.join("home");
    let cache = root.join("cache");
    let temp = root.join("temp");
    command
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("HF_HOME", cache.join("huggingface"))
        .env("TORCH_HOME", cache.join("torch"))
        .env("PIP_CACHE_DIR", cache.join("pip"))
        .env("UV_CACHE_DIR", cache.join("uv"))
        .env("PYTHONPYCACHEPREFIX", cache.join("pycache"))
        .env("TMPDIR", &temp)
        .env("TMP", &temp)
        .env("TEMP", &temp);
    #[cfg(windows)]
    command
        .env("USERPROFILE", &home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"));
}

fn integer(args: &Value, name: &str, default: u64, minimum: u64, maximum: u64) -> u64 {
    args.get(name)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(minimum, maximum)
}

fn normalized_video_dimension(value: u64) -> u64 {
    (value / 32 * 32).clamp(256, 1280)
}

fn normalized_frames(value: u64) -> u64 {
    (((value.saturating_sub(1)) / 8) * 8 + 1).clamp(9, 257)
}

fn generation_root() -> Result<PathBuf, String> {
    let bundled = crate::components::runtime_root().ok_or_else(|| {
        "エラー: PicoAgentの管理領域を取得できません。アプリを再起動してください。".to_string()
    })?;
    bundled
        .parent()
        .map(|parent| parent.join("generated-media"))
        .ok_or_else(|| "エラー: 生成結果の保存先を取得できません。".to_string())
}

fn unique_output(engine: Engine) -> Result<crate::permissions::SafePath, String> {
    let root = crate::permissions::SafePath::app_owned(generation_root()?);
    crate::permissions::fs::create_dir_all(&root)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(crate::permissions::SafePath::app_owned(
        root.as_path().join(format!(
            "{}-{}-{}.{}",
            engine.id(),
            now.as_secs(),
            now.subsec_nanos(),
            engine.extension()
        )),
    ))
}

fn validate_runtime(engine: Engine) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let bundled = crate::components::runtime_root()
        .ok_or_else(|| "エラー: 追加機能の管理領域を取得できません。".to_string())?;
    // GPUを交換した後、導入時と違うバックエンドのPyTorchをそのまま使わない。
    if !crate::generation_backend::is_nvfp4_component(engine.component()) {
        if let Some(reason) = crate::components::generation_runtime_mismatch_for_app() {
            return Err(format!("エラー: {reason}"));
        }
    }
    let python = crate::components::python_executable(&bundled.join(
        if crate::generation_backend::is_nvfp4_component(engine.component()) {
            "media-runtime/venv"
        } else {
            "python"
        },
    ));
    let comfy = bundled.join(
        if crate::generation_backend::is_nvfp4_component(engine.component()) {
            "media-runtime/ComfyUI"
        } else {
            "character-maker/ComfyUI"
        },
    );
    let models = match engine {
        Engine::QwenEdit => bundled.join("character-maker/models/qwen-eval"),
        _ => bundled.join(engine.component()),
    };
    if !python.is_file() || !comfy.join("main.py").is_file() {
        return Err("エラー: 設定の「追加機能・更新」から「画像編集・キャラクター作成基盤」を先に導入してください。".into());
    }
    if !models.is_dir() {
        return Err(format!(
            "エラー: 設定の「追加機能・更新」から {} を選んで導入してください。選ばなかった動画モデルは取得しません。",
            engine.label()
        ));
    }
    Ok((python, comfy, models))
}

/// 作る対象を表す語。音声入力や変換前の入力で届くひらがな書きも含める。
/// 1文字の「え」は「まえ」「こたえ」などを拾うため入れない。
const IMAGE_NOUNS: &[&str] = &["イラスト", "いらすと", "がぞう", "画像", "絵"];
/// 作る動作を表す語（長いものを先に並べ、短い語に先取りさせない）。
const IMAGE_VERBS: &[&str] = &[
    "作成して",
    "生成して",
    "えがいて",
    "つくって",
    "作って",
    "描いて",
    "かいて",
    "書いて",
];

/// 「<対象>[を]<動作>」の範囲（開始・終了のバイト位置）を返す。
fn image_creation_span(text: &str) -> Option<(usize, usize)> {
    IMAGE_NOUNS.iter().find_map(|noun| {
        text.match_indices(noun).find_map(|(start, _)| {
            let after_noun = start + noun.len();
            let rest = &text[after_noun..];
            let (particle, rest) = rest
                .strip_prefix('を')
                .map(|stripped| ('を'.len_utf8(), stripped))
                .unwrap_or((0, rest));
            IMAGE_VERBS
                .iter()
                .find(|verb| rest.starts_with(*verb))
                .map(|verb| (start, after_noun + particle + verb.len()))
        })
    })
}
/// 入力画像の編集・動画・パス指定は、入力パスやエンジンをモデルに選ばせるため先回りしない。
const NOT_TEXT_TO_IMAGE_MARKERS: &[&str] = &[
    "この画像",
    "その画像",
    "添付",
    "動画",
    ":\\",
    ":/",
    "Krea",
    "krea",
    "KREA",
    "クレア",
    "LTX",
    "ltx",
];

/// 明示的な画像作成依頼から、生成内容（依頼の言い回しを除いた部分）を取り出す。
fn image_creation_prompt(text: &str) -> Option<String> {
    let (start, end) = image_creation_span(text)?;
    if NOT_TEXT_TO_IMAGE_MARKERS
        .iter()
        .any(|marker| text.contains(marker))
    {
        return None;
    }
    let replaced = format!("{} {}", &text[..start], &text[end..]);
    let trim = |value: &str| -> String {
        value
            .trim_matches(|character: char| {
                character.is_whitespace() || "、。,.！？!?：:".contains(character)
            })
            .to_string()
    };
    let mut prompt = trim(&replaced);
    for suffix in ["ください", "ほしい", "欲しい"] {
        if let Some(stripped) = prompt.strip_suffix(suffix) {
            prompt = trim(stripped);
        }
    }
    for suffix in ["の", "を", "な", "で"] {
        if let Some(stripped) = prompt.strip_suffix(suffix) {
            prompt = trim(stripped);
            break;
        }
    }
    (!prompt.is_empty()).then_some(prompt)
}

#[async_trait]
impl Tool for ComfyuiTool {
    fn name(&self) -> &'static str {
        "comfyui_generate"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "画像編集",
            "画像生成",
            "動画生成",
            "画像",
            "comfyui",
            "wan",
            "ltx",
        ]
    }

    fn intent_priority(&self) -> i32 {
        // 「おすすめの猫の画像を作って」などを、Web検索（100）より先に画像生成へ回す。
        110
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        // 小型モデルのtool_calls判断に任せると呼ばれないため、明示依頼は直接実行する。
        image_creation_prompt(text).map(|prompt| {
            crate::tools::IntentInvocation::direct(
                json!({ "prompt": prompt, "engine": "qwen-edit" }),
            )
        })
    }

    fn label(&self) -> &'static str {
        "ローカル画像・動画生成"
    }

    fn description(&self) -> String {
        "管理下ComfyUIで画像の生成・編集または動画生成を行います。engineはqwen-edit（既定・画像）、krea-2-turbo-nvfp4（画像生成専用）、wan-2.2-5b、ltx-0.9.8-2b-fp8、ltx-2.5-nvfp4から選択します。qwen-editはinput_pathを指定すると編集、省略すると新規画像生成です。動画は任意の入力画像からも生成できます。NVFP4はBlackwell以降の専用構成で、利用条件への同意と任意導入が必要です。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "prompt":{"type":"string","description":"生成・編集内容。動画は詳しい英語プロンプトを推奨"},
                "engine":{"type":"string","enum":["qwen-edit","wan-2.2-5b","ltx-0.9.8-2b-fp8","krea-2-turbo-nvfp4","ltx-2.5-nvfp4"],"default":"qwen-edit"},
                "input_path":{"type":"string","description":"許可済み領域にある入力画像。qwen-editでは指定時に編集、省略時は文章から新規生成。動画では任意"},
                "width":{"type":"integer","minimum":256,"maximum":1280,"description":"省略時は画像とLTX 2.5が512、その他動画768"},
                "height":{"type":"integer","minimum":256,"maximum":1280,"description":"省略時はLTX 2.5が320、その他512"},
                "frames":{"type":"integer","default":97,"minimum":9,"maximum":257,"description":"動画フレーム数。8n+1へ切り下げ"},
                "seed":{"type":"integer","minimum":0}
            },
            "required":["prompt"]
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let prompt = match args.get("prompt").and_then(Value::as_str).map(str::trim) {
            Some(value) if !value.is_empty() => value.to_string(),
            _ => return "エラー: prompt が指定されていません。".into(),
        };
        let engine = match Engine::parse(args.get("engine").and_then(Value::as_str)) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let input = match args
            .get("input_path")
            .and_then(Value::as_str)
            .map(str::trim)
        {
            Some(value) if !value.is_empty() => match crate::permissions::resolve(cfg, value) {
                Ok(path)
                    if crate::permissions::fs::metadata(&path)
                        .map(|metadata| metadata.is_file())
                        .unwrap_or(false) =>
                {
                    let extension = path
                        .as_path()
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(str::to_ascii_lowercase);
                    if !matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg" | "webp")) {
                        return "エラー: 入力画像はPNG、JPEG、WebPだけを使用できます。".into();
                    }
                    if crate::permissions::fs::metadata(&path)
                        .map(|metadata| metadata.len() > 100_000_000)
                        .unwrap_or(true)
                    {
                        return "エラー: 入力画像は100MB以下にしてください。".into();
                    }
                    Some(path)
                }
                Ok(path) => return format!("エラー: 入力画像が見つかりません: {}", path.display()),
                Err(error) => return format!("エラー: {error}"),
            },
            _ => None,
        };
        if engine == Engine::Krea2 && input.is_some() {
            return "エラー: Krea 2 Turboは文章からの画像生成専用です。入力画像の編集にはqwen-editを指定してください。".into();
        }
        if crate::generation_backend::is_nvfp4_component(engine.component()) {
            if let Err(reason) = crate::generation_backend::validate_nvfp4_hardware().await {
                return format!("エラー: {reason}");
            }
        }
        let (python, comfy, models) = match validate_runtime(engine) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let assets = match crate::components::character_maker_assets() {
            Some(path) => path,
            None => return "エラー: 画像・動画生成サイドカーが見つかりません。アプリを修復してください。".into(),
        };
        let output = match unique_output(engine) {
            Ok(path) => path,
            Err(error) => return error,
        };
        // 8GB VRAMでQwen 20Bを動かすと512×512でも約9分かかるため、画像の既定は512四方にする。
        let (default_width, default_height) = if matches!(engine, Engine::QwenEdit | Engine::Krea2)
        {
            (512, 512)
        } else if engine == Engine::Ltx25 {
            (512, 320)
        } else {
            (768, 512)
        };
        let width = normalized_video_dimension(integer(&args, "width", default_width, 256, 1280));
        let height =
            normalized_video_dimension(integer(&args, "height", default_height, 256, 1280));
        let frames = normalized_frames(integer(
            &args,
            "frames",
            if engine == Engine::Ltx25 { 49 } else { 97 },
            9,
            257,
        ));
        let seed = args.get("seed").and_then(Value::as_u64).unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        });
        // 起動処理（`components::init`）より前に呼ばれた場合も、スキルは失敗を文字列で返す。
        let Some(bundled) = crate::components::runtime_root() else {
            return "エラー: 追加機能の管理領域がまだ準備できていません。少し待ってから再試行してください。"
                .into();
        };
        let lock = bundled
            .parent()
            .unwrap_or(bundled)
            .join("character-maker/characters/temp/completion-gpu.lock");
        let script = assets.join("sidecar/media/generate.py");
        if !script.is_file() {
            return "エラー: 画像・動画生成サイドカーが壊れています。アプリを修復してください。"
                .into();
        }
        // 専用VRAM16GB未満・共有メモリ・VRAM不明はlowvramで起動する。
        // MPS（Apple Silicon）は専用VRAM判定を流用せず、検証済みの起動設定だけを使う。
        let installed = crate::components::installed_generation_runtime_for_app();
        let backend = installed.as_ref().map(|runtime| runtime.backend);
        let device = crate::generation_backend::generation_device(
            &crate::gpu::devices(),
            installed.as_ref(),
        );
        let decision = crate::generation_backend::lowvram_decision(backend, device.as_ref());
        if decision.lowvram {
            crate::generation_backend::notify_progress(
                "省メモリ設定で画像・動画生成を始めます",
                &decision.reason,
            );
        }
        // Apple Siliconで実機確認が済んでいないモデルは、その旨を隠さず伝える。
        if let Some((model, backend)) =
            crate::generation_backend::model_for(engine.component()).zip(backend)
        {
            if let Some(notice) = crate::generation_backend::mps_unverified_notice(model, backend) {
                crate::generation_backend::notify_progress(
                    "実機未確認のモデルで生成します",
                    &notice,
                );
            }
        }
        let request = SidecarRequest {
            script: &script,
            engine,
            prompt: &prompt,
            output: output.as_path(),
            comfy: &comfy,
            models: &models,
            gpu_lock: &lock,
            seed,
            width,
            height,
            frames,
            input: input.as_ref().map(|path| path.as_path()),
        };
        let generation_environment = match prepare_generation_environment(bundled) {
            Ok(value) => value,
            Err(error) => return format!("エラー: {error}"),
        };
        let build = |lowvram: bool| {
            let mut command = crate::tools::process::command(&python);
            command.args(request.arguments(lowvram));
            configure_generation_environment(&mut command, &generation_environment);
            command
        };
        let mut lowvram = decision.lowvram;
        let mut result =
            match crate::tools::process::output(build(lowvram), 7_200, "ローカル画像・動画生成")
                .await
            {
                Ok(value) => value,
                Err(error) => return format!("エラー: {error}"),
            };
        // 通常起動のGPUメモリ不足だけ、省メモリ設定で1回だけやり直す。
        // サイドカーは終了前にComfyUIと子孫プロセス、途中出力を必ず回収している。
        if crate::generation_backend::should_retry_with_lowvram(
            backend,
            result.status.code(),
            lowvram,
        ) {
            crate::generation_backend::notify_progress(
                "GPUメモリ不足のため省メモリ設定でやり直します",
                "同じ指示・シード・モデルのまま、省メモリ設定（lowvram）で1回だけ再実行します。",
            );
            lowvram = true;
            result = match crate::tools::process::output(
                build(lowvram),
                7_200,
                "ローカル画像・動画生成（省メモリ再試行）",
            )
            .await
            {
                Ok(value) => value,
                Err(error) => return format!("エラー: {error}"),
            };
        }
        if !result.status.success() {
            if result.status.code() == Some(crate::generation_backend::GPU_OOM_EXIT_CODE) {
                return if lowvram {
                    "エラー: GPUメモリが不足しています。省メモリ設定でも生成できませんでした。画像・動画のサイズかフレーム数を小さくしてください。".into()
                } else {
                    // MPSでは未検証の起動引数へ切り替えず、条件を小さくする案内だけを返す。
                    "エラー: GPUメモリが不足しています。画像・動画のサイズかフレーム数を小さくして試してください。".to_string()
                };
            }
            // 英語の例外や内部コマンドを利用者へそのまま見せない。原文はログへ残す。
            let detail = String::from_utf8_lossy(&result.stderr);
            if let Some(app) = crate::durable_jobs::app_handle() {
                crate::log_paths::append_app_log(
                    app,
                    &format!("画像・動画生成に失敗しました: {}", detail.trim()),
                );
            }
            return format!(
                "エラー: 画像・動画生成に失敗しました（{}／{}）。指示や画像サイズを変えて試すか、設定の「診断・修復」からログを確認してください。",
                engine.label(),
                if lowvram { "省メモリ設定" } else { "通常起動" }
            );
        }
        if !crate::permissions::fs::metadata(&output)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return "エラー: 生成処理は終了しましたが成果物を確認できません。".into();
        }
        let path = output.as_path().to_string_lossy();
        let review_notice = if crate::generation_backend::is_nvfp4_component(engine.component()) {
            "\n公開前に成果物の内容・権利・提供元の利用条件を本人が確認してください。"
        } else {
            ""
        };
        if matches!(engine, Engine::QwenEdit | Engine::Krea2) {
            format!("![生成画像]({path})\n\n[画像を開く]({path})\nモデル: {} / シード: {seed}{review_notice}", engine.label())
        } else {
            format!("[生成動画を開く]({path})\nモデル: {} / エンジン: {} / シード: {seed} / {width}×{height} / {frames}フレーム{review_notice}", engine.label(), engine.id())
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ComfyuiTool) } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 動画寸法とフレーム数をモデル制約へ揃える() {
        assert_eq!(normalized_video_dimension(777), 768);
        assert_eq!(normalized_video_dimension(64), 256);
        assert_eq!(normalized_frames(100), 97);
        assert_eq!(normalized_frames(1), 9);
    }

    #[test]
    fn 再試行で変わるのはlowvramだけにする() {
        let request = SidecarRequest {
            script: std::path::Path::new("C:/pico/sidecar/media/generate.py"),
            engine: Engine::Wan,
            prompt: "白い猫が走る",
            output: std::path::Path::new("C:/pico/generated-media/wan-1.mp4"),
            comfy: std::path::Path::new("C:/pico/character-maker/ComfyUI"),
            models: std::path::Path::new("C:/pico/video-wan"),
            gpu_lock: std::path::Path::new("C:/pico/completion-gpu.lock"),
            seed: 12_345,
            width: 768,
            height: 512,
            frames: 97,
            input: Some(std::path::Path::new("C:/pico/input.png")),
        };
        let normal = request.arguments(false);
        let retry = request.arguments(true);
        assert_eq!(retry.len(), normal.len() + 1);
        assert_eq!(&retry[..normal.len()], normal.as_slice());
        assert_eq!(retry[normal.len()], std::ffi::OsString::from("--lowvram"));
        assert!(!normal.contains(&std::ffi::OsString::from("--lowvram")));
        // 同じ条件からは常に同じ引数列ができる（seed・入力・出力先が揺れない）。
        assert_eq!(request.arguments(false), normal);
    }

    #[test]
    fn 未知のエンジンを拒否する() {
        assert!(Engine::parse(Some("unknown")).is_err());
        assert!(Engine::parse(Some("wan-2.2-5b")).is_ok());
    }

    #[test]
    fn エンジン省略時は画像生成にする() {
        assert!(Engine::parse(None) == Ok(Engine::QwenEdit));
    }

    #[test]
    fn 新量子化エンジンのモデルと成果物形式を分離する() {
        assert!(Engine::parse(Some("krea-2-turbo-nvfp4")) == Ok(Engine::Krea2));
        assert!(Engine::parse(Some("ltx-2.5-nvfp4")) == Ok(Engine::Ltx25));
        assert_eq!(Engine::Krea2.component(), "image-krea2");
        assert_eq!(Engine::Krea2.extension(), "png");
        assert_eq!(Engine::Ltx25.component(), "video-ltx25");
        assert_eq!(Engine::Ltx25.extension(), "mp4");
    }

    #[test]
    fn 明示的な画像作成依頼から生成内容を取り出す() {
        assert_eq!(
            image_creation_prompt("青空の下で昼寝をする白い猫の画像を作って").as_deref(),
            Some("青空の下で昼寝をする白い猫")
        );
        assert_eq!(
            image_creation_prompt("猫のイラストを描いてください").as_deref(),
            Some("猫")
        );
        assert_eq!(
            image_creation_prompt("みこのがぞうつくって").as_deref(),
            Some("みこ")
        );
        assert_eq!(
            image_creation_prompt("桜の木のいらすとかいて").as_deref(),
            Some("桜の木")
        );
        assert_eq!(
            image_creation_prompt("夕焼けの海の絵を描いてほしい").as_deref(),
            Some("夕焼けの海")
        );
        assert_eq!(image_creation_prompt("画像作って"), None);
        assert_eq!(image_creation_prompt("まえをかいて"), None);
        assert_eq!(image_creation_prompt("画像を見て"), None);
        assert_eq!(image_creation_prompt("この画像を元に画像を作って"), None);
        assert_eq!(image_creation_prompt("猫の動画を作って"), None);
        assert_eq!(image_creation_prompt("今日の天気は？"), None);
    }

    #[test]
    fn 画像作成依頼はweb検索より先に直接実行する() {
        let matched = crate::tools::intent_for_text("おすすめの猫の画像を作って")
            .expect("画像作成依頼は先回り実行する");
        assert_eq!(matched.tool_name, "comfyui_generate");
        assert!(matches!(
            matched.invocation.handling,
            crate::tools::IntentHandling::Direct
        ));
        assert_eq!(matched.invocation.args["prompt"], "おすすめの猫");
        assert_eq!(matched.invocation.args["engine"], "qwen-edit");
    }
}
