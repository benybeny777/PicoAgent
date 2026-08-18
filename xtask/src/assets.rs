// xtask/src/assets.rs - キャラ同期・UI 配信ビルド・残存プロセス停止。
//
// 旧 `scripts/sync-characters.mjs` / `build-ui-dist.mjs` / `stop-stale.mjs` の置き換え。

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::util::{copy_dir, list_files, log, repo_root, warn};

/// ルートの `characters/`（正本）を webview 配信先 `ui/assets/characters/` へ同期する。
///
/// キャラは扱いやすいようプロジェクト直下を正本にしているが、webview は
/// `frontendDist`（`ui/`）配下しか配信できないため、dev / build 前にコピーして配る。
/// 生成物は `.gitignore` 対象。
pub fn sync_characters() -> Result<()> {
    let root = repo_root();
    let src = root.join("characters");
    let dest = root.join("ui/assets/characters");
    if !src.exists() {
        bail!("キャラクターの正本が見つかりません: {}", src.display());
    }
    // 配信先を消す前に判定する。消してから失敗すると、キャラが1体も居ない状態が残り、
    // 次の `dev` / `build` は `ui-dist/` を作れないままになる。
    let unfetched = unfetched_live2d_characters(&src)?;
    fs::remove_dir_all(&dest).ok();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    // live2d-src/ と expression-patches/ は制作素材で実行時に使わないため配信へ含めない。
    // モデル未取得の Live2D サンプルも、正本直下のフォルダごと外す。
    copy_dir(&src, &dest, &|path| {
        if is_character_source_only(path) {
            return true;
        }
        path.parent() == Some(src.as_path())
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| unfetched.iter().any(|id| id == name))
    })?;
    if !unfetched.is_empty() {
        drop_characters_from_index(&dest, &unfetched)?;
        warn(
            "sync-characters",
            format!(
                "Live2Dサンプル{}体はモデルが手元に無いため同期から外しました（使うなら cargo xtask setup live2d-samples）: {}",
                unfetched.len(),
                unfetched.join(", ")
            ),
        );
    }
    stamp_character_asset_versions(&dest)?;
    log(
        "sync-characters",
        format!("{} -> {} を同期しました", src.display(), dest.display()),
    );
    Ok(())
}

/// モデル一式が手元に無い Live2D キャラのIDを集める。
///
/// `characters/*/live2d/` は再配布条件があるため Git 管理外で、
/// `cargo xtask setup live2d-samples` を実行した環境にしか無い。一方 `character.json` は
/// Git 管理下にあり Live2D を宣言しているため、clone 直後は「宣言だけあってモデルが無い」
/// 状態になる。ここで外しておかないと同期が中断し、`ui-dist/` が作られず
/// `tauri::generate_context!` がコンパイル時に落ちて**アプリが起動しなくなる**。
///
/// 宣言したモデルが**1つも無い**ものだけを未取得と見なす。一部だけ欠けているのは
/// 取得が壊れているか書き出し漏れなので、外さずに後段の検証でエラーにする。
fn unfetched_live2d_characters(src: &Path) -> Result<Vec<String>> {
    let mut character_dirs = fs::read_dir(src)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect::<Vec<_>>();
    character_dirs.sort();

    let mut unfetched = Vec::new();
    for character_dir in character_dirs {
        let manifest_path = character_dir.join("character.json");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?).with_context(|| {
                format!("character.json を読めません: {}", manifest_path.display())
            })?;
        if manifest.get("live2d").is_none() {
            continue;
        }
        // 定義自体が壊れている場合は判定せず、検証側の詳しいエラーへ委ねる。
        let Ok(models) = live2d_model_paths(&manifest) else {
            continue;
        };
        if models
            .iter()
            .any(|model| character_dir.join(model).is_file())
        {
            continue;
        }
        if let Some(id) = character_dir.file_name().and_then(|name| name.to_str()) {
            unfetched.push(id.to_string());
        }
    }
    Ok(unfetched)
}

