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
}

impl Engine {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("wan-2.2-5b") {
            "qwen-edit" => Ok(Self::QwenEdit),
            "wan-2.2-5b" => Ok(Self::Wan),
            "ltx-0.9.8-2b-fp8" => Ok(Self::Ltx),
            other => Err(format!("エラー: 未対応の生成エンジンです: {other}")),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::QwenEdit => "qwen-edit",
            Self::Wan => "wan-2.2-5b",
            Self::Ltx => "ltx-0.9.8-2b-fp8",
        }
    }

    fn component(self) -> &'static str {
        match self {
            Self::QwenEdit => "character-maker",
            Self::Wan => "video-wan",
            Self::Ltx => "video-ltx",
        }
    }

    fn extension(self) -> &'static str {
        if self == Self::QwenEdit {
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
        }
    }
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
    let python = bundled.join("python/Scripts/python.exe");
    let comfy = bundled.join("character-maker/ComfyUI");
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

    fn label(&self) -> &'static str {
        "ローカル画像・動画生成"
    }

    fn description(&self) -> String {
        "管理下ComfyUIで画像編集または動画生成を行います。engineはqwen-edit、wan-2.2-5b、ltx-0.9.8-2b-fp8から選択します。Qwen画像編集はinput_path必須、動画は任意の入力画像からも生成できます。モデルは追加機能で選んだものだけを取得します。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "prompt":{"type":"string","description":"生成・編集内容。動画は詳しい英語プロンプトを推奨"},
                "engine":{"type":"string","enum":["qwen-edit","wan-2.2-5b","ltx-0.9.8-2b-fp8"],"default":"wan-2.2-5b"},
                "input_path":{"type":"string","description":"許可済み領域にある入力画像。Qwen編集では必須、動画では任意"},
                "width":{"type":"integer","default":768,"minimum":256,"maximum":1280},
                "height":{"type":"integer","default":512,"minimum":256,"maximum":1280},
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
        if engine == Engine::QwenEdit && input.is_none() {
            return "エラー: Qwen画像編集には input_path が必要です。".into();
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
        let width = normalized_video_dimension(integer(&args, "width", 768, 256, 1280));
        let height = normalized_video_dimension(integer(&args, "height", 512, 256, 1280));
        let frames = normalized_frames(integer(&args, "frames", 97, 9, 257));
        let seed = args.get("seed").and_then(Value::as_u64).unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        });
        let bundled = crate::components::runtime_root().expect("初期化済み管理領域");
        let lock = bundled
            .parent()
            .unwrap_or(bundled)
            .join("character-maker/characters/temp/completion-gpu.lock");
        let script = assets.join("sidecar/media/generate.py");
        if !script.is_file() {
            return "エラー: 画像・動画生成サイドカーが壊れています。アプリを修復してください。"
                .into();
        }
        let mut command = crate::tools::process::command(&python);
        command
            .arg(&script)
            .args(["--engine", engine.id(), "--prompt", &prompt])
            .arg("--output")
            .arg(output.as_path())
            .arg("--comfy")
            .arg(&comfy)
            .arg("--models")
            .arg(&models)
            .arg("--gpu-lock")
            .arg(&lock)
            .args([
                "--seed",
                &seed.to_string(),
                "--width",
                &width.to_string(),
                "--height",
                &height.to_string(),
                "--frames",
                &frames.to_string(),
            ]);
        if let Some(input) = &input {
            command.arg("--input").arg(input.as_path());
        }
        let result = match crate::tools::process::output(command, 7_200, "ローカル画像・動画生成")
            .await
        {
            Ok(value) => value,
            Err(error) => return format!("エラー: {error}"),
        };
        if !result.status.success() {
            let detail = String::from_utf8_lossy(&result.stderr);
            return format!("エラー: 画像・動画生成に失敗しました: {}", detail.trim());
        }
        if !crate::permissions::fs::metadata(&output)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return "エラー: 生成処理は終了しましたが成果物を確認できません。".into();
        }
        let path = output.as_path().to_string_lossy();
        if engine == Engine::QwenEdit {
            format!("![生成画像]({path})\n\n[画像を開く]({path})\nシード: {seed}")
        } else {
            format!("[生成動画を開く]({path})\nエンジン: {} / シード: {seed} / {width}×{height} / {frames}フレーム", engine.id())
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
    fn 未知のエンジンを拒否する() {
        assert!(Engine::parse(Some("unknown")).is_err());
        assert!(Engine::parse(Some("wan-2.2-5b")).is_ok());
    }
}
