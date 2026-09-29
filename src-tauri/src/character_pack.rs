// character_pack.rs - キャラクターパックの検証（アプリと xtask の共通ルール）。
//
// 検証はアプリ（設定画面からの導入）と `cargo xtask character-pack` の両方が使う。
// 規則が2箇所に分かれると、CLI で通ったパックがアプリで弾かれる（逆も）ため、
// **このファイルだけをルールの正本にする**。xtask 側は `#[path]` でこのファイルを
// 直接取り込むので、依存は `std` / `serde_json` / `anyhow` だけに保つこと
// （`crate::` のアプリ内部モジュールを参照しない）。
//
// 守りたいのは「他人が作った zip を開いても安全であること」。
// シンボリックリンク・パック外への参照・巨大ファイル・未知の拡張子を拒否し、
// Live2D は model3.json が参照するファイルの実在まで確かめる
// （欠けたまま導入すると、キャラを選んだ瞬間に画面が崩れる）。

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;

#[path = "mmd_character_pack.rs"]
mod mmd_character_pack;

pub(crate) fn mmd_texture_references(bytes: &[u8]) -> Result<Vec<String>> {
    mmd_character_pack::inspect(bytes)
}

/// パックに置ける拡張子。実行可能なものは入れない。
pub const ALLOWED_EXTENSIONS: &[&str] = &[
    "json",
    "png",
    "webp",
    "jpg",
    "jpeg",
    "moc3",
    "wav",
    "psd",
    "md",
    "txt",
    "exp3",
    "motion3",
    "physics3",
    "cdi3",
    "userdata3",
    "pose3",
    "pmx",
    "bmp",
    "tga",
    "sph",
    "spa",
];

pub const MAX_FILES: usize = 4000;
pub const MAX_TOTAL_BYTES: u64 = 250 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 40 * 1024 * 1024;
/// 非圧縮のMMDテクスチャだけは大きな原本をそのまま保持する。
pub fn file_limit(path: &Path) -> u64 {
    if ["bmp", "tga", "sph", "spa"].contains(&extension_of(path).as_str()) {
        128_000_000
    } else {
        MAX_FILE_BYTES
    }
}
pub const MAX_ID_CHARS: usize = 64;

/// キャラクターIDとして使ってよい文字列か。**このファイルが唯一の判定元。**
///
/// IDはそのままフォルダ名（`PicoAgent/characters/<id>/`）と設定キーの一部になるため、
/// 英数字・ハイフン・アンダースコアだけに絞って `.` や区切り文字を弾く。
///
/// 以前は同じ規則が6箇所へ写されており、`character_framing` と `stream_profile` の
/// 2つでは**長さ制限が抜けていた**（文字種だけ見ていた）。判定を写すと必ずどれかが
/// ずれるので、`character_pack::guard` が写しの復活を検出する。
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_ID_CHARS
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[derive(Default)]
pub struct PackStats {
    pub files: usize,
    pub bytes: u64,
}

/// パックを検証し、`character.json` の内容を返す。
pub fn validate(source: &Path) -> Result<Value> {
    if !source.is_dir() {
        bail!("パックフォルダが見つかりません: {}", source.display());
    }
    let canonical_root = source
        .canonicalize()
        .context("パックフォルダの実体を確認できません")?;
    let mut stats = PackStats::default();
    validate_tree(source, &canonical_root, &mut stats)?;

    let manifest_path = source.join("character.json");
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("{} を読めません", manifest_path.display()))?;
    let manifest: Value = serde_json::from_str(&text).context("character.json が不正なJSONです")?;

    let id = required_text(&manifest, "id")?;
    if !is_valid_id(id) {
        bail!("id は{MAX_ID_CHARS}文字以内の英数字・ハイフン・アンダースコアだけにしてください");
    }
    required_text(&manifest, "name")?;

    let live2d_model = manifest.pointer("/live2d/model").and_then(Value::as_str);
    let mmd_model = manifest.pointer("/mmd/model").and_then(Value::as_str);
    let mesh_rig = manifest.pointer("/meshRig/rig").and_then(Value::as_str);
    let idle_has_frames = manifest
        .pointer("/animations/idle")
        .is_some_and(animation_has_frames);
    if live2d_model.is_none() && mmd_model.is_none() && mesh_rig.is_none() && !idle_has_frames {
        bail!("live2d.model、mmd.model、meshRig.rig、animations.idle のいずれかが必要です");
    }
    for pointer in ["/boot/sleeping", "/live2d/model", "/meshRig/rig"] {
        if let Some(relative) = manifest.pointer(pointer).and_then(Value::as_str) {
            ensure_asset(source, relative)
                .with_context(|| format!("{pointer} が指すファイルがありません"))?;
        }
    }
    if let Some(model) = live2d_model {
        validate_live2d_model(source, model)?;
    }
    mmd_character_pack::validate(source, &manifest)?;
    validate_mesh_rig(source, &manifest)?;
    validate_animations(source, &manifest)?;
    validate_variants(source, &manifest)?;
    validate_secondary_motion(&manifest, "secondaryMotion")?;
    validate_spring_physics(&manifest, "springPhysics")?;
    validate_interactions(&manifest)?;
    Ok(manifest)
}