/// 同期から外したキャラを、配信先の `index.json` からも取り除く。
///
/// 一覧に残したままだと、設定画面が「選べるのに読み込めないキャラ」を見せてしまう。
fn drop_characters_from_index(dest: &Path, dropped: &[String]) -> Result<()> {
    let index_path = dest.join("index.json");
    let mut index: serde_json::Value = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    let characters = index
        .get_mut("characters")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("{} にcharacters配列がありません", index_path.display()))?;
    characters.retain(|character| {
        character
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|id| !dropped.iter().any(|removed| removed == id))
    });
    let Some(first) = characters
        .first()
        .and_then(|character| character.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    else {
        bail!(
            "同期できるキャラクターが1体もありません: {}",
            index_path.display()
        );
    };
    // 既定キャラを外した場合だけ、残っている先頭へ寄せる（キャラ不在で起動しないため）。
    if index
        .get("default")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|default| dropped.iter().any(|removed| removed == default))
    {
        index["default"] = serde_json::Value::String(first);
    }
    fs::write(
        &index_path,
        format!("{}\n", serde_json::to_string_pretty(&index)?),
    )?;
    Ok(())
}

/// 販売設定で明示したキャラだけを配信先へ同期する。
///
/// 開発用の公式サンプルや制作途中モデルを誤って販売物へ含めないため、通常同期とは
/// 別入口にする。元のcharacters/は変更せず、生成先のindex.jsonだけを絞り込む。
pub fn sync_distribution_characters(included: &[String]) -> Result<()> {
    let root = repo_root();
    let src = root.join("characters");
    let dest = root.join("ui/assets/characters");
    fs::remove_dir_all(&dest).ok();
    fs::create_dir_all(&dest)?;

    let index_path = src.join("index.json");
    let mut index: serde_json::Value = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    let characters = index
        .get_mut("characters")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("characters/index.json にcharacters配列がありません"))?;
    characters.retain(|character| {
        character
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| included.iter().any(|allowed| allowed == id))
    });
    if characters.len() != included.len() {
        bail!(
            "販売設定のキャラ件数とindex.jsonの一致件数が異なります: 設定{}件 / 一致{}件",
            included.len(),
            characters.len()
        );
    }
    let default = index
        .get("default")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !included.iter().any(|id| id == default) {
        index["default"] = serde_json::Value::String(included[0].clone());
    }
    fs::write(
        dest.join("index.json"),
        format!("{}\n", serde_json::to_string_pretty(&index)?),
    )?;

    for id in included {
        copy_dir(&src.join(id), &dest.join(id), &is_character_source_only)?;
    }
    let placeholder = src.join("_preview-placeholder.png");
    if placeholder.is_file() {
        fs::copy(placeholder, dest.join("_preview-placeholder.png"))?;
    }
    stamp_character_asset_versions(&dest)?;
    log(
        "sync-characters",
        format!(
            "販売対象{}体だけを同期しました: {}",
            included.len(),
            included.join(", ")
        ),
    );
    Ok(())
}

fn is_character_source_only(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| matches!(name, "live2d-src" | "expression-patches"))
    })
}

/// 同期済みキャラの全内容から自動版を作り、manifestへ書き込む。
///
/// 手書き版だけでは画像差し替え時に更新を忘れ、WebViewが旧画像を表示し続ける。
/// 配信するファイル自体のハッシュなら、通常の同期だけでURLが変わりキャッシュが切り替わる。
fn stamp_character_asset_versions(dest: &Path) -> Result<()> {
    let mut character_dirs = fs::read_dir(dest)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("character.json").is_file())
        .collect::<Vec<_>>();
    character_dirs.sort();

    for character_dir in character_dirs {
        let manifest_path = character_dir.join("character.json");
        let version = character_content_version(&character_dir)?;
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
        validate_character_asset_generation(&character_dir, &manifest)?;
        manifest["assetVersion"] = serde_json::Value::String(format!("auto-{version}"));
        fs::write(
            manifest_path,
            format!("{}\n", serde_json::to_string_pretty(&manifest)?),
        )?;
    }
    Ok(())
}

const THA3_MOTION_LAYERS: [&str; 12] = [
    "idle_breathe",
    "idle_left",
    "idle_right",
    "sad",
    "sad_blink",
    "talking_left_closed",
    "talking_left_open",
    "talking_right_closed",
    "talking_right_open",
    "thinking_blink",
    "thinking_left",
    "thinking_right",
];

const SHARP_EXPRESSION_MINIMUM_SIZE: (u32, u32) = (1780, 3600);

fn png_dimensions(path: &Path) -> Result<(u32, u32)> {
    let bytes = fs::read(path)?;
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        bail!("PNGヘッダーを読み取れません: {}", path.display());
    }
    Ok((
        u32::from_be_bytes(bytes[16..20].try_into()?),
        u32::from_be_bytes(bytes[20..24].try_into()?),
    ))
}

