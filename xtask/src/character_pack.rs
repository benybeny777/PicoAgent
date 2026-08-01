// character_pack.rs - 2Dキャラクターパックの検証と正本への導入。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::util::{copy_dir, log, repo_root};

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
    validate_tree(source, &canonical_root)?;
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
    Ok(manifest)
}

fn validate_tree(directory: &Path, canonical_root: &Path) -> Result<()> {
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
            validate_tree(&path, canonical_root)?;
        }
    }
    Ok(())
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
    if !source.join(path).exists() {
        bail!("アセットが見つかりません: {relative}");
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
    copy_dir(source, &destination, &|path| {
        path.file_name()
            .map(|name| name == ".git" || name == "target")
            .unwrap_or(false)
    })?;

    let index_path = root.join("characters/index.json");
    let mut index: Value = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    let characters = index
        .get_mut("characters")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("characters/index.json の characters が不正です"))?;
    characters.push(json!({
        "id": id,
        "name": name,
        "manifest": format!("{id}/character.json"),
        "description": manifest.get("description").and_then(Value::as_str).unwrap_or("追加キャラクター"),
    }));
    fs::write(
        &index_path,
        format!("{}\n", serde_json::to_string_pretty(&index)?),
    )?;
    crate::assets::sync_characters()?;
    log("character-pack", format!("{name} ({id}) を導入しました"));
    Ok(())
}
