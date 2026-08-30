// skills/image_tools/mod.rs - ImageMagick による画像ファイル変換・情報取得スキル。
// PSD/XCF/HEIC/TIFF/WebP など WebView やモデルが直接扱えない画像形式を PNG/JPG 等へ
// 変換する。実行ファイルは「明示設定 → 同梱（setup tools）→ PATH」の順で解決し、
// PATH では v7（magick）→ v6（convert / identify）の順にフォールバックする。

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::permissions::SafePath;

pub struct ImageToolsTool;

/// 入出力パスを共通の許可ルール（許可ルート・拒否パス・リンク実体）で解決する。
fn resolve_path(cfg: &Config, args: &Value, key: &str) -> Result<SafePath, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{} が指定されていません。", key))?;
    crate::permissions::resolve(cfg, raw)
}

// レイヤー・複数フレームを持ちうる形式（変換時は [0]＝統合ビューだけを使う）。
const MULTI_LAYER_EXTS: [&str; 5] = ["psd", "xcf", "tif", "tiff", "gif"];

/// ImageMagick の実行プログラムを解決する。
/// 明示設定・同梱があればそれ（v7 の magick）を、PATH フォールバック時は
/// `magick -version` が通れば v7、通らなければ v6 の `legacy`（convert / identify）を使う。
/// `<program> -version` が **ImageMagick 自身**だと名乗るか確かめる。
///
/// 起動できたかどうか（`is_ok()` / 終了コード）だけで判定してはならない。Windows には
/// `C:\Windows\System32\convert.exe`（FAT→NTFS 変換ユーティリティ）が標準で存在するため、
/// コマンド名 `convert` は ImageMagick が無い環境でも必ず起動に成功してしまう。
/// 名前が同じだけの無関係なシステムユーティリティへ画像変換の引数を渡さないよう、
/// 出力に `ImageMagick` が含まれることまで確認する。
async fn is_imagemagick(program: &str) -> bool {
    let mut command = crate::tools::process::command(program);
    command.arg("-version").stdin(Stdio::null());
    // 名乗りの確認だけなので短い上限で打ち切る。共通層を通さず `.output().await` を
    // 直接呼ぶと上限が無くなり、壊れた実行ファイルを掴んだときに変換全体が止まる。
    match crate::tools::process::output(command, 10, "ImageMagick の確認").await {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).contains("ImageMagick")
        }
        _ => false,
    }
}

async fn magick_program(cfg: &Config, legacy: &str) -> String {
    let resolved =
        crate::tools::resolve_cli_path(cfg, "PICOAGENT_MAGICK_PATH", "imagemagick", "magick");
    if resolved != "magick" {
        return resolved;
    }
    if is_imagemagick("magick").await {
        return "magick".to_string();
    }
    // フォールバック許可: apt の ImageMagick v6 はコマンド名が convert / identify。
    // ただし名乗りを確認できたときだけ使う（同名の別物を掴まないため）。
    if is_imagemagick(legacy).await {
        return legacy.to_string();
    }
    // どちらも ImageMagick ではない＝未導入。正式名で返し、`run_magick` のエラーメッセージが
    // 「ImageMagick が入っていない」という本当の原因を指すようにする。
    "magick".to_string()
}

async fn run_magick(
    mut command: Command,
    timeout: u64,
    label: &str,
) -> Result<std::process::Output, String> {
    match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(format!(
            "エラー: ImageMagick の起動に失敗しました: {}。cargo xtask setup tools か apt/brew で ImageMagick を導入してください。",
            error
        )),
        Err(_) => Err(format!("エラー: {} が {} 秒でタイムアウトしました。", label, timeout)),
    }
}