fn validate_sharp_expression_pack(
    character_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<()> {
    if manifest
        .get("animationQualityMode")
        .and_then(serde_json::Value::as_str)
        != Some("sharp-expression")
    {
        return Ok(());
    }
    let stale = THA3_MOTION_LAYERS
        .iter()
        .filter(|name| {
            character_dir
                .join("layers")
                .join(format!("{name}.png"))
                .is_file()
        })
        .copied()
        .collect::<Vec<_>>();
    if !stale.is_empty() {
        bail!(
            "{} はsharp-expressionですが旧THA3全身レイヤーが残っています: {}",
            character_dir.display(),
            stale.join(", ")
        );
    }
    for name in FULL_MOTION_LAYERS {
        let path = character_dir.join("layers").join(format!("{name}.png"));
        let (width, height) = png_dimensions(&path)?;
        if width < SHARP_EXPRESSION_MINIMUM_SIZE.0 || height < SHARP_EXPRESSION_MINIMUM_SIZE.1 {
            bail!(
                "{} はsharp-expressionですが{}が低解像度です: {}x{}（最低{}x{}）",
                character_dir.display(),
                name,
                width,
                height,
                SHARP_EXPRESSION_MINIMUM_SIZE.0,
                SHARP_EXPRESSION_MINIMUM_SIZE.1
            );
        }
    }
    Ok(())
}

const FULL_MOTION_LAYERS: &[&str] = &[
    "base",
    "blink_half",
    "blink_closed",
    "mouth_closed",
    "mouth_open",
    "gaze_left",
    "gaze_right",
    "expression_happy",
    "expression_sad",
];

fn validate_full_motion_pack(character_dir: &Path, manifest: &serde_json::Value) -> Result<()> {
    if manifest
        .get("motionProfile")
        .and_then(serde_json::Value::as_str)
        != Some("full")
    {
        return Ok(());
    }
    let missing = FULL_MOTION_LAYERS
        .iter()
        .filter(|name| {
            !character_dir
                .join("layers")
                .join(format!("{name}.png"))
                .is_file()
        })
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "{} はmotionProfile=fullですが共通モーションレイヤーが不足しています: {}",
            character_dir.display(),
            missing.join(", ")
        );
    }
    Ok(())
}

fn validate_character_asset_generation(
    character_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<()> {
    if manifest.get("live2d").is_some() {
        return validate_live2d_model_sets(character_dir, manifest);
    }
    validate_sharp_expression_pack(character_dir, manifest)?;
    validate_full_motion_pack(character_dir, manifest)?;
    let generation = manifest
        .get("frameGeneration")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            anyhow::anyhow!("{} にframeGenerationがありません", character_dir.display())
        })?;
    let expected_prefix = format!("frames/{generation}/");
    let animations = manifest
        .get("animations")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("{} にanimationsがありません", character_dir.display()))?;
    for (name, animation) in animations {
        let paths = animation
            .get("framePaths")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("animations.{name} にframePathsがありません"))?;
        for path in paths {
            let path = path.as_str().unwrap_or("");
            if !path.starts_with(&expected_prefix) {
                bail!(
                    "{} の{}が現世代{}以外を参照しています: {}",
                    character_dir.display(),
                    name,
                    generation,
                    path
                );
            }
        }
    }
    let frame_root = character_dir.join("frames");
    let entries = fs::read_dir(&frame_root)?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if entries.len() != 1 || entries[0] != generation {
        bail!(
            "{} のframesに複数世代があります: {}",
            character_dir.display(),
            entries.join(", ")
        );
    }
    Ok(())
}

/// マニフェストが宣言する Live2D モデルのパス一覧（`live2d.model` と `live2d.models[].path`）。
///
/// 検証（実在確認）と同期（未取得サンプルの判定）が同じ定義を見るよう1か所にまとめる。
fn live2d_model_paths(manifest: &serde_json::Value) -> Result<Vec<String>> {
    let live2d = &manifest["live2d"];
    let mut models = Vec::new();
    if let Some(path) = live2d.get("model").and_then(serde_json::Value::as_str) {
        models.push(path.to_string());
    }
    if let Some(sets) = live2d.get("models").and_then(serde_json::Value::as_array) {
        for set in sets {
            let path = set
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if path.is_empty() {
                bail!("Live2Dモデル世代にpathがありません");
            }
            models.push(path.to_string());
        }
    }
    if models.is_empty() {
        bail!("Live2Dモデル定義が空です");
    }
    Ok(models)
}

