// skills/image_generate/mod.rs - OpenAI DALL-E 3 で画像を生成するスキル。
// OPENAI_BASE_URL / OPENAI_API_KEY から接続先を読む（openai プロバイダと設定を共用）。
// 成功時は ![alt](url) 形式の Markdown 画像記法と開くリンクを返す。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{config_guide, Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct ImageGenerateTool;

#[async_trait]
impl Tool for ImageGenerateTool {
    fn name(&self) -> &'static str {
        "image_generate"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["dall-e", "dall\u{00b7}e", "openai画像"]
    }
    fn label(&self) -> &'static str {
        "画像生成"
    }
    fn description(&self) -> String {
        "テキストプロンプトから画像を生成します（DALL-E 3）。生成した画像の URL と表示用 Markdown を返します。".to_string()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "生成する画像の説明（英語推奨）"
                },
                "size": {
                    "type": "string",
                    "description": "画像サイズ（1024x1024 / 1792x1024 / 1024x1792）",
                    "enum": ["1024x1024", "1792x1024", "1024x1792"]
                },
                "quality": {
                    "type": "string",
                    "description": "画質（standard / hd）",
                    "enum": ["standard", "hd"]
                }
            },
            "required": ["prompt"]
        })
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "OPENAI_API_KEY",
                "OpenAI API キー（DALL-E 3 用）",
                "password",
                "sk-...",
            ),
            Field::new(
                "OPENAI_BASE_URL",
                "エンドポイント",
                "text",
                "https://api.openai.com/v1",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let api_key = cfg.get("OPENAI_API_KEY", "");
        if api_key.is_empty() {
            return config_guide(
                "画像生成",
                &["OPENAI_API_KEY"],
                "OpenAI の API キーが必要です（DALL-E 3 を利用）。https://platform.openai.com/api-keys から取得してください。",
            );
        }

        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return "エラー: prompt が指定されていません。".to_string(),
        };
        let size = args
            .get("size")
            .and_then(|v| v.as_str())
            .unwrap_or("1024x1024");
        let quality = args
            .get("quality")
            .and_then(|v| v.as_str())
            .unwrap_or("standard");

        let base_url = cfg
            .get("OPENAI_BASE_URL", "https://api.openai.com/v1")
            .trim_end_matches('/')
            .to_string();
        let url = format!("{}/images/generations", base_url);

        let body = json!({
            "model": "dall-e-3",
            "prompt": prompt,
            "n": 1,
            "size": size,
            "quality": quality,
            "response_format": "url"
        });

        let request_timeout = std::time::Duration::from_secs(90);

        let res = match crate::tool_try!(crate::tools::http::service(
            reqwest::Method::POST,
            &url,
            request_timeout
        ))
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        {
            Ok(r) => r,
            Err(e) => return format!("エラー: 画像生成 API への接続に失敗しました: {e}"),
        };

        if !res.status().is_success() {
            let status = res.status();
            let text = crate::tool_response_text!(res);
            return format!("エラー: 画像生成に失敗しました ({status}): {text}");
        }

        let data: Value = match res.json().await {
            Ok(d) => d,
            Err(e) => return format!("エラー: レスポンスの解析に失敗しました: {e}"),
        };

        let image_url = match data
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("url"))
            .and_then(|v| v.as_str())
        {
            Some(u) => u.to_string(),
            None => return "エラー: 画像 URL が取得できませんでした。".to_string(),
        };

        let revised = data
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("revised_prompt"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("");

        let alt: String = prompt.chars().take(40).collect();
        if revised.is_empty() {
            format!("![{alt}]({image_url})\n[画像を開く]({image_url})")
        } else {
            format!("![{alt}]({image_url})\n[画像を開く]({image_url})\n※ プロンプト調整: {revised}")
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ImageGenerateTool) } }