/// 画像を別形式へ変換する（PSD/XCF 等は統合ビューを使用。resize / quality 対応）。
async fn run_convert(cfg: &Config, args: &Value, timeout: u64) -> String {
    let input = match resolve_path(cfg, args, "input_path") {
        Ok(path) => path,
        Err(error) => return format!("エラー: {}", error),
    };
    if !input.exists() {
        return format!("エラー: 入力画像が見つかりません: {}", input.display());
    }
    let output = match resolve_path(cfg, args, "output_path") {
        Ok(path) => path,
        Err(error) => return format!("エラー: {}", error),
    };
    if let Err(error) = crate::permissions::fs::create_dir_all(&output.parent_or_root(cfg)) {
        return format!("エラー: 出力フォルダ作成に失敗しました: {}", error);
    }
    let program = magick_program(cfg, "convert").await;
    let mut command = crate::tools::process::command(&program);
    // レイヤーを持つ形式は [0]（統合ビュー）だけを読み、複数ファイル出力を避ける。
    let input_display = input.as_path().to_string_lossy().to_string();
    let input_arg = if MULTI_LAYER_EXTS.contains(&input.extension_lossy().as_str()) {
        format!("{}[0]", input_display)
    } else {
        input_display
    };
    command.arg(&input_arg);
    if let Some(resize) = args
        .get("resize")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        command.arg("-resize").arg(resize);
    }
    if let Some(quality) = args
        .get("quality")
        .and_then(Value::as_u64)
        .filter(|v| (1..=100).contains(v))
    {
        command.arg("-quality").arg(quality.to_string());
    }
    // 透過を持てない形式（jpg 等）へ変換する際に背景が黒くならないよう白で敷く。
    if matches!(output.extension_lossy().as_str(), "jpg" | "jpeg" | "bmp") {
        command
            .arg("-background")
            .arg("white")
            .arg("-alpha")
            .arg("remove");
    }
    command.arg(&output);
    let result = match run_magick(command, timeout, "画像変換").await {
        Ok(output) => output,
        Err(error) => return error,
    };
    if !result.status.success() || !output.exists() {
        return format!(
            "エラー: 画像変換に失敗しました（終了コード {}）:\n{}",
            result
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "シグナル終了".to_string()),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    format!(
        "画像を変換しました: {} → {}",
        input.display(),
        output.display()
    )
}