fn validate_mesh_rig(source: &Path, manifest: &Value) -> Result<()> {
    let Some(relative) = manifest.pointer("/meshRig/rig").and_then(Value::as_str) else {
        return Ok(());
    };
    let rig_path = source.join(relative);
    let rig: Value = serde_json::from_str(
        &std::fs::read_to_string(&rig_path)
            .with_context(|| format!("{} を読めません", rig_path.display()))?,
    )
    .context("meshRig.rig が不正なJSONです")?;
    let version = rig.get("version").and_then(Value::as_u64);
    let mode = rig.get("mode").and_then(Value::as_str);
    let legacy =
        version == Some(1) && matches!(mode, Some("single-sheet-grid-v1" | "layered-grid-v2"));
    let anime25d = version == Some(2) && mode == Some("anime25d-v1");
    let lvs_anime25d = rig.get("schema_version").and_then(Value::as_u64) == Some(3)
        && rig.get("profile").and_then(Value::as_str) == Some("lvs-anime25d-v1")
        && rig.get("eye_rig_version").and_then(Value::as_u64) == Some(3)
        && rig.get("lip_rig_version").and_then(Value::as_u64) == Some(1)
        && rig.get("scene_graph_version").and_then(Value::as_u64) == Some(1);
    if !legacy && !anime25d && !lvs_anime25d {
        bail!("meshRig.rig の形式に対応していません");
    }
    let root = rig_path
        .parent()
        .context("meshRig.rig の親フォルダがありません")?;
    if lvs_anime25d {
        validate_lvs_secondary_motion(&rig)?;
        let layers = rig
            .get("layers")
            .and_then(Value::as_object)
            .context("lvs-anime25d-v1 の layers が不正です")?;
        let graph = rig
            .get("scene_graph")
            .and_then(Value::as_array)
            .filter(|items| items.len() >= 6)
            .context("lvs-anime25d-v1 の scene_graph は6件以上必要です")?;
        let mut names = graph
            .iter()
            .map(|item| {
                item.get("layer")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .context("scene_graph.layer が不正です")
            })
            .collect::<Result<Vec<_>>>()?;
        names.extend(
            ["scene_face", "mouth_open", "mouth_closed"]
                .into_iter()
                .map(str::to_owned),
        );
        for side in ["left", "right"] {
            names.extend(
                [
                    "eye_base",
                    "eye_closed",
                    "eyelid_upper",
                    "eye_backplate",
                    "eye_iris",
                ]
                .into_iter()
                .map(|part| format!("{side}_{part}")),
            );
        }
        names.sort();
        names.dedup();
        for name in names {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
            {
                bail!("lvs-anime25d-v1 の素材名が不正です: {name}");
            }
            let layer = layers
                .get(&name)
                .with_context(|| format!("lvs-anime25d-v1 の必須素材がありません: {name}"))?;
            let expected_url = format!("/assets/rig2d/parts/{name}.png");
            if layer.get("url").and_then(Value::as_str) != Some(expected_url.as_str()) {
                bail!("lvs-anime25d-v1 の素材URLが不正です: {name}");
            }
            if !root.join("parts").join(format!("{name}.png")).is_file() {
                bail!("meshRigのテクスチャがありません: parts/{name}.png");
            }
        }
        return Ok(());
    }

    let mut textures = Vec::new();
    if legacy {
        textures.push(rig.get("base").and_then(Value::as_str).unwrap_or(""));
        if let Some(drawables) = rig.get("drawables").and_then(Value::as_object) {
            textures.extend(
                drawables
                    .values()
                    .filter_map(|item| item.get("texture").and_then(Value::as_str)),
            );
        }
    } else {
        let layers = rig
            .get("layers")
            .and_then(Value::as_array)
            .filter(|layers| !layers.is_empty())
            .context("anime25d-v1 の layers は1件以上必要です")?;
        textures.extend(
            layers
                .iter()
                .filter_map(|item| item.get("texture").and_then(Value::as_str)),
        );
        if textures.len() != layers.len() {
            bail!("anime25d-v1 の全レイヤーに texture が必要です");
        }
    }
    for texture in textures {
        if texture.is_empty()
            || texture.replace('\\', "/").contains("../")
            || texture.contains("source/")
        {
            bail!("meshRigの実行テクスチャパスが不正です: {texture}");
        }
        if !root.join(texture).is_file() {
            bail!("meshRigのテクスチャがありません: {texture}");
        }
    }
    Ok(())
}