fn validate_live2d_model_sets(character_dir: &Path, manifest: &serde_json::Value) -> Result<()> {
    let models = live2d_model_paths(manifest)
        .with_context(|| format!("{} のLive2D定義", character_dir.display()))?;
    for model in models {
        if !character_dir.join(&model).is_file() {
            bail!(
                "{} のLive2Dモデル一式がありません: {model}",
                character_dir.display()
            );
        }
    }
    Ok(())
}

fn character_content_version(character_dir: &Path) -> Result<String> {
    let mut files = list_files(character_dir);
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(character_dir).unwrap_or(&file);
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0]);
        let mut bytes = fs::read(&file)?;
        if relative == Path::new("character.json") {
            let mut manifest: serde_json::Value = serde_json::from_slice(&bytes)?;
            if let Some(object) = manifest.as_object_mut() {
                object.remove("assetVersion");
            }
            bytes = serde_json::to_vec(&manifest)?;
        }
        hasher.update(bytes);
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize())[..16].to_string())
}

/// `ui/` を Tauri 配信用 `ui-dist/` へコピーする。
///
/// release では依存追加なしで HTML/CSS/JS の余分な空白とコメントを落とし、
/// 配布物から読みやすいソースを減らす（解析を不可能にするものではない）。
///
/// `beta` は開発中ベータ版のときだけ真にする。偽なら画面の BETA 表示を取り除く。
pub fn build_ui_dist(release: bool, beta: bool) -> Result<()> {
    let root = repo_root();
    let src = root.join("ui");
    let dest = root.join("ui-dist");
    if !src.exists() {
        bail!("ui が見つかりません: {}", src.display());
    }
    fs::remove_dir_all(&dest).ok();
    copy_dir(&src, &dest, &|_| false)?;
    // READMEで実表示を検証した8形式の画像を、設定画面の選択カードにも使う。
    // 正本を二重管理せず、配信用生成物へだけまとめてコピーする。
    let preview_src = root.join("docs/images");
    let preview_dest = dest.join("assets/layout-previews");
    fs::create_dir_all(&preview_dest)?;
    for id in [
        "pet",
        "mascot",
        "chat",
        "bar",
        "workspace",
        "stream",
        "widget",
    ] {
        let source = preview_src.join(format!("layout-shot-{id}.webp"));
        if !source.exists() {
            bail!(
                "表示形式のプレビュー画像が見つかりません: {}",
                source.display()
            );
        }
        fs::copy(source, preview_dest.join(format!("{id}.webp")))?;
    }

    if !beta {
        strip_beta_badge(&dest)?;
    }

    if release {
        for file in list_files(&dest) {
            let ext = file
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let minified = match ext.as_str() {
                "html" => minify_html(&text),
                "css" => minify_css(&text),
                "js" => minify_js(&text),
                _ => continue,
            };
            fs::write(&file, minified)?;
        }
    }

    // 配信する中身が確定してから版を打つ（release の最小化後の内容でハッシュを取る）。
    stamp_asset_versions(&dest)?;

    log(
        "build-ui-dist",
        format!(
            "ui -> ui-dist ({}{})",
            if release { "release" } else { "dev" },
            if beta { " / beta" } else { "" }
        ),
    );
    Ok(())
}

/// 配信用 `index.html` から BETA 表示を取り除く。
///
/// BETA 表示は `ui/index.html` に直接書いてあり、`ui/` はどのビルドでもそのまま
/// `ui-dist/` へ写る。剥がさないと販売版にも「BETA」が出続けるため、ベータ以外の
/// ビルドはここで確実に落とす（表示の有無をビルド種別だけで決める）。
fn strip_beta_badge(dest: &std::path::Path) -> Result<()> {
    let index = dest.join("index.html");
    let text = fs::read_to_string(&index)
        .with_context(|| format!("index.html を読めません: {}", index.display()))?;
    let Some(stripped) = remove_beta_badge(&text) else {
        return Ok(()); // 目印が無いソース（将来 BETA 表示を廃止した場合）は何もしない
    };
    fs::write(&index, stripped)?;
    Ok(())
}

