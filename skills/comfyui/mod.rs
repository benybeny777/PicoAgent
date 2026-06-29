// skills/comfyui/mod.rs - ComfyUI ローカルサーバーで画像・動画を生成するスキル。
// ComfyUI を http://127.0.0.1:8188 (設定で変更可) で起動しておく必要がある。
// APIキー不要。COMFYUI_BASE_URL と COMFYUI_CHECKPOINT だけ設定すれば動く。
//
// 基本フロー:
//   1. KSampler ワークフロー JSON を構築
//   2. POST /prompt → prompt_id を取得
//   3. GET /history/{prompt_id} をポーリングして完了を待つ（最大 120 秒）
//   4. 出力ファイル名を取り出し、/view?filename=... の URL を返す

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{config_guide, Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct ComfyuiTool;

// デフォルトチェックポイント名（ComfyUI の models/checkpoints/ に置くファイル名）。
const DEFAULT_CHECKPOINT: &str = "v1-5-pruned-emaonly.safetensors";
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8188";
const POLL_INTERVAL_MS: u64 = 2_000;
const POLL_MAX_SECS: u64 = 120;

fn build_txt2img_workflow(
    prompt: &str,
    negative: &str,
    width: u32,
    height: u32,
    steps: u32,
    cfg: f64,
    seed: i64,
    checkpoint: &str,
) -> Value {
    json!({
        "4": {
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": checkpoint }
        },
        "5": {
            "class_type": "EmptyLatentImage",
            "inputs": { "width": width, "height": height, "batch_size": 1 }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": prompt, "clip": ["4", 1] }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "text": negative, "clip": ["4", 1] }
        },
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "seed": seed,
                "steps": steps,
                "cfg": cfg,
                "sampler_name": "euler",
                "scheduler": "normal",
                "denoise": 1.0,
                "model": ["4", 0],
                "positive": ["6", 0],
                "negative": ["7", 0],
                "latent_image": ["5", 0]
            }
        },
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["3", 0], "vae": ["4", 2] }
        },
        "9": {
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": "PicoAgent", "images": ["8", 0] }
        }
    })
}

/// /history レスポンスから出力ファイル情報を抽出する。
/// `(filename, subfolder, type)` のタプルを返す。
fn extract_output(history: &Value, prompt_id: &str) -> Option<(String, String, String)> {
    let outputs = history.get(prompt_id)?.get("outputs")?;
    for node_val in outputs.as_object()?.values() {
        if let Some(images) = node_val.get("images").and_then(|v| v.as_array()) {
            if let Some(first) = images.first() {
                let filename = first.get("filename")?.as_str()?.to_string();
                let subfolder = first
                    .get("subfolder")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let file_type = first
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("output")
                    .to_string();
                return Some((filename, subfolder, file_type));
            }
        }
        // 動画出力（AnimateDiff など）
        if let Some(gifs) = node_val.get("gifs").and_then(|v| v.as_array()) {
            if let Some(first) = gifs.first() {
                let filename = first.get("filename")?.as_str()?.to_string();
                let subfolder = first
                    .get("subfolder")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let file_type = first
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("output")
                    .to_string();
                return Some((filename, subfolder, file_type));
            }
        }
    }
    None
}

