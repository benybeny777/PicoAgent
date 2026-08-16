//! 販売者用ライセンス署名ツール。秘密鍵は必ずリポジトリ外へ保存する。

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use rand::{rngs::SysRng, TryRng};
use serde::Serialize;

use crate::util::repo_root;

#[derive(Serialize)]
struct Claims<'a> {
    version: u8,
    edition: &'a str,
    customer: &'a str,
    purchase_id: &'a str,
    issued_at: i64,
    expires_at: Option<i64>,
}

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("keygen") => keygen(),
        Some("issue") => issue(&args[1..]),
        _ => bail!("使い方: cargo xtask license keygen | license issue --customer <名前> --purchase-id <注文番号> [--edition pro|business] [--expires-at UNIX秒]"),
    }
}

fn private_key_path() -> Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA").context("APPDATA が見つかりません")?;
    Ok(PathBuf::from(appdata)
        .join("PicoAgent-vendor")
        .join("license-signing.key"))
}

fn keygen() -> Result<()> {
    let private_path = private_key_path()?;
    if private_path.exists() {
        bail!("秘密鍵は既に存在します: {}", private_path.display());
    }
    let mut secret = [0_u8; 32];
    SysRng
        .try_fill_bytes(&mut secret)
        .context("OSから署名鍵用の乱数を取得できません")?;
    let signing = SigningKey::from_bytes(&secret);
    fs::create_dir_all(
        private_path
            .parent()
            .context("秘密鍵フォルダがありません")?,
    )?;
    fs::write(&private_path, URL_SAFE_NO_PAD.encode(signing.to_bytes()))
        .with_context(|| format!("秘密鍵を書けません: {}", private_path.display()))?;
    let public_path = repo_root().join("src-tauri/resources/license-public-key.txt");
    fs::write(
        &public_path,
        format!(
            "{}\n",
            URL_SAFE_NO_PAD.encode(signing.verifying_key().to_bytes())
        ),
    )?;
    println!(
        "署名鍵を生成しました。秘密鍵はリポジトリ外に保存しました: {}",
        private_path.display()
    );
    println!("公開鍵を製品へ設定しました: {}", public_path.display());
    Ok(())
}

fn issue(args: &[String]) -> Result<()> {
    let value = |name: &str| -> Option<&str> {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
            .map(String::as_str)
    };
    let customer = value("--customer").context("--customer が必要です")?;
    let purchase_id = value("--purchase-id").context("--purchase-id が必要です")?;
    let edition = value("--edition").unwrap_or("pro");
    if !matches!(edition, "pro" | "business") {
        bail!("--edition は pro または business です");
    }
    let expires_at = value("--expires-at")
        .map(str::parse)
        .transpose()
        .context("--expires-at は UNIX 秒で指定してください")?;
    let private_path = private_key_path()?;
    let encoded = fs::read_to_string(&private_path).with_context(|| {
        format!(
            "秘密鍵を読めません。先に keygen を実行してください: {}",
            private_path.display()
        )
    })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .context("秘密鍵が壊れています")?;
    let key_bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("秘密鍵の長さが正しくありません"))?;
    let signing = SigningKey::from_bytes(&key_bytes);
    let claims = Claims {
        version: 1,
        edition,
        customer,
        purchase_id,
        issued_at: chrono::Utc::now().timestamp(),
        expires_at,
    };
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
    let signed = format!("PA1.{payload}");
    let signature = URL_SAFE_NO_PAD.encode(signing.sign(signed.as_bytes()).to_bytes());
    println!("{signed}.{signature}");
    Ok(())
}