/// `<span class="beta-badge" …>BETA</span>` を1つ取り除く。見つからなければ `None`。
fn remove_beta_badge(text: &str) -> Option<String> {
    const OPEN: &str = "<span class=\"beta-badge\"";
    let start = text.find(OPEN)?;
    let end = text[start..].find("</span>")? + start + "</span>".len();
    let mut stripped = String::with_capacity(text.len());
    stripped.push_str(&text[..start]);
    stripped.push_str(text[end..].trim_start_matches(['\r', '\n', ' ', '\t']));
    Some(stripped)
}

/// HTML が読み込むローカルの CSS / JS へ、内容から決まる `?v=…` を付ける。
///
/// 手書きの版番号は付け忘れ・書き間違いが起きる（実際に `styles.css` と `renderer.js` で
/// 版が食い違っていた）。内容ハッシュにしておけば、変更したファイルだけが確実に読み直され、
/// webview のキャッシュに古い CSS が残らない。
fn stamp_asset_versions(dest: &std::path::Path) -> Result<()> {
    for file in list_files(dest) {
        if file.extension().map(|e| e != "html").unwrap_or(true) {
            continue;
        }
        let Some(dir) = file.parent().map(|dir| dir.to_path_buf()) else {
            continue;
        };
        let text = fs::read_to_string(&file)?;
        let stamped = rewrite_asset_refs(&text, &|relative| {
            let target = dir.join(relative);
            fs::read(&target).ok().map(|bytes| content_version(&bytes))
        });
        if stamped != text {
            fs::write(&file, stamped)?;
        }
    }
    Ok(())
}

/// 内容から決まる短い版文字列（FNV-1a。外部クレートを増やさない）。
fn content_version(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
}

/// HTML 内の `href="…"` / `src="…"` のうち、同梱の CSS / JS を指すものへ版を付け直す。
fn rewrite_asset_refs(html: &str, version_of: &dyn Fn(&str) -> Option<String>) -> String {
    const ATTRIBUTES: [&str; 2] = ["href=\"", "src=\""];
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    loop {
        let Some((index, attribute)) = ATTRIBUTES
            .iter()
            .filter_map(|attribute| rest.find(attribute).map(|index| (index, *attribute)))
            .min_by_key(|(index, _)| *index)
        else {
            out.push_str(rest);
            return out;
        };
        let value_start = index + attribute.len();
        out.push_str(&rest[..value_start]);
        let after = &rest[value_start..];
        let Some(end) = after.find('"') else {
            out.push_str(after);
            return out;
        };
        out.push_str(&stamped_reference(&after[..end], version_of));
        out.push('"');
        rest = &after[end + 1..];
    }
}

/// 参照1つ分の書き換え。外部 URL・絶対パス・CSS/JS 以外はそのまま返す。
fn stamped_reference(value: &str, version_of: &dyn Fn(&str) -> Option<String>) -> String {
    let path = value.split(['?', '#']).next().unwrap_or(value);
    let lower = path.to_lowercase();
    if !(lower.ends_with(".css") || lower.ends_with(".js")) {
        return value.to_string();
    }
    if path.starts_with('/') || path.contains("://") || lower.starts_with("data:") {
        return value.to_string();
    }
    match version_of(path) {
        Some(version) => format!("{path}?v={version}"),
        None => value.to_string(),
    }
}