#[async_trait]
impl Tool for ComfyuiTool {
    fn name(&self) -> &'static str {
        "comfyui_generate"
    }
    fn label(&self) -> &'static str {
        "ComfyUI 画像生成"
    }
    fn description(&self) -> String {
        "ComfyUI ローカルサーバーでテキストから画像（または動画）を生成します。\
         APIキー不要。ComfyUI を起動しておく必要があります。\
         パラメータ: prompt（必須）、negative_prompt、width、height、steps、cfg_scale、seed。"
            .to_string()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "生成したい画像の説明（英語推奨）"
                },
                "negative_prompt": {
                    "type": "string",
                    "description": "生成したくない要素（例: blurry, ugly, text）"
                },
                "width": {
                    "type": "integer",
                    "description": "画像幅（px、デフォルト 512）",
                    "default": 512
                },
                "height": {
                    "type": "integer",
                    "description": "画像高さ（px、デフォルト 512）",
                    "default": 512
                },
                "steps": {
                    "type": "integer",
                    "description": "サンプリングステップ数（デフォルト 20）",
                    "default": 20
                },
                "cfg_scale": {
                    "type": "number",
                    "description": "CFG スケール（デフォルト 7.0）",
                    "default": 7.0
                },
                "seed": {
                    "type": "integer",
                    "description": "シード値（-1 でランダム）",
                    "default": -1
                }
            },
            "required": ["prompt"]
        })
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "COMFYUI_BASE_URL",
                "ComfyUI サーバー URL",
                "text",
                DEFAULT_BASE_URL,
            ),
            Field::new(
                "COMFYUI_CHECKPOINT",
                "チェックポイント名（models/checkpoints/ 内のファイル名）",
                "text",
                DEFAULT_CHECKPOINT,
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base_url = cfg
            .get("COMFYUI_BASE_URL", DEFAULT_BASE_URL)
            .trim_end_matches('/')
            .to_string();
        let checkpoint = cfg.get("COMFYUI_CHECKPOINT", DEFAULT_CHECKPOINT);

        if checkpoint.is_empty() {
            return config_guide(
                "ComfyUI 画像生成",
                &["COMFYUI_CHECKPOINT"],
                "models/checkpoints/ に配置したモデルファイル名（例: v1-5-pruned-emaonly.safetensors）を設定してください。",
            );
        }

        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return "エラー: prompt が指定されていません。".to_string(),
        };
        let negative = args
            .get("negative_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("worst quality, low quality, blurry, text, watermark")
            .to_string();
        let width = args
            .get("width")
            .and_then(|v| v.as_u64())
            .unwrap_or(512)
            .clamp(64, 2048) as u32;
        let height = args
            .get("height")
            .and_then(|v| v.as_u64())
            .unwrap_or(512)
            .clamp(64, 2048) as u32;
        let steps = args
            .get("steps")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .clamp(1, 100) as u32;
        let cfg_scale = args
            .get("cfg_scale")
            .and_then(|v| v.as_f64())
            .unwrap_or(7.0)
            .clamp(1.0, 30.0);
        let seed_raw = args
            .get("seed")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        let seed = if seed_raw < 0 {
            rand_seed()
        } else {
            seed_raw
        };

        let workflow = build_txt2img_workflow(
            &prompt, &negative, width, height, steps, cfg_scale, seed, &checkpoint,
        );

        // client_id は ComfyUI がプロンプトの所有者を識別するために使う（UUID形式を推奨）。
        let client_id = format!(
            "picoagent-{:016x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        );

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => return format!("エラー: HTTPクライアントの初期化に失敗しました: {e}"),
        };

        // ── 1. ワークフローをキューへ投入 ──────────────────────────────────
        let queue_url = format!("{base_url}/prompt");
        let body = json!({ "prompt": workflow, "client_id": client_id });
        let res = match client.post(&queue_url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                return format!(
                    "エラー: ComfyUI ({base_url}) へ接続できませんでした。ComfyUI が起動しているか確認してください。\n詳細: {e}"
                );
            }
        };
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return format!("エラー: ComfyUI へのキュー投入に失敗しました ({status}): {text}");
        }
        let queue_data: Value = match res.json().await {
            Ok(d) => d,
            Err(e) => return format!("エラー: ComfyUI レスポンスの解析に失敗しました: {e}"),
        };
        let prompt_id = match queue_data
            .get("prompt_id")
            .and_then(|v| v.as_str())
        {
            Some(id) => id.to_string(),
            None => return format!("エラー: prompt_id が取得できませんでした: {queue_data}"),
        };

        // ── 2. /history/{prompt_id} をポーリングして完了待ち ────────────────
        let history_url = format!("{base_url}/history/{prompt_id}");
        let poll_client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => return format!("エラー: ポーリングクライアントの初期化に失敗しました: {e}"),
        };
        let max_polls = (POLL_MAX_SECS * 1_000 / POLL_INTERVAL_MS) as u32;
        let mut output_info: Option<(String, String, String)> = None;

        for _ in 0..max_polls {
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            let history: Value = match poll_client.get(&history_url).send().await {
                Ok(r) if r.status().is_success() => r.json().await.unwrap_or(json!({})),
                _ => continue,
            };
            if let Some(info) = extract_output(&history, &prompt_id) {
                output_info = Some(info);
                break;
            }
        }

        // ── 3. 出力 URL を組み立てて返す ─────────────────────────────────────
        let (filename, subfolder, file_type) = match output_info {
            Some(info) => info,
            None => {
                return format!(
                    "エラー: {POLL_MAX_SECS} 秒以内に ComfyUI の生成が完了しませんでした。\
                     steps を減らすか、ComfyUI の状態を確認してください。"
                );
            }
        };

        let mut view_url = format!(
            "{base_url}/view?filename={}&type={}",
            urlenccode(&filename),
            urlenccode(&file_type)
        );
        if !subfolder.is_empty() {
            view_url.push_str(&format!("&subfolder={}", urlenccode(&subfolder)));
        }

        let alt: String = prompt.chars().take(40).collect();
        let is_video = filename.ends_with(".gif")
            || filename.ends_with(".mp4")
            || filename.ends_with(".webm");
        if is_video {
            format!(
                "動画を生成しました: [{filename}]({view_url})\n\
                 シード: {seed} / ステップ: {steps} / CFG: {cfg_scale}"
            )
        } else {
            format!(
                "![{alt}]({view_url})\n\
                 [画像を開く]({view_url})\n\
                 シード: {seed} / ステップ: {steps} / CFG: {cfg_scale}"
            )
        }
    }
}

fn rand_seed() -> i64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // 秒とナノ秒を混合して範囲を広げる（subsec_nanos のみだと 1e9 未満で衝突しやすい）。
    (d.as_secs().wrapping_mul(1_000_000_007) ^ d.subsec_nanos() as u64) as i64
}

fn urlenccode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                let mut buf = [0u8; 4];
                let bytes = c.encode_utf8(&mut buf).as_bytes().to_vec();
                bytes
                    .into_iter()
                    .flat_map(|b| {
                        format!("%{b:02X}").chars().collect::<Vec<_>>()
                    })
                    .collect()
            }
        })
        .collect()
}

inventory::submit! { ToolReg { make: || Box::new(ComfyuiTool) } }
