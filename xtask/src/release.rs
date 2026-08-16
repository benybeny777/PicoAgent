// release.rs - 販売用ビルドを作る前の出荷条件と同梱キャラを検査する。

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::util::{log, repo_root};

const REQUIRED_DOCUMENTS: [&str; 5] = [
    "docs/EULA-DRAFT.md",
    "docs/PRIVACY.md",
    "docs/SUPPORT.md",
    "docs/THIRD-PARTY-NOTICES.md",
    "docs/SALE-READINESS.md",
];
const MAX_UPDATE_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_UPDATE_NOTES_BYTES: usize = 8 * 1024;

pub fn manifest() -> Result<Value> {
    let path = repo_root().join("distribution/release.json");
    let text = fs::read_to_string(&path)
        .with_context(|| format!("販売設定を読めません: {}", path.display()))?;
    serde_json::from_str(&text).context("distribution/release.json が正しいJSONではありません")
}

/// 公開配布Gitへ置く最新版情報へ、実際のWindows NSISのサイズとSHA-256を書き込む。
pub fn write_update_feed(args: &[String]) -> Result<()> {
    let installer = args
        .first()
        .map(std::path::PathBuf::from)
        .context("使い方: cargo xtask update-feed <NSISインストーラ> [更新内容]")?;
    anyhow::ensure!(
        installer.is_file(),
        "インストーラが見つかりません: {}",
        installer.display()
    );
    anyhow::ensure!(
        installer.extension().and_then(|ext| ext.to_str()) == Some("exe"),
        "Windows NSISの.exeを指定してください"
    );
    let name = installer
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        })
        .context("インストーラ名が不正です")?;
    let version = package_version(&repo_root().join("src-tauri/Cargo.toml"))?;
    let size = fs::metadata(&installer)?.len();
    anyhow::ensure!(
        size > 0 && size <= MAX_UPDATE_INSTALLER_BYTES,
        "インストーラのサイズが更新上限を超えています"
    );
    verify_windows_pe(&installer)?;
    let mut file = fs::File::open(&installer)?;
    let mut digest = Sha256::new();
    // Windowsの既定スレッドスタックと同じ1MiBをローカル配列にすると、
    // 実インストーラのハッシュ計算へ入る前にスタックオーバーフローする。
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let sha256 = hex::encode(digest.finalize());
    let notes = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        format!("PicoAgent v{version}")
    };
    anyhow::ensure!(
        notes.len() <= MAX_UPDATE_NOTES_BYTES,
        "更新内容が長すぎます"
    );
    let feed = serde_json::json!({
        "schemaVersion": 1,
        "version": version,
        "notes": notes,
        "releasePage": format!("https://github.com/benybeny777/PicoAgent-Distribution/releases/tag/v{version}"),
        "platforms": {
            "windows-x64": {
                "url": format!("https://github.com/benybeny777/PicoAgent-Distribution/releases/download/v{version}/{name}"),
                "size": size,
                "sha256": sha256
            },
            "macos-arm64": { "url": "", "size": 0, "sha256": "" },
            "macos-x64": { "url": "", "size": 0, "sha256": "" }
        }
    });
    let destination = repo_root().join("distribution/site/latest.json");
    write_atomic(
        &destination,
        format!("{}\n", serde_json::to_string_pretty(&feed)?).as_bytes(),
    )?;
    log(
        "update-feed",
        format!("{} bytes / SHA-256 {}", size, sha256),
    );
    Ok(())
}