/// 画像の情報（形式・寸法・レイヤー数など）を identify で取得する。
async fn run_info(cfg: &Config, args: &Value, timeout: u64) -> String {
    let input = match resolve_path(cfg, args, "input_path") {
        Ok(path) => path,
        Err(error) => return format!("エラー: {}", error),
    };
    if !input.exists() {
        return format!("エラー: 入力画像が見つかりません: {}", input.display());
    }
    let program = magick_program(cfg, "identify").await;
    let mut command = crate::tools::process::command(&program);
    // v7（magick）はサブコマンドで identify を呼ぶ。v6 の identify や、明示設定で
    // identify 実行ファイルを直接指定した場合はサブコマンドを付けない。
    let is_identify_exe = Path::new(&program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.eq_ignore_ascii_case("identify"))
        .unwrap_or(false);
    if !is_identify_exe {
        command.arg("identify");
    }
    command.arg(&input);
    let result = match run_magick(command, timeout, "画像情報取得").await {
        Ok(output) => output,
        Err(error) => return error,
    };
    if !result.status.success() {
        return format!(
            "エラー: 画像情報の取得に失敗しました:\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let text = String::from_utf8_lossy(&result.stdout);
    let lines: Vec<&str> = text.lines().collect();
    format!(
        "{} の画像情報（{}レイヤー/フレーム）:\n{}",
        input.display(),
        lines.len().max(1),
        text.trim()
    )
}

#[async_trait]
impl Tool for ImageToolsTool {
    fn name(&self) -> &'static str {
        "image_tools"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "psd",
            "xcf",
            "heic",
            "heif",
            "tiff",
            "webp",
            "avif",
            "画像変換",
            "pngに",
            "png化",
            "jpgに",
            "jpegに",
            "リサイズ",
            "縮小して",
            "画像情報",
            "レイヤー",
        ]
    }

    fn label(&self) -> &'static str {
        "画像ファイル変換"
    }

    fn description(&self) -> String {
        "ImageMagick（同梱→PATH。cargo xtask setup tools）で PSD/XCF/HEIC/TIFF/WebP/AVIF 等の画像を PNG/JPG などへ変換し、寸法・レイヤー等の情報も取得する。PSD はレイヤー統合ビューを変換。".to_string()
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "このPSDをPNGに変換して",
            "HEICの写真をJPGにして",
            "この画像の情報（サイズやレイヤー）を見せて",
        ]
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["convert", "info"], "description": "convert=形式変換、info=画像情報取得。" },
                "input_path": { "type": "string", "description": "入力画像（psd/xcf/heic/tiff/webp/avif/png/jpg 等）。" },
                "output_path": { "type": "string", "description": "convert の出力先。拡張子で形式を決める（例: out.png）。" },
                "resize": { "type": "string", "description": "リサイズ指定（例: 800x600, 50%）。省略時は原寸。" },
                "quality": { "type": "integer", "description": "JPG/WebP 品質（1〜100）。" }
            },
            "required": ["action", "input_path"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_MAGICK_PATH",
                "ImageMagick 実行ファイルパス（空ならPicoAgent内→PATH の magick/convert）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_IMAGE_TOOLS_TIMEOUT_SECS",
                "画像操作タイムアウト秒（5〜300）",
                "text",
                "120",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let timeout = cfg
            .get("PICOAGENT_IMAGE_TOOLS_TIMEOUT_SECS", "120")
            .parse::<u64>()
            .ok()
            .filter(|value| (5..=300).contains(value))
            .unwrap_or(120);
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "convert" => run_convert(cfg, &args, timeout).await,
            "info" => run_info(cfg, &args, timeout).await,
            _ => "エラー: action は convert/info のいずれかを指定してください。".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// 実行に使う ImageMagick を、**スキル本体とまったく同じ解決順**（設定 → 同梱 → PATH）で決める。
    ///
    /// 以前は素の `magick` / `convert` を PATH から探していたため、このプロジェクトの
    /// 既定構成（`cargo xtask setup tools` で `resources/bundled/tools/imagemagick/` へ同梱）
    /// では**常にスキップされ、E2E が一度も走っていなかった**。
    /// 名乗りの確認は `is_imagemagick` に任せる（Windows の `System32\convert.exe` 対策）。
    fn resolve_imagemagick(cfg: &crate::config::Config) -> Option<String> {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let program = super::magick_program(cfg, "convert").await;
            super::is_imagemagick(&program).await.then_some(program)
        })
    }

    // ImageMagick がある環境だけで走る E2E（無ければ即成功で抜ける）。
    // PSD を生成 → PNG へ変換 → 情報取得まで一連の流れを確認する。
    #[test]
    fn psdをpngへ変換して情報取得できる() {
        let dir =
            std::env::temp_dir().join(format!("picoagent-image-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &dir.to_string_lossy());
        let Some(program) = resolve_imagemagick(&cfg) else {
            eprintln!("ImageMagick が無いためスキップ");
            return;
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();

        // テスト用 PSD を ImageMagick 自身で生成する（v7 magick → v6 convert の順）。
        let psd = dir.join("sample.psd");
        let status = std::process::Command::new(&program)
            .args(["-size", "16x16", "xc:red"])
            .arg(&psd)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(psd.exists());

        let args = json!({ "action": "convert", "input_path": "sample.psd", "output_path": "sample.png", "resize": "8x8" });
        let result = runtime.block_on(super::run_convert(&cfg, &args, 120));
        assert!(result.contains("変換しました"), "変換結果: {result}");
        let png = std::fs::read(dir.join("sample.png")).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n"); // PNG シグネチャ

        let args = json!({ "action": "info", "input_path": "sample.psd" });
        let result = runtime.block_on(super::run_info(&cfg, &args, 120));
        assert!(result.contains("画像情報"), "情報結果: {result}");
        assert!(result.to_lowercase().contains("psd"), "情報結果: {result}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

inventory::submit! { ToolReg { make: || Box::new(ImageToolsTool) } }
