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
        match value.unwrap_or("qwen-edit") {
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
const NOT_TEXT_TO_IMAGE_MARKERS: &[&str] = &["この画像", "その画像", "添付", "動画", ":\\", ":/"];

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
        "管理下ComfyUIで画像の生成・編集または動画生成を行います。engineはqwen-edit（既定・画像）、wan-2.2-5b、ltx-0.9.8-2b-fp8から選択します。qwen-editはinput_pathを指定すると編集、省略すると文章から新しい画像を生成します。動画は任意の入力画像からも生成できます。モデルは追加機能で選んだものだけを取得します。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "prompt":{"type":"string","description":"生成・編集内容。動画は詳しい英語プロンプトを推奨"},
                "engine":{"type":"string","enum":["qwen-edit","wan-2.2-5b","ltx-0.9.8-2b-fp8"],"default":"qwen-edit"},
                "input_path":{"type":"string","description":"許可済み領域にある入力画像。qwen-editでは指定時に編集、省略時は文章から新規生成。動画では任意"},
                "width":{"type":"integer","minimum":256,"maximum":1280,"description":"省略時は画像512、動画768"},
                "height":{"type":"integer","minimum":256,"maximum":1280,"description":"省略時は画像512、動画512"},
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
        let (default_width, default_height) = if engine == Engine::QwenEdit {
            (512, 512)
        } else {
            (768, 512)
        };
        let width = normalized_video_dimension(integer(&args, "width", default_width, 256, 1280));
        let height =
            normalized_video_dimension(integer(&args, "height", default_height, 256, 1280));
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

    #[test]
    fn エンジン省略時は画像生成にする() {
        assert!(Engine::parse(None) == Ok(Engine::QwenEdit));
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