fn minify_html(text: &str) -> String {
    let without_comments = strip_between(text, "<!--", "-->");
    let mut out = String::with_capacity(without_comments.len());
    let chars = without_comments.chars();
    let mut pending_space = false;
    for ch in chars {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            // タグ間の空白は落とし、それ以外は1つの空白へまとめる。
            if !(out.is_empty() || ch == '<' && out.ends_with('>')) {
                out.push(' ');
            }
            pending_space = false;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn minify_css(text: &str) -> String {
    let without_comments = strip_between(text, "/*", "*/");
    let mut collapsed = String::with_capacity(without_comments.len());
    let mut in_space = false;
    for ch in without_comments.chars() {
        if ch.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        in_space = false;
        collapsed.push(ch);
    }
    // 記号の前後の空白を落とす。
    let mut out = String::with_capacity(collapsed.len());
    // `calc(100% + 16px)` の + / - は前後の空白が必須。ここで削ると宣言全体が
    // 無効になり、release 配布版だけキャラ幅が0になるため演算子は対象に含めない。
    let symbols = ['{', '}', ':', ';', ',', '>', '~'];
    let chars: Vec<char> = collapsed.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == ' ' {
            let prev = out.chars().last();
            let next = chars.get(index + 1).copied();
            let touching_symbol = prev.is_some_and(|c| symbols.contains(&c))
                || next.is_some_and(|c| symbols.contains(&c));
            if touching_symbol {
                index += 1;
                continue;
            }
        }
        out.push(ch);
        index += 1;
    }
    out.replace(";}", "}").trim().to_string()
}

fn minify_js(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `open` と `close` に囲まれた範囲を落とす（コメント除去用の素朴な実装）。
fn strip_between(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        match after.find(close) {
            Some(end) => rest = &after[end + close.len()..],
            // 閉じが無い場合は残り全部がコメント扱い。
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// 前回起動の残存プロセスを停止する。
///
/// 残っていると同梱 DLL を掴んだままになり、ビルドが `os error 32` で失敗したり、
/// 新しいインスタンスの WebView 生成と競合して起動直後に落ちたりする。
pub fn stop_stale() -> Result<()> {
    // PicoAgent 管理下のものだけを対象にする（同名の無関係プロセスを巻き込まない）。
    let targets = ["picoagent", "llama-server", "whisper-cli"];
    let mut stopped = Vec::new();

    for name in targets {
        let killed = if cfg!(windows) {
            Command::new("taskkill")
                .args(["/IM", &format!("{name}.exe"), "/T", "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        } else {
            // 完全一致だけを対象にする（-x）。
            Command::new("pkill")
                .args(["-x", name])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
        if killed {
            stopped.push(name);
        }
    }

    // Aivis は PicoAgent 管理下のパスから起動したものだけを止める。
    stop_managed_aivis(&mut stopped);

    if stopped.is_empty() {
        log("stop-stale", "停止対象の残存プロセスはありません");
    } else {
        log(
            "stop-stale",
            format!("停止しました: {}", stopped.join(", ")),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn stop_managed_aivis(stopped: &mut Vec<&'static str>) {
    // 実行パスに PicoAgent 管理下の bundled/aivis を含むものだけを止める。
    let ok = Command::new("pkill")
        .args(["-f", "PicoAgent.*bundled.*aivis.*run"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if ok {
        stopped.push("aivis");
    }
}

#[cfg(not(unix))]
fn stop_managed_aivis(stopped: &mut Vec<&'static str>) {
    let ok = Command::new("taskkill")
        .args(["/IM", "run.exe", "/T", "/F"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if ok {
        stopped.push("aivis");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        character_content_version, content_version, drop_characters_from_index,
        is_character_source_only, remove_beta_badge, rewrite_asset_refs,
        stamp_character_asset_versions, unfetched_live2d_characters,
        validate_character_asset_generation,
    };

    #[test]
    fn 制作用素材は配信コピーから外す() {
        assert!(is_character_source_only(std::path::Path::new(
            "characters/mugi/expression-patches/blink.png"
        )));
        assert!(is_character_source_only(std::path::Path::new(
            "characters/hiyori/live2d-src/source.psd"
        )));
        assert!(!is_character_source_only(std::path::Path::new(
            "characters/mugi/frames/idle.png"
        )));

        let ui = crate::util::repo_root().join("ui");
        let mut stack = vec![ui];
        let mut references = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().and_then(|name| name.to_str()) != Some("assets") {
                        stack.push(path);
                    }
                } else if matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("js" | "html" | "css")
                ) && fs::read_to_string(&path)
                    .is_ok_and(|text| text.contains("expression-patches"))
                {
                    references.push(path);
                }
            }
        }
        let mut character_stack = vec![crate::util::repo_root().join("characters")];
        while let Some(dir) = character_stack.pop() {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    character_stack.push(path);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && fs::read_to_string(&path)
                        .is_ok_and(|text| text.contains("expression-patches"))
                {
                    references.push(path);
                }
            }
        }
        assert!(
            references.is_empty(),
            "実行UIから制作用素材を参照しています: {references:?}"
        );
    }

    /// `characters/<id>/` を1体ぶん作る。`models` は `character.json` へ書く宣言、
    /// `present` は実際に置くファイル（宣言だけあってファイルが無い状態を作れるようにする）。
    fn live2d_character(src: &std::path::Path, id: &str, models: &[&str], present: &[&str]) {
        let dir = src.join(id);
        fs::create_dir_all(&dir).unwrap();
        let declared = models
            .iter()
            .map(|model| format!(r#"{{"path":"{model}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            dir.join("character.json"),
            format!(r#"{{"id":"{id}","live2d":{{"models":[{declared}]}}}}"#),
        )
        .unwrap();
        for model in present {
            let path = dir.join(model);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"model").unwrap();
        }
    }

    /// `characters/*/live2d/` は再配布条件があるため Git 管理外で、clone 直後には無い。
    /// ここで外し損ねると同期が中断し、`ui-dist/` が作られずアプリが起動しなくなる。
    #[test]
    fn モデル未取得のlive2dサンプルは同期から外す() {
        let root =
            std::env::temp_dir().join(format!("picoagent-unfetched-live2d-{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let src = root.join("characters");
        fs::create_dir_all(&src).unwrap();

        // 宣言だけあってモデルが1つも無い＝未取得。
        live2d_character(&src, "hiyori", &["live2d/hiyori.model3.json"], &[]);
        // 取得済み＝外さない。
        live2d_character(
            &src,
            "mugi-live2d",
            &["live2d/mugi.model3.json"],
            &["live2d/mugi.model3.json"],
        );
        // 一部だけ欠けているのは壊れた状態なので、外さず検証側のエラーに任せる。
        live2d_character(
            &src,
            "half",
            &["live2d/sdk4/a.model3.json", "live2d/sdk5/b.model3.json"],
            &["live2d/sdk4/a.model3.json"],
        );
        // Live2D を宣言しない PNG キャラは対象外。
        let png = src.join("mugi");
        fs::create_dir_all(&png).unwrap();
        fs::write(png.join("character.json"), r#"{"id":"mugi"}"#).unwrap();

        assert_eq!(unfetched_live2d_characters(&src).unwrap(), vec!["hiyori"]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// 外したキャラを一覧に残すと、設定画面が「選べるのに読み込めないキャラ」を見せてしまう。
    #[test]
    fn 外したキャラは一覧と既定からも消す() {
        let dest =
            std::env::temp_dir().join(format!("picoagent-drop-index-{}", std::process::id()));
        fs::remove_dir_all(&dest).ok();
        fs::create_dir_all(&dest).unwrap();
        fs::write(
            dest.join("index.json"),
            r#"{"default":"hiyori","characters":[{"id":"hiyori"},{"id":"mugi"},{"id":"rin"}]}"#,
        )
        .unwrap();

        drop_characters_from_index(&dest, &["hiyori".to_string()]).unwrap();
        let index: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dest.join("index.json")).unwrap()).unwrap();
        let ids = index["characters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|character| character["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["mugi", "rin"]);
        // 既定キャラを外したときだけ、残っている先頭へ寄せる。
        assert_eq!(index["default"].as_str(), Some("mugi"));

        // 全部外すと選べるキャラが居なくなるため、黙って空の一覧を配らない。
        assert!(drop_characters_from_index(&dest, &["mugi".into(), "rin".into()]).is_err());
        fs::remove_dir_all(&dest).unwrap();
    }

    #[test]
    fn 同梱のcssとjsへ内容版を付ける() {
        let html =
            r#"<link rel="stylesheet" href="styles.css" /><script src="renderer.js"></script>"#;
        let stamped = rewrite_asset_refs(html, &|relative| match relative {
            "styles.css" => Some("aaaaaaaa".to_string()),
            "renderer.js" => Some("bbbbbbbb".to_string()),
            _ => None,
        });
        assert_eq!(
            stamped,
            r#"<link rel="stylesheet" href="styles.css?v=aaaaaaaa" /><script src="renderer.js?v=bbbbbbbb"></script>"#
        );
    }

    #[test]
    fn 手書きの版と外部参照は壊さない() {
        // 手書きの `?v=…` は内容版へ置き換える（付け忘れ・食い違いを残さない）。
        let stamped =
            rewrite_asset_refs(r#"<script src="renderer.js?v=20260807"></script>"#, &|_| {
                Some("cccccccc".to_string())
            });
        assert_eq!(stamped, r#"<script src="renderer.js?v=cccccccc"></script>"#);

        // 外部 URL・存在しないファイル・CSS/JS 以外は素通し。
        let untouched =
            r#"<a href="https://example.com/app.js"><img src="icon.png"><link href="missing.css">"#;
        assert_eq!(
            rewrite_asset_refs(untouched, &|relative| (relative == "styles.css")
                .then(|| "dddddddd".to_string())),
            untouched
        );
    }

    /// BETA 表示は開発中ベータ版だけの目印。`ui/index.html` に直接書いてあるため、
    /// 剥がす処理が実際の記述と噛み合っていないと販売版へそのまま出てしまう。
    #[test]
    fn ベータ表示は販売ビルドから取り除ける() {
        let index = fs::read_to_string(crate::util::repo_root().join("ui/index.html")).unwrap();
        assert!(
            index.contains("beta-badge"),
            "ui/index.html に BETA 表示がありません。廃止したなら strip_beta_badge も消すこと"
        );
        let stripped = remove_beta_badge(&index).expect("BETA 表示を取り除けません");
        assert!(!stripped.contains("beta-badge"));
        assert!(remove_beta_badge(&stripped).is_none());
        // 目印が無い HTML は書き換え対象にしない。
        assert!(remove_beta_badge("<main></main>").is_none());
    }

    #[test]
    fn 内容が変われば版も変わる() {
        assert_eq!(content_version(b"body{}"), content_version(b"body{}"));
        assert_ne!(content_version(b"body{}"), content_version(b"body{ }"));
        assert_eq!(content_version(b"body{}").len(), 8);
    }

    #[test]
    fn css圧縮でcalc演算子の空白を保つ() {
        assert_eq!(
            super::minify_css(".a { right: calc(var(--width) + 16px); }").as_str(),
            ".a{right:calc(var(--width) + 16px)}"
        );
    }

    #[test]
    fn キャラ素材更新でmanifestの自動版が変わる() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-character-version-{}",
            std::process::id()
        ));
        let character = root.join("sample");
        fs::create_dir_all(&character).unwrap();
        fs::write(
            character.join("character.json"),
            r#"{"id":"sample","assetVersion":"manual","frameGeneration":"current","animations":{"idle":{"framePaths":["frames/current/a.png"]}}}"#,
        )
        .unwrap();
        fs::create_dir_all(character.join("frames/current")).unwrap();
        fs::write(character.join("frames/current/a.png"), b"frame").unwrap();
        fs::write(character.join("preview.png"), b"old image").unwrap();
        let old = character_content_version(&character).unwrap();
        fs::write(character.join("preview.png"), b"new image").unwrap();
        let new = character_content_version(&character).unwrap();
        assert_ne!(old, new);

        stamp_character_asset_versions(&root).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(character.join("character.json")).unwrap())
                .unwrap();
        assert!(manifest["assetVersion"]
            .as_str()
            .unwrap()
            .starts_with("auto-"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pngキャラは単一フレーム世代だけを許可する() {
        let root =
            std::env::temp_dir().join(format!("picoagent-frame-generation-{}", std::process::id()));
        let character = root.join("sample");
        fs::create_dir_all(character.join("frames/current")).unwrap();
        fs::write(character.join("frames/current/a.png"), b"image").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{"frameGeneration":"current","animations":{"idle":{"framePaths":["frames/current/a.png"]}}}"#,
        )
        .unwrap();
        validate_character_asset_generation(&character, &manifest).unwrap();

        fs::create_dir_all(character.join("frames/old")).unwrap();
        assert!(validate_character_asset_generation(&character, &manifest).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn フルモーション宣言は全共通レイヤーを必須にする() {
        let root =
            std::env::temp_dir().join(format!("picoagent-full-motion-{}", std::process::id()));
        let character = root.join("sample");
        fs::create_dir_all(character.join("layers")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"motionProfile":"full"}"#).unwrap();
        assert!(super::validate_full_motion_pack(&character, &manifest).is_err());
        for name in super::FULL_MOTION_LAYERS {
            fs::write(
                character.join("layers").join(format!("{name}.png")),
                b"image",
            )
            .unwrap();
        }
        super::validate_full_motion_pack(&character, &manifest).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 高精細表情キャラは全レイヤーの最低寸法を満たす() {
        let root =
            std::env::temp_dir().join(format!("picoagent-sharp-size-{}", std::process::id()));
        let character = root.join("sample");
        fs::create_dir_all(character.join("layers")).unwrap();
        let mut header = Vec::from(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice());
        header.extend_from_slice(&890_u32.to_be_bytes());
        header.extend_from_slice(&1800_u32.to_be_bytes());
        for name in super::FULL_MOTION_LAYERS {
            fs::write(
                character.join("layers").join(format!("{name}.png")),
                &header,
            )
            .unwrap();
        }
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"animationQualityMode":"sharp-expression"}"#).unwrap();
        assert!(super::validate_sharp_expression_pack(&character, &manifest).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
