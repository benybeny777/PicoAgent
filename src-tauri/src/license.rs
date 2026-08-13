//! 販売版ライセンスの検証と権限制御。

#[cfg(any(not(feature = "beta-full"), test))]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
#[cfg(any(not(feature = "beta-full"), test))]
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
#[cfg(any(not(feature = "beta-full"), test))]
use serde::Deserialize;
use serde::Serialize;

use crate::config::Config;

#[cfg(not(feature = "beta-full"))]
pub const CONFIG_KEY: &str = "PICOAGENT_LICENSE_TOKEN";
#[cfg(not(feature = "beta-full"))]
const PUBLIC_KEY_B64: &str = include_str!("../resources/license-public-key.txt");

#[cfg(any(not(feature = "beta-full"), test))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub version: u8,
    pub edition: String,
    pub customer: String,
    pub purchase_id: String,
    pub issued_at: i64,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub active: bool,
    pub edition: String,
    pub customer: String,
    pub message: String,
}

#[cfg(feature = "beta-full")]
pub fn status(_cfg: &Config) -> Status {
    Status {
        active: true,
        edition: "beta".to_string(),
        customer: String::new(),
        message: "ベータ期間中は全機能を試用できます。".to_string(),
    }
}

#[cfg(not(feature = "beta-full"))]
pub fn status(cfg: &Config) -> Status {
    let token = cfg.get(CONFIG_KEY, "");
    match verify(&token) {
        Ok(claims) => Status {
            active: true,
            edition: claims.edition,
            customer: claims.customer,
            message: "ライセンスは有効です。".to_string(),
        },
        Err(error) => Status {
            active: false,
            edition: "free".to_string(),
            customer: String::new(),
            message: if token.is_empty() {
                "無料版として利用中です。".to_string()
            } else {
                error
            },
        },
    }
}

#[cfg(all(not(test), not(feature = "beta-full")))]
pub fn is_pro(cfg: &Config) -> bool {
    status(cfg).active
}

#[cfg(any(test, feature = "beta-full"))]
pub fn is_pro(_cfg: &Config) -> bool {
    // テストは安全境界まで到達させる。beta-fullは試用版だけの明示的な全機能開放。
    true
}

#[cfg(not(feature = "beta-full"))]
pub fn verify(token: &str) -> Result<Claims, String> {
    let public_bytes = URL_SAFE_NO_PAD
        .decode(PUBLIC_KEY_B64.trim())
        .map_err(|_| "製品のライセンス公開鍵が壊れています。".to_string())?;
    let public_array: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| "製品のライセンス公開鍵が壊れています。".to_string())?;
    let key = VerifyingKey::from_bytes(&public_array)
        .map_err(|_| "製品のライセンス公開鍵が壊れています。".to_string())?;
    verify_with_key(token, &key, chrono::Utc::now().timestamp())
}

#[cfg(any(not(feature = "beta-full"), test))]
fn verify_with_key(token: &str, key: &VerifyingKey, now: i64) -> Result<Claims, String> {
    let mut parts = token.trim().split('.');
    if parts.next() != Some("PA1") {
        return Err("ライセンスキーの形式が正しくありません。".to_string());
    }
    let payload_part = parts
        .next()
        .ok_or_else(|| "ライセンスキーが途中で切れています。".to_string())?;
    let signature_part = parts
        .next()
        .ok_or_else(|| "ライセンス署名がありません。".to_string())?;
    if parts.next().is_some() {
        return Err("ライセンスキーの形式が正しくありません。".to_string());
    }
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(signature_part)
        .map_err(|_| "ライセンス署名を読めません。".to_string())?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| "ライセンス署名を読めません。".to_string())?;
    let signed = format!("PA1.{payload_part}");
    key.verify(signed.as_bytes(), &signature)
        .map_err(|_| "ライセンス署名を確認できません。".to_string())?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload_part)
        .map_err(|_| "ライセンス情報を読めません。".to_string())?;
    let claims: Claims =
        serde_json::from_slice(&payload).map_err(|_| "ライセンス情報を読めません。".to_string())?;
    if claims.version != 1 || !matches!(claims.edition.as_str(), "pro" | "business") {
        return Err("この製品では使えないライセンスです。".to_string());
    }
    if claims.expires_at.is_some_and(|value| value < now) {
        return Err("ライセンスの有効期限が切れています。".to_string());
    }
    Ok(claims)
}

pub fn tool_requires_pro(name: &str) -> bool {
    name.starts_with("mcp__")
        || matches!(
            name,
            "app_control"
                | "browser_control"
                | "bluesky"
                | "code_runner"
                | "command_job"
                | "desktop_control"
                | "discord_send_message"
                | "document_convert"
                | "edit_file"
                | "gmail_message"
                | "google_calendar"
                | "google_docs"
                | "google_drive"
                | "google_sheets"
                | "google_tasks"
                | "github"
                | "http_api"
                | "jira"
                | "linear"
                | "linkedin"
                | "mastodon"
                | "meeting_recorder"
                | "misskey"
                | "microsoft_graph"
                | "ms_office"
                | "notion"
                | "pdf_tools"
                | "project"
                | "run_command"
                | "slack_send_message"
                | "subagent"
                | "task_scheduler"
                | "todoist"
                | "trello"
                | "trash_file"
                | "workflow"
                | "workspace_files"
                | "x_twitter"
        )
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn 高度な操作だけをpro対象にする() {
        assert!(super::tool_requires_pro("browser_control"));
        assert!(super::tool_requires_pro("mcp__sample__tool"));
        assert!(!super::tool_requires_pro("weather"));
        assert!(!super::tool_requires_pro("chitchat"));
    }

    #[test]
    fn 正規の署名だけを受け付け期限も検査する() {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let payload = serde_json::json!({
            "version": 1, "edition": "pro", "customer": "テスト購入者",
            "purchase_id": "ORDER-1", "issued_at": 100, "expires_at": 200
        });
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let signed = format!("PA1.{encoded}");
        let token = format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(signing.sign(signed.as_bytes()).to_bytes())
        );
        assert_eq!(
            super::verify_with_key(&token, &signing.verifying_key(), 150)
                .unwrap()
                .purchase_id,
            "ORDER-1"
        );
        assert!(super::verify_with_key(&token, &signing.verifying_key(), 201).is_err());
        assert!(super::verify_with_key(&(token + "x"), &signing.verifying_key(), 150).is_err());
    }
}
