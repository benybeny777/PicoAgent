// character_pack.rs - 2Dキャラクターパックの検証と正本への導入。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::util::{copy_dir, log, repo_root};

const MAX_FILES: usize = 1_500;
const MAX_TOTAL_BYTES: u64 = 250 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 40 * 1024 * 1024;
const ALLOWED_EXTENSIONS: &[&str] = &[
    "json", "png", "webp", "jpg", "jpeg", "moc3", "wav", "psd", "md",
];

pub fn run(args: &[String]) -> Result<()> {
    let action = args.first().map(String::as_str).unwrap_or("");
    let source = args.get(1).map(PathBuf::from);
    match (action, source) {
        ("validate", Some(source)) => {
            let manifest = validate(&source)?;
            log(
                "character-pack",
                format!("{} ({}) は有効です", manifest["name"], manifest["id"]),
            );
            Ok(())
        }
        ("install", Some(source)) => install(&source),
        _ => bail!("使い方: cargo xtask character-pack validate|install <パックフォルダ>"),
    }
}

fn validate(source: &Path) -> Result<Value> {
    if !source.is_dir() {
        bail!("パックフォルダが見つかりません: {}", source.display());
    }
    let canonical_root = source
        .canonicalize()
        .context("パックフォルダの実体を確認できません")?;
    let mut stats = PackStats::default();
    validate_tree(source, &canonical_root, &mut stats)?;
    let manifest_path = source.join("character.json");
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("{} を読めません", manifest_path.display()))?;
    let manifest: Value = serde_json::from_str(&text).context("character.json が不正なJSONです")?;
    let id = required_text(&manifest, "id")?;
    if id.len() > 64
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("id は64文字以内の英数字・ハイフン・アンダースコアだけにしてください");
    }
    required_text(&manifest, "name")?;
    if manifest.pointer("/live2d/model").is_none()
        && manifest.pointer("/animations/idle/path").is_none()
    {
        bail!("live2d.model または animations.idle.path が必要です");
    }
    for pointer in ["/boot/sleeping", "/live2d/model"] {
        if let Some(relative) = manifest.pointer(pointer).and_then(Value::as_str) {
            ensure_asset(source, relative)?;
        }
    }
    if let Some(animations) = manifest.get("animations").and_then(Value::as_object) {
        for (name, animation) in animations {
            let relative = animation
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("animations.{name}.path がありません"))?;
            ensure_asset(source, relative)?;
            let frames = animation.get("frames").and_then(Value::as_u64).unwrap_or(0);
            if frames == 0 {
                bail!("animations.{name}.frames は1以上にしてください");
            }
        }
    }
    validate_variants(source, &manifest)?;
    validate_interactions(&manifest)?;
    Ok(manifest)
}

#[derive(Default)]
struct PackStats {
    files: usize,
    bytes: u64,
}

fn validate_tree(directory: &Path, canonical_root: &Path, stats: &mut PackStats) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
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
                bail!("パックは{MAX_FILES}ファイル・250MB以内にしてください");
            }
            if metadata.len() > MAX_FILE_BYTES {
                bail!("1ファイルは40MB以内にしてください: {}", path.display());
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
                bail!("利用できないファイル形式です: {}", path.display());
            }
            if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
                validate_image_dimensions(&path, &extension)?;
            }
        }
    }
    Ok(())
}