fn validate_lvs_secondary_motion(rig: &Value) -> Result<()> {
    let version = rig.get("secondary_motion_version");
    let profile = rig.get("secondary_motion");
    if version.is_none() && profile.is_none() {
        return Ok(());
    }
    if version.and_then(Value::as_u64) != Some(1) {
        bail!("lvs-anime25d-v1 のsecondary_motion_versionが不正です");
    }
    let hair = profile
        .and_then(|value| value.get("hair"))
        .context("lvs-anime25d-v1 の髪物理定義がありません")?;
    if hair.get("role").and_then(Value::as_str) != Some("hair") {
        bail!("lvs-anime25d-v1 の髪物理対象が不正です");
    }
    for (name, min, max) in [
        ("frequency_hz", 0.2, 8.0),
        ("damping_ratio", 0.2, 2.0),
        ("strength", 0.0, 2.0),
    ] {
        let value = hair
            .get(name)
            .and_then(Value::as_f64)
            .with_context(|| format!("lvs-anime25d-v1 の髪物理{name}が不正です"))?;
        if !value.is_finite() || value < min || value > max {
            bail!("lvs-anime25d-v1 の髪物理{name}が範囲外です");
        }
    }
    Ok(())
}

fn animation_has_frames(animation: &Value) -> bool {
    animation.get("path").and_then(Value::as_str).is_some()
        || animation
            .get("framePaths")
            .and_then(Value::as_array)
            .is_some_and(|paths| !paths.is_empty())
}

fn validate_animations(source: &Path, manifest: &Value) -> Result<()> {
    let Some(animations) = manifest.get("animations") else {
        return Ok(());
    };
    let Some(animations) = animations.as_object() else {
        bail!("animations はオブジェクトで指定してください");
    };
    for (name, animation) in animations {
        let Some(animation) = animation.as_object() else {
            bail!("animations.{name} はオブジェクトで指定してください");
        };
        let declared_frames = animation.get("frames").and_then(Value::as_u64);
        if let Some(paths) = animation.get("framePaths") {
            let Some(paths) = paths.as_array() else {
                bail!("animations.{name}.framePaths は配列で指定してください");
            };
            if paths.is_empty() {
                bail!("animations.{name}.framePaths は1件以上必要です");
            }
            if declared_frames.is_some_and(|count| count as usize != paths.len()) {
                bail!("animations.{name}.frames と framePaths の件数が一致しません");
            }
            for (index, path) in paths.iter().enumerate() {
                let Some(path) = path.as_str() else {
                    bail!("animations.{name}.framePaths[{index}] は文字列で指定してください");
                };
                ensure_asset(source, path).with_context(|| {
                    format!("animations.{name}.framePaths[{index}] が指すファイルがありません")
                })?;
            }
        } else if let Some(path) = animation.get("path").and_then(Value::as_str) {
            let frames = declared_frames.unwrap_or(0);
            if frames == 0 {
                bail!("animations.{name}.frames は1以上にしてください");
            }
            for index in 0..frames {
                ensure_asset(source, &format!("{path}/{index:02}.png")).with_context(|| {
                    format!("animations.{name} のフレーム {index:02}.png がありません")
                })?;
            }
        } else {
            bail!("animations.{name} には path または framePaths が必要です");
        }
    }
    Ok(())
}