fn verify_windows_pe(path: &Path) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let mut dos_header = [0_u8; 64];
    file.read_exact(&mut dos_header)?;
    anyhow::ensure!(
        dos_header[0] == b'M' && dos_header[1] == b'Z',
        "Windows実行ファイルではありません"
    );
    let pe_offset = u32::from_le_bytes([
        dos_header[0x3c],
        dos_header[0x3d],
        dos_header[0x3e],
        dos_header[0x3f],
    ]) as u64;
    anyhow::ensure!(
        (64..=1024 * 1024).contains(&pe_offset),
        "Windows PEヘッダー位置が不正です"
    );
    file.seek(SeekFrom::Start(pe_offset))?;
    let mut signature = [0_u8; 4];
    file.read_exact(&mut signature)?;
    anyhow::ensure!(signature == *b"PE\0\0", "Windows PE署名がありません");
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("保存先の親フォルダがありません")?;
    fs::create_dir_all(parent)?;
    let nonce = std::process::id();
    let temporary = parent.join(format!(".update-feed-{nonce}.tmp"));
    let backup = parent.join(format!(".update-feed-{nonce}.backup"));
    let result = (|| -> Result<()> {
        if temporary.exists() {
            fs::remove_file(&temporary)?;
        }
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            fs::rename(path, &backup)?;
        }
        if let Err(error) = fs::rename(&temporary, path) {
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
        let _ = fs::remove_file(&temporary);
        if backup.exists() && !path.exists() {
            let _ = fs::rename(&backup, path);
        }
    }
    result
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
    let mut warnings = Vec::new();

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
    let updates_required = value
        .pointer("/updates/required")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if value
        .pointer("/updates/feed")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://"))
        .is_none()
    {
        blockers.push("最新版情報フィードのHTTPS URLが未設定です".to_string());
    }
    if updates_required {
        for (key, label) in [
            ("endpoint", "更新配信エンドポイント"),
            ("publicKey", "更新署名の公開鍵"),
        ] {
            if value
                .get("updates")
                .and_then(|item| item.get(key))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .is_empty()
            {
                blockers.push(format!("{label}が未設定です"));
            }
        }
    }
    let platforms = value
        .get("platforms")
        .and_then(Value::as_array)
        .context("platforms は配列で指定してください")?;
    if platforms.is_empty() {
        bail!("販売対象OSが1件も指定されていません");
    }
    if !updates_required {
        warnings.push(
            "初期販売は更新署名付きの完全自動更新なしです。利用者が明示操作する更新手順を販売対象OSごとに案内してください"
                .to_string(),
        );
    }
    let signing_required = value
        .pointer("/signing/required")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut unsigned_targets = Vec::new();
    for platform in platforms {
        let platform = platform
            .as_str()
            .context("platforms に文字列以外の値があります")?;
        let (key, label) = match platform {
            "windows" => {
                unsigned_targets.push("Windowsコード署名");
                ("windowsReady", "Windowsコード署名")
            }
            "macos" => {
                unsigned_targets.push("macOS署名・公証");
                ("macosReady", "macOS署名・公証")
            }
            other => {
                blockers.push(format!("未対応の販売対象OSです: {other}"));
                continue;
            }
        };
        if signing_required
            && value
                .get("signing")
                .and_then(|item| item.get(key))
                .and_then(Value::as_bool)
                != Some(true)
        {
            blockers.push(format!("{label}の準備が未完了です"));
        }
    }
    if !signing_required {
        let target_label = if unsigned_targets.is_empty() {
            "コード署名".to_string()
        } else {
            unsigned_targets.join("・")
        };
        warnings.push(format!(
            "初期販売は{}なしです。販売対象OSの未署名警告と正規配布元を販売ページで事前案内してください",
            target_label
        ));
    }

    // 販売版に入る第三者バイナリは同梱エンジン（llama-server）だけだが、その取得は
    // `releases/latest` 固定なしで、SHA-256 の照合も無い（Live2D Core の取得は照合している）。
    // 同じ PicoAgent の版でもビルド日ごとに中身が変わるため、出荷物を再現できず、
    // ライセンス台帳にどの版を同梱したかも書けない。販売前に版を固定して照合を入れる。
    if !bundled_engine_pinned(&root)? {
        warnings.push(
            "同梱エンジン（llama-server）が releases/latest 取得のままで、版の固定とSHA-256照合がありません。\
             販売物を再現できず第三者ライセンス台帳にも版を記載できないため、販売前に固定してください"
                .to_string(),
        );
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
    if updates_required
        && (!cargo_text.contains("tauri-plugin-updater")
            || !lib_text.contains("tauri_plugin_updater")
            || tauri_config.pointer("/bundle/createUpdaterArtifacts") != Some(&Value::Bool(true)))
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
    for warning in &warnings {
        eprintln!("  注意: {warning}");
    }
    if strict && !blockers.is_empty() {
        bail!("販売用ビルドを停止しました。上の未完了項目を解消してください");
    }
    Ok(serde_json::json!({
        "ready": blockers.is_empty(),
        "blockers": blockers,
        "warnings": warnings,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VersionBump {
    Major,
    Minor,
    Patch,
}

impl VersionBump {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "major" => Ok(Self::Major),
            "minor" => Ok(Self::Minor),
            "patch" => Ok(Self::Patch),
            _ => bail!("版の種別は major / minor / patch のいずれかを指定してください"),
        }
    }
}

fn next_version(current: &str, bump: VersionBump) -> Result<String> {
    let parts = current
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("現在の版がSemVer形式ではありません: {current}"))?;
    anyhow::ensure!(
        parts.len() == 3,
        "現在の版がSemVer形式ではありません: {current}"
    );
    let (major, minor, patch) = (parts[0], parts[1], parts[2]);
    let next = match bump {
        VersionBump::Major => (major.checked_add(1), Some(0), Some(0)),
        VersionBump::Minor => (Some(major), minor.checked_add(1), Some(0)),
        VersionBump::Patch => (Some(major), Some(minor), patch.checked_add(1)),
    };
    let (Some(major), Some(minor), Some(patch)) = next else {
        bail!("版番号が上限を超えました: {current}");
    };
    Ok(format!("{major}.{minor}.{patch}"))
}

fn replace_once(text: &str, before: &str, after: &str, label: &str) -> Result<String> {
    anyhow::ensure!(
        text.matches(before).count() == 1,
        "{label}の版番号が一意ではありません: {before}"
    );
    Ok(text.replacen(before, after, 1))
}

/// 公開中の版を起点にSemVerを1段上げ、アプリ・xtask・Tauri設定を一括同期する。
pub fn bump_version(args: &[String]) -> Result<()> {
    anyhow::ensure!(
        args.len() == 1,
        "使い方: cargo xtask bump-version <major|minor|patch>"
    );
    let bump = VersionBump::parse(&args[0])?;
    let root = repo_root();
    let app_path = root.join("src-tauri/Cargo.toml");
    let xtask_path = root.join("xtask/Cargo.toml");
    let tauri_path = root.join("src-tauri/tauri.conf.json");
    let feed_path = root.join("distribution/site/latest.json");
    let current = package_version(&app_path)?;
    anyhow::ensure!(
        package_version(&xtask_path)? == current,
        "xtaskの版がアプリと一致しません"
    );
    let feed: Value = serde_json::from_str(
        &fs::read_to_string(&feed_path)
            .with_context(|| format!("公開版情報を読めません: {}", feed_path.display()))?,
    )
    .context("latest.jsonが不正です")?;
    anyhow::ensure!(
        feed.get("version").and_then(Value::as_str) == Some(current.as_str()),
        "公開中の版とソースの版が一致しません。二重採番を防ぐため停止しました"
    );
    let tauri_text = fs::read_to_string(&tauri_path)?;
    let tauri: Value = serde_json::from_str(&tauri_text).context("tauri.conf.jsonが不正です")?;
    anyhow::ensure!(
        tauri.get("version").and_then(Value::as_str) == Some(current.as_str()),
        "Tauri設定の版がアプリと一致しません"
    );

    let next = next_version(&current, bump)?;
    let cargo_before = format!("version = \"{current}\"");
    let cargo_after = format!("version = \"{next}\"");
    let json_before = format!("\"version\": \"{current}\"");
    let json_after = format!("\"version\": \"{next}\"");
    let app_text = replace_once(
        &fs::read_to_string(&app_path)?,
        &cargo_before,
        &cargo_after,
        "アプリCargo.toml",
    )?;
    let xtask_text = replace_once(
        &fs::read_to_string(&xtask_path)?,
        &cargo_before,
        &cargo_after,
        "xtask Cargo.toml",
    )?;
    let tauri_text = replace_once(&tauri_text, &json_before, &json_after, "tauri.conf.json")?;
    write_atomic(&app_path, app_text.as_bytes())?;
    write_atomic(&xtask_path, xtask_text.as_bytes())?;
    write_atomic(&tauri_path, tauri_text.as_bytes())?;
    log("bump-version", format!("{current} -> {next} ({})", args[0]));
    Ok(())
}

/// 同梱エンジンの取得が版固定＋SHA-256照合になっているか。
///
/// 実装（`bundled::fetch_engine`）を読んで判定する。`releases/latest` を引いている間は
/// 固定されていない。ここを直したら判定も一緒に更新すること。
fn bundled_engine_pinned(root: &Path) -> Result<bool> {
    let text = fs::read_to_string(root.join("xtask/src/bundled.rs"))
        .context("xtask/src/bundled.rsを読めません")?;
    let Some(start) = text.find("pub fn fetch_engine(") else {
        bail!("fetch_engine が見つかりません（判定を更新してください）");
    };
    // 次に現れる関数定義までを本体とみなす。`\nfn ` だけを探すと `pub fn` が続く
    // 並びで本体が伸び、無関係な箇所の `sha256` を拾って「固定済み」と誤判定する。
    let body = &text[start + 1..];
    let end = ["\nfn ", "\npub fn ", "\npub(crate) fn "]
        .iter()
        .filter_map(|marker| body.find(marker))
        .min()
        .unwrap_or(body.len());
    let body = &body[..end];
    Ok(!body.contains("releases/latest") && body.contains("sha256"))
}

#[cfg(test)]
mod tests {
    use super::{
        check, included_characters, next_version, package_version, replace_once, verify_windows_pe,
        write_atomic, VersionBump,
    };

    #[test]
    fn 販売設定はキャラを重複なく列挙する() {
        let value = super::manifest().unwrap();
        let characters = included_characters(&value).unwrap();
        assert_eq!(
            characters,
            [
                "mugi",
                "rin",
                "mai",
                "koharu",
                "sena",
                "noa",
                "photoreal-prototype"
            ]
        );
    }

    #[test]
    fn 販売監査は機械可読なレポートを返す() {
        let report = check(false).unwrap();
        assert!(report["ready"].is_boolean());
        assert!(report["blockers"].is_array());
        assert!(report["warnings"].is_array());
        assert_eq!(
            report["version"],
            package_version(&crate::util::repo_root().join("src-tauri/Cargo.toml")).unwrap()
        );
    }

    #[test]
    fn 初期販売は署名と自動更新を必須にしない() {
        let report = check(false).unwrap();
        let blockers = report["blockers"].as_array().unwrap();
        let joined = blockers
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!joined.contains("コード署名"));
        assert!(!joined.contains("署名・公証"));
        assert!(!joined.contains("更新署名"));
        assert!(!joined.contains("Tauri updater"));
        let warnings = report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(warnings.contains("Windowsコード署名"));
        assert!(!warnings.contains("macOS Gatekeeper"));
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
    fn semverの種別ごとに次版を決める() {
        assert_eq!(next_version("1.2.3", VersionBump::Patch).unwrap(), "1.2.4");
        assert_eq!(next_version("1.2.3", VersionBump::Minor).unwrap(), "1.3.0");
        assert_eq!(next_version("1.2.3", VersionBump::Major).unwrap(), "2.0.0");
        assert!(next_version("1.2", VersionBump::Patch).is_err());
        assert!(VersionBump::parse("beta").is_err());
    }

    #[test]
    fn 版の置換は一意なときだけ許可する() {
        assert_eq!(
            replace_once("version = \"1.0.0\"", "1.0.0", "1.0.1", "test").unwrap(),
            "version = \"1.0.1\""
        );
        assert!(replace_once("1.0.0 1.0.0", "1.0.0", "1.0.1", "test").is_err());
        assert!(replace_once("none", "1.0.0", "1.0.1", "test").is_err());
    }

    #[test]
    fn アンインストールはユーザーデータを無確認で削除しない() {
        let root = crate::util::repo_root();
        for name in ["installer-hooks.nsh", "installer-hooks-lite.nsh"] {
            let script = std::fs::read_to_string(root.join("src-tauri").join(name)).unwrap();
            let prompt = script.find("MessageBox MB_YESNO").unwrap();
            let delete = script.find("RMDir /r \"$APPDATA\\PicoAgent\"").unwrap();
            assert!(prompt < delete, "{name}");
            assert!(
                script.contains("IfSilent picoagent_keep_user_data"),
                "{name}"
            );
        }
    }

    #[test]
    fn 軽量配布版は追加機能を一括同梱しない() {
        let root = crate::util::repo_root();
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("src-tauri/tauri.distribution.conf.json")).unwrap(),
        )
        .unwrap();
        let resources = config["bundle"]["resources"].as_array().unwrap();
        let joined = resources
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("bundled/engine"));
        for optional in ["aivis", "whisper", "tha3", "office", "tools"] {
            assert!(
                !joined.contains(optional),
                "{optional} を軽量配布へ入れない"
            );
        }
    }

    #[test]
    fn 最新版情報は原子的に置き換える() {
        let root = std::env::temp_dir().join(format!(
            "picoagent-update-feed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("latest.json");
        std::fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 最新版情報へ登録する実行ファイルはpe署名を持つ() {
        let path = std::env::temp_dir().join(format!(
            "picoagent-update-pe-{}-{}.exe",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&path, b"MZ only").unwrap();
        assert!(verify_windows_pe(&path).is_err());
        let mut pe = vec![0_u8; 0x84];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        std::fs::write(&path, pe).unwrap();
        assert!(verify_windows_pe(&path).is_ok());
        std::fs::remove_file(path).unwrap();
    }
}
