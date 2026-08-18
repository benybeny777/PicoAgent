// character_pack.rs - 開発時にキャラクターパックを検証してリポジトリへ導入する。
//
// **検証規則はアプリ側の `src-tauri/src/character_pack.rs` が正本**。CLI とアプリで
// 規則が分かれると「CLI では通るのにアプリが弾く」状態になるため、ここでは
// `#[path]` でその正本を取り込んで使う（アプリの他モジュールには依存していない）。
// このファイルに残すのは、リポジトリの `characters/` へ入れる導入手順だけ。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::util::{copy_dir, log, repo_root, write_atomic};

// 検証規則の正本をそのまま取り込む。CLI が直接呼ばない補助関数・定数も含むため
// dead_code は許可する（アプリ側では使っている）。
#[allow(dead_code)]
#[path = "../../src-tauri/src/character_pack.rs"]
mod shared;

use shared::{required_text, validate};

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