/// `model3.json` が参照するファイル（moc3・テクスチャ・モーション・物理演算など）が
/// すべて同梱されているか確かめる。
///
/// Live2D の配布物はフォルダ構成が作者ごとに違い、テクスチャだけ入れ忘れた zip が
/// 珍しくない。導入時に気づかないと、そのキャラを選んだ瞬間に描画が崩れる。
pub fn validate_live2d_model(source: &Path, model_relative: &str) -> Result<()> {
    let model_path = source.join(safe_relative(model_relative, "live2d.model")?);
    let text = std::fs::read_to_string(&model_path)
        .with_context(|| format!("{} を読めません", model_path.display()))?;
    let model: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} が不正なJSONです", model_path.display()))?;
    let base = model_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| source.to_path_buf());

    let mut missing = BTreeSet::new();
    for reference in model_references(&model) {
        let relative = safe_relative(&reference, "model3.json の参照")?;
        if !base.join(&relative).is_file() {
            missing.insert(reference);
        }
    }
    if !missing.is_empty() {
        bail!(
            "Live2D モデルが参照するファイルが不足しています（{} 個）: {}",
            missing.len(),
            missing.into_iter().take(5).collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}

/// `model3.json` の `FileReferences` から相対パスを集める。
///
/// 形はモデルによって揺れる（`"Moc": "x.moc3"` の文字列、`"Textures": [...]` の配列、
/// `"Motions": { "Idle": [{ "File": ... }] }` のグループ）。キー名で判別すると
/// モデルごとの差で取りこぼすため、**「拡張子を持つ文字列だけを参照とみなす」**方針にする。
/// `"Name": "笑顔"` や `"Group": "Idle"` のような値は拡張子が無いので自然に除外される。
fn model_references(model: &Value) -> Vec<String> {
    let Some(references) = model.get("FileReferences") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_files(references, &mut out);
    out
}

/// パスらしい文字列か（末尾が扱える拡張子）。
fn looks_like_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    Path::new(value)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| ALLOWED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn collect_files(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(path) if looks_like_path(path) => out.push(path.trim().to_string()),
        Value::String(_) => {}
        Value::Array(items) => {
            for item in items {
                collect_files(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_files(item, out);
            }
        }
        _ => {}
    }
}

pub fn validate_tree(directory: &Path, canonical_root: &Path, stats: &mut PackStats) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "パック内にシンボリックリンクは置けません: {}",
                path.display()
            );
        }
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(canonical_root) {
            bail!("パック外を指すパスは使えません: {}", path.display());
        }
        if metadata.is_dir() {
            validate_tree(&path, canonical_root, stats)?;
        } else if metadata.is_file() {
            stats.files += 1;
            stats.bytes = stats.bytes.saturating_add(metadata.len());
            if stats.files > MAX_FILES || stats.bytes > MAX_TOTAL_BYTES {
                bail!("パックは{MAX_FILES}ファイル・262MB以内にしてください");
            }
            if metadata.len() > file_limit(&path) {
                bail!(
                    "1ファイルは通常形式42MB、MMD非圧縮画像128MB以内にしてください: {}",
                    path.display()
                );
            }
            if !ALLOWED_EXTENSIONS.contains(&extension_of(&path).as_str()) {
                bail!("利用できないファイル形式です: {}", path.display());
            }
        }
    }
    Ok(())
}

/// 末尾の拡張子（小文字）。`Hiyori.motion3.json` は `json` として扱う
/// （Live2D の付随ファイルは実体が JSON なので、末尾だけで判定すれば足りる）。
fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn required_text<'a>(manifest: &'a Value, key: &str) -> Result<&'a str> {
    manifest
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("character.json に {key} が必要です"))
}

pub fn ensure_asset(source: &Path, relative: &str) -> Result<()> {
    let path = source.join(safe_relative(relative, relative)?);
    if !path.is_file() {
        bail!("参照先のファイルがありません: {relative}");
    }
    Ok(())
}

/// マニフェスト内の相対パスを検証して返す。絶対パス・`..`・ルート脱出を拒否する。
pub fn safe_relative(value: &str, label: &str) -> Result<PathBuf> {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        bail!("{label} には相対パスを指定してください: {value}");
    }
    let mut out = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => bail!("{label} に .. やドライブ指定は使えません: {value}"),
        }
    }
    if out.as_os_str().is_empty() {
        bail!("{label} が空です");
    }
    Ok(out)
}

