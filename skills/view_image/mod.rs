// skills/view_image/mod.rs - 許可ルート配下の画像をモデルへ見せるスキル。
//
// ツールの戻り値は文字列なので、画像そのものを結果として返すことはできない。
// ここでは「この画像を見せる」という指示（マーカー）だけを返し、エージェントループ
// （`agent.rs`）が画像付き user メッセージを会話へ差し込む。Codex の view_image に相当する。
//
// 実際に画像を読めるかはモデル側の対応（マルチモーダルかどうか）に依る。
// 送る側の形式は各プロバイダが変換するが、非対応モデルは画像を無視する。
// 黙って「見えたふり」にならないよう、その旨をツール結果へ明記する。

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions;

/// 埋め込む画像の元サイズ上限（base64 で約 4/3 に膨らむ点を見込む）。
const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
/// 1回で渡せる枚数（コンテキストを画像で埋め尽くさないため）。
const MAX_IMAGES: usize = 3;

const MARKER: &str = "__PICOAGENT_VIEW_IMAGE__";

/// マーカーで受け渡す添付画像。`agent.rs` が会話へ差し込む。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingImage {
    /// 表示用のパス（モデルへ「どのファイルか」を伝える）。
    pub label: String,
    /// `data:image/png;base64,...` 形式。
    pub data_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingImages {
    pub note: String,
    pub images: Vec<PendingImage>,
}

fn media_type(extension: &str) -> Option<&'static str> {
    match extension {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn marker(pending: &PendingImages) -> String {
    match serde_json::to_string(pending) {
        Ok(encoded) => format!("{MARKER}{encoded}"),
        // フォールバック許可: 直列化に失敗したら添付をあきらめ、理由を文字列で返す
        // （マーカーを壊した形で返すと、agent 側が黙って無視することになるため）。
        Err(error) => format!("エラー: 画像を渡す準備に失敗しました: {error}"),
    }
}

/// エージェントループが呼ぶ。マーカーなら添付画像を取り出す。
pub fn parse_marker(text: &str) -> Option<PendingImages> {
    let raw = text.strip_prefix(MARKER)?;
    serde_json::from_str(raw).ok()
}

fn paths_arg(args: &Value) -> Result<Vec<String>, String> {
    if let Some(list) = args.get("paths").and_then(Value::as_array) {
        if list.is_empty() {
            return Err("paths が空です。".to_string());
        }
        if list.len() > MAX_IMAGES {
            return Err(format!("画像は一度に {MAX_IMAGES} 枚までです。"));
        }
        let mut out = Vec::with_capacity(list.len());
        for item in list {
            let path = item.as_str().unwrap_or("").trim();
            if path.is_empty() {
                return Err("paths に空の要素があります。".to_string());
            }
            out.push(path.to_string());
        }
        return Ok(out);
    }
    match args.get("path").and_then(Value::as_str).map(str::trim) {
        Some(path) if !path.is_empty() => Ok(vec![path.to_string()]),
        _ => Err("path が指定されていません。".to_string()),
    }
}

pub struct ViewImageTool;

#[async_trait]
impl Tool for ViewImageTool {
    fn name(&self) -> &'static str {
        "view_image"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "画像を見",
            "スクリーンショット",
            "写真を見",
            "何が写",
            "この図",
            "画面の内容",
        ]
    }

    fn label(&self) -> &'static str {
        "画像を見る"
    }

    fn description(&self) -> String {
        "許可ルート配下の画像ファイルを自分（モデル）へ見せる。スクリーンショットや図の中身を判断したいときに使う。読めるかはモデルがマルチモーダルかどうかに依る。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "見たい画像のパス（許可ルート配下）。png / jpg / gif / webp / bmp。"
                },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "複数枚まとめて見る場合の一覧（最大3枚）。指定した場合は path より優先する。"
                }
            },
            "required": ["path"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["このスクリーンショットを見て何が写っているか教えて"]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let paths = match paths_arg(&args) {
            Ok(paths) => paths,
            Err(error) => return format!("エラー: {error}"),
        };
        let mut images = Vec::with_capacity(paths.len());
        for path in &paths {
            // 許可ルート内かつ保護パスでないことを共通ルールで検証する。
            let target = match permissions::resolve(cfg, path) {
                Ok(target) => target,
                Err(error) => return format!("エラー: {error}"),
            };
            let extension = target.extension_lossy().to_lowercase();
            let Some(media_type) = media_type(&extension) else {
                return format!(
                    "エラー: 対応していない画像形式です（png / jpg / gif / webp / bmp）: {}",
                    target.display()
                );
            };
            let size = match permissions::fs::metadata(&target) {
                Ok(meta) => meta.len(),
                Err(error) => {
                    return format!(
                        "エラー: 情報を取得できません（{error}）: {}",
                        target.display()
                    )
                }
            };
            if size > MAX_IMAGE_BYTES {
                return format!(
                    "エラー: {MAX_IMAGE_BYTES} バイトを超える画像は渡せません（{size} バイト）: {}",
                    target.display()
                );
            }
            let bytes = match permissions::fs::read(&target) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return format!(
                        "エラー: 読み取りに失敗しました（{error}）: {}",
                        target.display()
                    )
                }
            };
            images.push(PendingImage {
                label: target.display().to_string(),
                data_url: format!("data:{media_type};base64,{}", B64.encode(&bytes)),
            });
        }

        let note = format!(
            "画像 {} 枚を会話へ添付しました（{}）。マルチモーダル非対応のモデルではこの画像は無視されます。その場合は見えないと正直に答えること。",
            images.len(),
            images
                .iter()
                .map(|image| image.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        marker(&PendingImages { note, images })
    }
}

inventory::submit! { ToolReg { make: || Box::new(ViewImageTool) } }

#[cfg(test)]
mod tests {
    use super::{marker, media_type, parse_marker, paths_arg, PendingImage, PendingImages};
    use serde_json::json;

    #[test]
    fn 対応形式だけを受け付ける() {
        assert_eq!(media_type("png"), Some("image/png"));
        assert_eq!(media_type("jpeg"), Some("image/jpeg"));
        assert_eq!(media_type("svg"), None);
    }

    #[test]
    fn 単数と複数のパス指定を受け取る() {
        assert_eq!(
            paths_arg(&json!({ "path": "a.png" })).unwrap(),
            vec!["a.png"]
        );
        assert_eq!(
            paths_arg(&json!({ "paths": ["a.png", "b.png"] }))
                .unwrap()
                .len(),
            2
        );
        assert!(paths_arg(&json!({ "paths": [] })).is_err());
        assert!(paths_arg(&json!({ "paths": ["a", "b", "c", "d"] })).is_err());
        assert!(paths_arg(&json!({})).is_err());
    }

    /// マーカーは agent 側で往復できること（壊れると画像が黙って消える）。
    #[test]
    fn マーカーを往復できる() {
        let pending = PendingImages {
            note: "テスト".to_string(),
            images: vec![PendingImage {
                label: "図.png".to_string(),
                data_url: "data:image/png;base64,AAAA".to_string(),
            }],
        };
        let text = marker(&pending);
        let parsed = parse_marker(&text).expect("往復できない");
        assert_eq!(parsed.images.len(), 1);
        assert_eq!(parsed.images[0].label, "図.png");
        assert!(parse_marker("ただの文字列").is_none());
    }
}