fn validate_image_dimensions(path: &Path, extension: &str) -> Result<()> {
    let bytes = fs::read(path)?;
    let dimensions = match extension {
        "png" if bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" => Some((
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )),
        "jpg" | "jpeg" => jpeg_dimensions(&bytes),
        "webp" => webp_dimensions(&bytes),
        _ => None,
    }
    .ok_or_else(|| anyhow::anyhow!("画像ヘッダーを確認できません: {}", path.display()))?;
    if dimensions.0 == 0 || dimensions.1 == 0 || dimensions.0 > 8192 || dimensions.1 > 8192 {
        bail!(
            "画像は縦横8192px以内にしてください（{}x{}）: {}",
            dimensions.0,
            dimensions.1,
            path.display()
        );
    }
    Ok(())
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..2)? != [0xff, 0xd8] {
        return None;
    }
    let mut offset = 2;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let length = u16::from_be_bytes([*bytes.get(offset)?, *bytes.get(offset + 1)?]) as usize;
        if length < 2 || offset + length > bytes.len() {
            return None;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let height = u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]) as u32;
            return Some((width, height));
        }
        offset += length;
    }
    None
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    match bytes.get(12..16)? {
        b"VP8X" => Some((
            1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]),
            1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]),
        )),
        b"VP8L" if bytes.get(20) == Some(&0x2f) => Some((
            1 + (((bytes[22] & 0x3f) as u32) << 8 | bytes[21] as u32),
            1 + (((bytes[24] & 0x0f) as u32) << 10
                | (bytes[23] as u32) << 2
                | ((bytes[22] & 0xc0) as u32) >> 6),
        )),
        b"VP8 " if bytes.get(23..26) == Some(&[0x9d, 0x01, 0x2a]) => Some((
            (u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3fff) as u32,
            (u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3fff) as u32,
        )),
        _ => None,
    }
}

fn required_text<'a>(manifest: &'a Value, key: &str) -> Result<&'a str> {
    manifest
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("character.json に {key} がありません"))
}

fn ensure_asset(source: &Path, relative: &str) -> Result<()> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("アセットパスはパック内の相対パスにしてください: {relative}");
    }
    if !source.join(path).is_file() && !source.join(path).is_dir() {
        bail!("アセットが見つかりません: {relative}");
    }
    Ok(())
}

fn safe_relative(value: &str, label: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("{label} はパック内の相対パスにしてください: {value}");
    }
    Ok(path.to_path_buf())
}

fn validate_variants(source: &Path, manifest: &Value) -> Result<()> {
    let Some(variants) = manifest.get("variants") else {
        return Ok(());
    };
    let variants = variants
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("variants は配列にしてください"))?;
    let mut ids = std::collections::HashSet::new();
    for variant in variants {
        let id = required_text(variant, "id")?;
        if id.len() > 48
            || !id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            || !ids.insert(id)
        {
            bail!("variant.id は重複しない48文字以内の英数字・記号にしてください: {id}");
        }
        required_text(variant, "name")?;
        let variant_root = variant
            .get("root")
            .and_then(Value::as_str)
            .map(|value| safe_relative(value, "variant.root"))
            .transpose()?
            .unwrap_or_default();
        if !variant_root.as_os_str().is_empty() && !source.join(&variant_root).is_dir() {
            bail!("variant.root が見つかりません: {}", variant_root.display());
        }
        if let Some(animations) = variant.get("animations").and_then(Value::as_object) {
            for (name, animation) in animations {
                let relative = animation
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!("variant animations.{name}.path がありません")
                    })?;
                let relative = safe_relative(relative, "variant animation path")?;
                if !source.join(&variant_root).join(&relative).exists() {
                    bail!("variant のアセットが見つかりません: {}", relative.display());
                }
                if animation.get("frames").and_then(Value::as_u64).unwrap_or(0) == 0 {
                    bail!("variant animations.{name}.frames は1以上にしてください");
                }
            }
        }
    }
    Ok(())
}

fn validate_interactions(manifest: &Value) -> Result<()> {
    let Some(reactions) = manifest.pointer("/interactions/reactions") else {
        return Ok(());
    };
    let reactions = reactions
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("interactions.reactions はオブジェクトにしてください"))?;
    for (zone, lines) in reactions {
        if !matches!(zone.as_str(), "head" | "cheek" | "hand") {
            bail!("未対応のふれあい領域です: {zone}");
        }
        let lines = lines
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("interactions.reactions.{zone} は配列にしてください"))?;
        if lines.len() > 12
            || lines.iter().any(|line| {
                line.as_str()
                    .map_or(true, |text| text.is_empty() || text.chars().count() > 80)
            })
        {
            bail!("interactions.reactions.{zone} は80文字以内の台詞を12件以下にしてください");
        }
    }
    Ok(())
}