pub fn validate_variants(source: &Path, manifest: &Value) -> Result<()> {
    let Some(variants) = manifest.get("variants").and_then(Value::as_array) else {
        return Ok(());
    };
    if variants.len() > 8 {
        bail!("variants は8個以内にしてください");
    }
    let mut seen = BTreeSet::new();
    for variant in variants {
        let id = required_text(variant, "id")?;
        if !seen.insert(id.to_string()) {
            bail!("variants の id が重複しています: {id}");
        }
        required_text(variant, "name")?;
        if let Some(root) = variant.get("root").and_then(Value::as_str) {
            let path = source.join(safe_relative(root, "variants[].root")?);
            if !path.is_dir() {
                bail!("variants[].root のフォルダがありません: {root}");
            }
        }
        validate_secondary_motion(variant, "variants[].secondaryMotion")?;
        validate_spring_physics(variant, "variants[].springPhysics")?;
    }
    Ok(())
}

pub fn validate_secondary_motion(manifest: &Value, label: &str) -> Result<()> {
    let Some(motion) = manifest.get("secondaryMotion") else {
        return Ok(());
    };
    let Some(motion) = motion.as_object() else {
        bail!("{label} はオブジェクトで指定してください");
    };
    if motion.get("type").and_then(Value::as_str) != Some("bust") {
        bail!("{label}.type は bust を指定してください");
    }
    for (key, minimum, maximum) in [
        ("centerX", 0.0, 100.0),
        ("centerY", 0.0, 100.0),
        ("radiusX", 4.0, 45.0),
        ("radiusY", 4.0, 35.0),
        ("strength", 0.2, 2.0),
        ("duration", 1.8, 8.0),
    ] {
        let Some(value) = motion.get(key).and_then(Value::as_f64) else {
            bail!("{label}.{key} は数値で指定してください");
        };
        if !(minimum..=maximum).contains(&value) {
            bail!("{label}.{key} は {minimum} 以上 {maximum} 以下にしてください");
        }
    }
    Ok(())
}

pub fn validate_spring_physics(manifest: &Value, label: &str) -> Result<()> {
    let Some(config) = manifest.get("springPhysics") else {
        return Ok(());
    };
    let Some(regions) = config.get("regions").and_then(Value::as_array) else {
        bail!("{label}.regions は配列で指定してください");
    };
    if regions.is_empty() || regions.len() > 12 {
        bail!("{label}.regions は1件以上12件以下にしてください");
    }
    for (index, region) in regions.iter().enumerate() {
        let Some(region) = region.as_object() else {
            bail!("{label}.regions[{index}] はオブジェクトで指定してください");
        };
        for (key, minimum, maximum) in [
            ("centerX", 0.0, 100.0),
            ("centerY", 0.0, 100.0),
            ("radiusX", 3.0, 48.0),
            ("radiusY", 3.0, 42.0),
            ("stiffness", 4.0, 80.0),
            ("damping", 1.0, 24.0),
            ("drive", 0.05, 2.4),
            ("frequency", 0.08, 1.2),
        ] {
            let Some(value) = region.get(key).and_then(Value::as_f64) else {
                bail!("{label}.regions[{index}].{key} は数値で指定してください");
            };
            if !(minimum..=maximum).contains(&value) {
                bail!(
                    "{label}.regions[{index}].{key} は {minimum} 以上 {maximum} 以下にしてください"
                );
            }
        }
        for (key, minimum, maximum) in [
            ("axisX", -8.0, 8.0),
            ("axisY", -8.0, 8.0),
            ("rotation", -6.0, 6.0),
            ("scale", 0.0, 0.03),
        ] {
            let Some(value) = region.get(key).and_then(Value::as_f64) else {
                continue;
            };
            if !(minimum..=maximum).contains(&value) {
                bail!(
                    "{label}.regions[{index}].{key} は {minimum} 以上 {maximum} 以下にしてください"
                );
            }
        }
    }
    Ok(())
}

