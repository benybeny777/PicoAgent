// release.rs - 販売用ビルドを作る前の出荷条件と同梱キャラを検査する。

use std::fs;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::util::{log, repo_root};

const REQUIRED_DOCUMENTS: [&str; 5] = [
    "docs/EULA-DRAFT.md",
    "docs/PRIVACY.md",
    "docs/SUPPORT.md",
    "docs/THIRD-PARTY-NOTICES.md",
    "docs/SALE-READINESS.md",
];

pub fn manifest() -> Result<Value> {
    let path = repo_root().join("distribution/release.json");
    let text = fs::read_to_string(&path)
        .with_context(|| format!("販売設定を読めません: {}", path.display()))?;
    serde_json::from_str(&text).context("distribution/release.json が正しいJSONではありません")
}

pub fn included_characters(value: &Value) -> Result<Vec<String>> {
    let ids = value
        .get("includedCharacters")
        .and_then(Value::as_array)
        .context("includedCharacters は配列で指定してください")?;
    let mut result = Vec::new();
    for value in ids {
        let id = value
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .context("includedCharacters に空または文字列以外の値があります")?;
        if result.iter().any(|current| current == id) {
            bail!("販売用キャラが重複しています: {id}");
        }
        result.push(id.to_string());
    }
    if result.is_empty() {
        bail!("販売用キャラが1体も指定されていません");
    }
    Ok(result)
}