fn install(source: &Path) -> Result<()> {
    let manifest = validate(source)?;
    let id = required_text(&manifest, "id")?;
    let name = required_text(&manifest, "name")?;
    let root = repo_root();
    let destination = root.join("characters").join(id);
    if destination.exists() {
        bail!("同じidのキャラクターが既にあります: {id}");
    }
    let index_path = root.join("characters/index.json");
    let original_index = fs::read(&index_path)?;
    let mut index: Value = serde_json::from_slice(&original_index)?;
    let characters = index
        .get_mut("characters")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("characters/index.json の characters が不正です"))?;
    if characters
        .iter()
        .any(|character| character.get("id").and_then(Value::as_str) == Some(id))
    {
        bail!("characters/index.json に同じidが既にあります: {id}");
    }
    characters.push(json!({
        "id": id,
        "name": name,
        "manifest": format!("{id}/character.json"),
        "description": manifest.get("description").and_then(Value::as_str).unwrap_or("追加キャラクター"),
    }));
    let next_index = format!("{}\n", serde_json::to_string_pretty(&index)?);
    let staging = root
        .join("characters")
        .join(format!(".installing-{id}-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }

    let result = (|| -> Result<()> {
        copy_dir(source, &staging, &|path| {
            path.file_name()
                .map(|name| name == ".git" || name == "target")
                .unwrap_or(false)
        })?;
        validate(&staging).context("コピー後のキャラクターパック検証に失敗しました")?;
        fs::rename(&staging, &destination)
            .with_context(|| format!("導入先を確定できません: {}", destination.display()))?;
        if let Err(error) = write_atomic(&index_path, next_index.as_bytes()) {
            let rollback_index = write_atomic(&index_path, &original_index);
            let rollback_character = fs::remove_dir_all(&destination);
            if rollback_index.is_err() || rollback_character.is_err() {
                bail!(
                    "キャラクター一覧の更新に失敗し、ロールバックも完了できませんでした: {error}"
                );
            }
            return Err(error).context("キャラクター一覧を更新できません");
        }
        if let Err(error) = crate::assets::sync_characters() {
            let rollback_index = write_atomic(&index_path, &original_index);
            let rollback_character = fs::remove_dir_all(&destination);
            let rollback_assets = crate::assets::sync_characters();
            if rollback_index.is_err() || rollback_character.is_err() || rollback_assets.is_err() {
                bail!("配信用同期に失敗し、ロールバックも完了できませんでした: {error}");
            }
            return Err(error).context("配信用キャラクター同期に失敗したため導入を戻しました");
        }
        Ok(())
    })();
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    log("character-pack", format!("{name} ({id}) を導入しました"));
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("保存先の親フォルダがありません"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".character-pack-{}.tmp", std::process::id()));
    let backup = parent.join(format!(".character-pack-{}.backup", std::process::id()));
    let result = (|| -> Result<()> {
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            fs::rename(path, &backup)?;
        }
        if let Err(error) = fs::rename(&temp, path) {
            if backup.exists() {
                let _ = fs::rename(&backup, path);
            }
            return Err(error.into());
        }
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
        if backup.exists() && !path.exists() {
            let _ = fs::rename(&backup, path);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "picoagent-character-pack-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn パック外参照と未許可形式を拒否する() {
        let root = temp_dir("invalid");
        fs::write(
            root.join("character.json"),
            r#"{"id":"demo","name":"デモ","animations":{"idle":{"path":"idle","frames":1}}}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("idle")).unwrap();
        fs::write(root.join("idle/00.exe"), b"x").unwrap();
        assert!(validate(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 原子的な書き込みで既存内容を置き換える() {
        let root = temp_dir("atomic");
        let path = root.join("index.json");
        fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }
}