pub fn validate_interactions(manifest: &Value) -> Result<()> {
    let Some(reactions) = manifest.pointer("/interactions/reactions") else {
        return Ok(());
    };
    let Some(map) = reactions.as_object() else {
        bail!("interactions.reactions はオブジェクトで指定してください");
    };
    for (part, lines) in map {
        let Some(lines) = lines.as_array() else {
            bail!("interactions.reactions.{part} は配列で指定してください");
        };
        if lines.len() > 16 {
            bail!("interactions.reactions.{part} は16個以内にしてください");
        }
        for line in lines {
            let Some(line) = line.as_str() else {
                bail!("interactions.reactions.{part} には文字列だけを入れてください");
            };
            if line.chars().count() > 120 {
                bail!("interactions.reactions.{part} の台詞は120文字以内にしてください");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("picoagent-pack-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 最小構成の Live2D パックを作る。
    fn live2d_pack(dir: &Path, with_texture: bool) {
        std::fs::create_dir_all(dir.join("live2d")).unwrap();
        std::fs::write(dir.join("preview.png"), [0u8; 8]).unwrap();
        std::fs::write(dir.join("live2d/Model.moc3"), [0u8; 8]).unwrap();
        if with_texture {
            std::fs::create_dir_all(dir.join("live2d/Model.2048")).unwrap();
            std::fs::write(dir.join("live2d/Model.2048/texture_00.png"), [0u8; 8]).unwrap();
        }
        std::fs::write(
            dir.join("live2d/Model.model3.json"),
            serde_json::to_string(&json!({
                "FileReferences": {
                    "Moc": "Model.moc3",
                    "Textures": ["Model.2048/texture_00.png"],
                    "Motions": { "Idle": [{ "File": "motions/idle.motion3.json" }] }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("live2d/motions")).unwrap();
        std::fs::write(dir.join("live2d/motions/idle.motion3.json"), b"{}").unwrap();
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "mymodel",
                "name": "マイモデル",
                "boot": { "sleeping": "preview.png" },
                "live2d": { "model": "live2d/Model.model3.json" }
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn 参照が揃ったlive2dパックは通る() {
        let dir = temp_dir("ok");
        live2d_pack(&dir, true);
        let manifest = validate(&dir).expect("検証に失敗");
        assert_eq!(manifest["id"], "mymodel");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 共有フレーム参照だけの2dパックは通る() {
        let dir = temp_dir("frame-paths-ok");
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        std::fs::write(dir.join("frames/a.png"), [0u8; 8]).unwrap();
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "compact-2d",
                "name": "共有フレーム",
                "animations": {
                    "idle": {
                        "frames": 2,
                        "framePaths": ["frames/a.png", "frames/a.png"],
                        "fps": 3.0
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        validate(&dir).expect("共有フレーム参照を検証できない");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lossless_webpの共有フレームも通る() {
        let dir = temp_dir("webp-frame-paths-ok");
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        std::fs::write(dir.join("frames/a.webp"), b"RIFF\0\0\0\0WEBP").unwrap();
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "compact-webp-2d",
                "name": "WebP共有フレーム",
                "animations": {
                    "idle": {
                        "frames": 2,
                        "framePaths": ["frames/a.webp", "frames/a.webp"],
                        "fps": 3.0
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        validate(&dir).expect("WebP共有フレーム参照を検証できない");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn メッシュリグの実行資産を検証する() {
        let dir = temp_dir("mesh-rig");
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        std::fs::create_dir_all(dir.join("mesh-rig/parts")).unwrap();
        std::fs::write(dir.join("frames/a.webp"), b"RIFF\0\0\0\0WEBP").unwrap();
        std::fs::write(dir.join("mesh-rig/parts/base.webp"), b"RIFF\0\0\0\0WEBP").unwrap();
        std::fs::write(dir.join("mesh-rig/parts/blink.webp"), b"RIFF\0\0\0\0WEBP").unwrap();
        std::fs::write(
            dir.join("mesh-rig/rig.json"),
            serde_json::to_string(&json!({
                "version": 1,
                "mode": "single-sheet-grid-v1",
                "base": "parts/base.webp",
                "drawables": { "blink_closed": { "texture": "parts/blink.webp" } }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "mesh-rig", "name": "メッシュリグ",
                "animations": { "idle": { "frames": 1, "framePaths": ["frames/a.webp"] } },
                "meshRig": { "rig": "mesh-rig/rig.json" }
            }))
            .unwrap(),
        )
        .unwrap();
        validate(&dir).expect("正しいメッシュリグを拒否した");

        std::fs::write(
            dir.join("mesh-rig/rig.json"),
            serde_json::to_string(&json!({
                "version": 1, "mode": "single-sheet-grid-v1",
                "base": "source/regions.json"
            }))
            .unwrap(),
        )
        .unwrap();
        let error = validate(&dir).unwrap_err().to_string();
        assert!(error.contains("パスが不正"), "error: {error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn anime25d単独パックとlayered_grid_v2を検証する() {
        let dir = temp_dir("anime25d-rig");
        std::fs::create_dir_all(dir.join("mesh-rig/parts")).unwrap();
        std::fs::write(dir.join("mesh-rig/parts/face.png"), [0u8; 8]).unwrap();
        std::fs::write(
            dir.join("mesh-rig/rig.json"),
            serde_json::to_string(&json!({
                "version": 2,
                "mode": "anime25d-v1",
                "canvas": [1280, 1280],
                "anchors": { "face": { "cx": 640, "cy": 180 } },
                "layers": [{
                    "name": "face", "texture": "parts/face.png",
                    "textureBox": [570, 88, 136, 161]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "anime25d", "name": "独自2.5D",
                "meshRig": { "rig": "mesh-rig/rig.json" }
            }))
            .unwrap(),
        )
        .unwrap();

        validate(&dir).expect("anime25d単独パックを拒否した");

        std::fs::write(
            dir.join("mesh-rig/rig.json"),
            serde_json::to_string(&json!({
                "version": 1,
                "mode": "layered-grid-v2",
                "base": "parts/face.png",
                "drawables": {}
            }))
            .unwrap(),
        )
        .unwrap();
        validate(&dir).expect("UIが対応済みのlayered-grid-v2を拒否した");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn local_vtuber_studio_schema3を変換せず検証する() {
        let dir = temp_dir("lvs-anime25d-rig");
        std::fs::create_dir_all(dir.join("rig2d/parts")).unwrap();
        let graph = [
            ("scene_residual", "residual"),
            ("scene_torso", "torso"),
            ("scene_left_arm", "left_arm"),
            ("scene_right_arm", "right_arm"),
            ("scene_neck", "neck"),
            ("scene_face", "face"),
            ("scene_hair", "hair"),
        ];
        let mut names = graph
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect::<Vec<_>>();
        names.extend(
            ["mouth_open", "mouth_closed"]
                .into_iter()
                .map(str::to_owned),
        );
        for side in ["left", "right"] {
            names.extend(
                [
                    "eye_base",
                    "eye_closed",
                    "eyelid_upper",
                    "eye_backplate",
                    "eye_iris",
                ]
                .into_iter()
                .map(|part| format!("{side}_{part}")),
            );
        }
        let mut layers = serde_json::Map::new();
        for name in names {
            std::fs::write(
                dir.join("rig2d/parts").join(format!("{name}.png")),
                [0u8; 8],
            )
            .unwrap();
            layers.insert(
                name.clone(),
                json!({ "url": format!("/assets/rig2d/parts/{name}.png") }),
            );
        }
        std::fs::write(
            dir.join("rig2d/rig.json"),
            serde_json::to_string(&json!({
                "schema_version": 3,
                "profile": "lvs-anime25d-v1",
                "eye_rig_version": 3,
                "lip_rig_version": 1,
                "scene_graph_version": 1,
                "secondary_motion_version": 1,
                "secondary_motion": { "hair": {
                    "role": "hair", "frequency_hz": 1.8,
                    "damping_ratio": 0.72, "strength": 1.0
                }},
                "scene_graph": graph.iter().map(|(layer, role)| json!({ "layer": layer, "role": role })).collect::<Vec<_>>(),
                "layers": layers
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "lvs-anime25d", "name": "LocalVTuberStudio直接取込",
                "meshRig": { "rig": "rig2d/rig.json" }
            }))
            .unwrap(),
        )
        .unwrap();

        validate(&dir).expect("LocalVTuberStudio schema 3を拒否した");
        let rig_path = dir.join("rig2d/rig.json");
        let mut invalid: Value =
            serde_json::from_str(&std::fs::read_to_string(&rig_path).unwrap()).unwrap();
        invalid["secondary_motion"]["hair"]["strength"] = json!(3.0);
        std::fs::write(&rig_path, serde_json::to_string(&invalid).unwrap()).unwrap();
        assert!(validate(&dir)
            .unwrap_err()
            .to_string()
            .contains("髪物理strengthが範囲外"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 共有フレーム参照の件数ずれと欠損を弾く() {
        let dir = temp_dir("frame-paths-ng");
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "compact-2d",
                "name": "共有フレーム",
                "animations": {
                    "idle": {
                        "frames": 2,
                        "framePaths": ["frames/missing.png"]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let error = validate(&dir).unwrap_err().to_string();
        assert!(error.contains("件数が一致"), "error: {error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 通常2dの胸元二次動作を検証する() {
        let valid = json!({
            "secondaryMotion": {
                "type": "bust", "centerX": 50, "centerY": 32,
                "radiusX": 16, "radiusY": 10, "strength": 1.1, "duration": 3.8
            }
        });
        validate_secondary_motion(&valid, "secondaryMotion").expect("正しい二次動作を拒否した");

        let invalid = json!({
            "secondaryMotion": {
                "type": "bust", "centerX": 50, "centerY": 32,
                "radiusX": 80, "radiusY": 10, "strength": 1.1, "duration": 3.8
            }
        });
        let error = validate_secondary_motion(&invalid, "secondaryMotion")
            .expect_err("範囲外のマスクを許可した")
            .to_string();
        assert!(error.contains("radiusX"), "error: {error}");
    }

    #[test]
    fn 通常2dのバネ物理を検証する() {
        let valid = json!({
            "springPhysics": { "regions": [{
                "centerX": 50, "centerY": 48, "radiusX": 20, "radiusY": 12,
                "stiffness": 22, "damping": 8, "drive": 0.6, "frequency": 0.24
            }]}
        });
        validate_spring_physics(&valid, "springPhysics").expect("正しいバネ物理を拒否した");

        let invalid = json!({
            "springPhysics": { "regions": [{
                "centerX": 50, "centerY": 48, "radiusX": 20, "radiusY": 12,
                "stiffness": 22, "damping": 8, "drive": 9.0, "frequency": 0.24
            }]}
        });
        let error = validate_spring_physics(&invalid, "springPhysics")
            .expect_err("範囲外のバネ力を許可した")
            .to_string();
        assert!(error.contains("drive"), "error: {error}");
    }

    /// テクスチャ欠けは導入前に止める（通すとキャラ選択時に描画が崩れる）。
    #[test]
    fn テクスチャが欠けたパックは弾く() {
        let dir = temp_dir("ng-tex");
        live2d_pack(&dir, false);
        let error = validate(&dir).unwrap_err().to_string();
        assert!(error.contains("参照するファイルが不足"), "error: {error}");
        assert!(error.contains("texture_00.png"), "error: {error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 未知の拡張子と巨大ファイルを弾く() {
        let dir = temp_dir("ng-ext");
        live2d_pack(&dir, true);
        std::fs::write(dir.join("setup.exe"), [0u8; 4]).unwrap();
        let error = validate(&dir).unwrap_err().to_string();
        assert!(error.contains("利用できないファイル形式"), "error: {error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 危険な相対パスを拒否する() {
        assert!(safe_relative("../secret", "テスト").is_err());
        assert!(safe_relative("/etc/passwd", "テスト").is_err());
        assert!(safe_relative("", "テスト").is_err());
        assert_eq!(
            safe_relative("live2d/a.moc3", "テスト").unwrap(),
            Path::new("live2d/a.moc3")
        );
    }

    #[test]
    fn idは英数字とハイフンだけにする() {
        let dir = temp_dir("ng-id");
        live2d_pack(&dir, true);
        std::fs::write(
            dir.join("character.json"),
            serde_json::to_string(&json!({
                "id": "my model!",
                "name": "だめ",
                "live2d": { "model": "live2d/Model.model3.json" }
            }))
            .unwrap(),
        )
        .unwrap();
        let error = validate(&dir).unwrap_err().to_string();
        assert!(error.contains("id は"), "error: {error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// model3.json の参照は「文字列 / 配列 / File キー」のどの形でも拾う。
    #[test]
    fn 参照の形が違っても拾う() {
        let model = json!({
            "FileReferences": {
                "Moc": "a.moc3",
                "Textures": ["t0.png", "t1.png"],
                "Physics": "p.physics3.json",
                "Motions": { "Idle": [{ "File": "m0.motion3.json", "FadeInTime": 0.5 }] },
                "Expressions": [{ "Name": "笑顔", "File": "e0.exp3.json" }]
            }
        });
        let mut found = model_references(&model);
        found.sort();
        assert_eq!(
            found,
            vec![
                "a.moc3",
                "e0.exp3.json",
                "m0.motion3.json",
                "p.physics3.json",
                "t0.png",
                "t1.png"
            ]
        );
        // 表示名（Name）はパスではないので拾わない。
        assert!(!found.contains(&"笑顔".to_string()));
    }
}