pub fn check(strict: bool) -> Result<Value> {
    let root = repo_root();
    let value = manifest()?;
    let characters = included_characters(&value)?;
    let mut blockers = Vec::new();

    for document in REQUIRED_DOCUMENTS {
        if !root.join(document).is_file() {
            blockers.push(format!("必須文書がありません: {document}"));
        }
    }
    for id in &characters {
        if !root
            .join("characters")
            .join(id)
            .join("character.json")
            .is_file()
        {
            blockers.push(format!("販売用キャラが見つかりません: {id}"));
        }
    }

    let legal = value.get("legal").and_then(Value::as_object);
    if legal
        .and_then(|item| item.get("eulaReviewed"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        blockers.push("EULAの専門家確認が未完了です".to_string());
    }
    if legal
        .and_then(|item| item.get("privacyReviewed"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        blockers.push("プライバシー表示の専門家確認が未完了です".to_string());
    }
    if legal
        .and_then(|item| item.get("thirdPartyInventoryReviewed"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        blockers.push(
            "同梱バイナリ・モデル・素材の再配布条件とライセンス台帳の確認が未完了です".to_string(),
        );
    }
    if legal
        .and_then(|item| item.get("live2dApprovalReference"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        blockers.push("Live2D拡張性アプリケーションの審査・契約参照が未登録です".to_string());
    }

    for (group, key, label) in [
        ("support", "supportUrl", "サポート窓口URL"),
        ("support", "privacyUrl", "プライバシー表示URL"),
        ("support", "termsUrl", "利用条件URL"),
        ("updates", "endpoint", "更新配信エンドポイント"),
        ("updates", "publicKey", "更新署名の公開鍵"),
    ] {
        if value
            .get(group)
            .and_then(|item| item.get(key))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            blockers.push(format!("{label}が未設定です"));
        }
    }
    let platforms = value
        .get("platforms")
        .and_then(Value::as_array)
        .context("platforms は配列で指定してください")?;
    if platforms.is_empty() {
        bail!("販売対象OSが1件も指定されていません");
    }
    for platform in platforms {
        let platform = platform
            .as_str()
            .context("platforms に文字列以外の値があります")?;
        let (key, label) = match platform {
            "windows" => ("windowsReady", "Windowsコード署名"),
            "macos" => ("macosReady", "macOS署名・公証"),
            other => {
                blockers.push(format!("未対応の販売対象OSです: {other}"));
                continue;
            }
        };
        if value
            .get("signing")
            .and_then(|item| item.get(key))
            .and_then(Value::as_bool)
            != Some(true)
        {
            blockers.push(format!("{label}の準備が未完了です"));
        }
    }

    let cargo_version = package_version(&root.join("src-tauri/Cargo.toml"))?;
    let xtask_version = package_version(&root.join("xtask/Cargo.toml"))?;
    let tauri_text = fs::read_to_string(root.join("src-tauri/tauri.conf.json"))
        .context("tauri.conf.jsonを読めません")?;
    let tauri_config =
        serde_json::from_str::<Value>(&tauri_text).context("tauri.conf.jsonが不正です")?;
    let tauri_version = tauri_config
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if cargo_version != tauri_version || cargo_version != xtask_version {
        blockers.push(format!(
            "バージョンが一致しません: app={cargo_version}, tauri={tauri_version}, xtask={xtask_version}"
        ));
    }
    let cargo_text = fs::read_to_string(root.join("src-tauri/Cargo.toml"))?;
    let lib_text = fs::read_to_string(root.join("src-tauri/src/lib.rs"))?;
    if !cargo_text.contains("tauri-plugin-updater")
        || !lib_text.contains("tauri_plugin_updater")
        || tauri_config.pointer("/bundle/createUpdaterArtifacts") != Some(&Value::Bool(true))
    {
        blockers.push("Tauri updater本体と更新成果物の生成設定が未実装です".to_string());
    }

    log(
        "release-check",
        format!(
            "販売用キャラ: {} / バージョン: {} / ブロッカー: {}件",
            characters.join(", "),
            cargo_version,
            blockers.len()
        ),
    );
    for blocker in &blockers {
        eprintln!("  未完了: {blocker}");
    }
    if strict && !blockers.is_empty() {
        bail!("販売用ビルドを停止しました。上の未完了項目を解消してください");
    }
    Ok(serde_json::json!({
        "ready": blockers.is_empty(),
        "blockers": blockers,
        "characters": characters,
        "version": cargo_version,
    }))
}

fn package_version(path: &std::path::Path) -> Result<String> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("Cargo.tomlを読めません: {}", path.display()))?;
    let package = text
        .split("[package]")
        .nth(1)
        .and_then(|tail| tail.split("\n[").next())
        .context("[package]を見つけられません")?;
    package
        .lines()
        .find_map(|line| line.trim().strip_prefix("version = "))
        .map(|value| value.trim().trim_matches('"').to_string())
        .context("package.versionを見つけられません")
}

#[cfg(test)]
mod tests {
    use super::{check, included_characters, package_version};

    #[test]
    fn 販売設定はキャラを重複なく列挙する() {
        let value = super::manifest().unwrap();
        let characters = included_characters(&value).unwrap();
        assert_eq!(characters, ["mugi", "rin", "mai"]);
    }

    #[test]
    fn 販売監査は機械可読なレポートを返す() {
        let report = check(false).unwrap();
        assert!(report["ready"].is_boolean());
        assert!(report["blockers"].is_array());
        assert_eq!(report["version"], "1.0.0");
    }

    #[test]
    fn 三箇所のバージョンは一致する() {
        let root = crate::util::repo_root();
        let app = package_version(&root.join("src-tauri/Cargo.toml")).unwrap();
        let xtask = package_version(&root.join("xtask/Cargo.toml")).unwrap();
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("src-tauri/tauri.conf.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(app, xtask);
        assert_eq!(app, config["version"]);
    }

    #[test]
    fn アンインストールはユーザーデータを無確認で削除しない() {
        let root = crate::util::repo_root();
        let script = std::fs::read_to_string(root.join("src-tauri/installer-hooks.nsh")).unwrap();
        let prompt = script.find("MessageBox MB_YESNO").unwrap();
        let delete = script.find("RMDir /r \"$APPDATA\\PicoAgent\"").unwrap();
        assert!(prompt < delete);
        assert!(script.contains("IfSilent picoagent_keep_user_data"));
    }
}
