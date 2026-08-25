// secret.rs - APIキー等のシークレットをOSの資格情報保護へ預けて保存する。
//
// Windows は DPAPI で暗号化し、暗号文を `config.json` に置く。同一Windowsユーザーだけが復号できる。
// macOS は値そのものをログインキーチェーンへ預け、`config.json` には項目名だけを置く。
// どちらも同じPC・同じOSユーザーでだけ元の値へ戻せる。
// `config.get()` が読み取り時に透過的に復号し、設定保存時にパスワード型の項目だけを保護する。
// Linux は同等のOS機能を使っていないため平文で保存する（フォールバック許可: OS機能が無いため）。

#[cfg(windows)]
use base64::Engine;

/// Windows DPAPIで暗号化した値。本体は base64(DPAPI出力) で、秘密は`config.json`内にある。
const DPAPI_PREFIX: &str = "enc:dpapi:v1:";
/// macOSキーチェーンへ預けた値。本体は項目のアカウント名（＝設定キー名）だけで、秘密は入らない。
const KEYCHAIN_PREFIX: &str = "enc:keychain:v1:";

/// 名前は秘密っぽいが実際は秘密でないキー（意図的な例外のみ。理由を添えること）。
/// - `PICOAGENT_HTTP_API_KEY_HEADER`: APIキーを載せる「ヘッダー名」。値そのものは
///   `PICOAGENT_HTTP_API_KEY_VALUE`（password 型）側で扱う
const NOT_SECRET: [&str; 1] = ["PICOAGENT_HTTP_API_KEY_HEADER"];

/// 秘密値を示す語（キー名に含まれたら秘密として扱う）。
const SECRET_WORDS: [&str; 6] = [
    "API_KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "WEBHOOK",
    "CREDENTIAL",
];

/// 設定キー名から「秘密として扱うべき値か」を判定する。
///
/// コネクタの `config_schema()` に載らない設定キー（設定画面が直接組み立てる項目など）も
/// あるため、**型の申告だけに頼らず名前でも判定する**。実際に `PICOAGENT_API_KEY`
/// （ローカル API のキー）はどのスキーマにも載っておらず、平文で保存されていた。
pub fn is_secret_key(key: &str) -> bool {
    !NOT_SECRET.contains(&key) && SECRET_WORDS.iter().any(|word| key.contains(word))
}

/// 値が本モジュールで保護済みか。復号できないOSの形式（別OSのconfig.jsonを持ち込んだ場合）も真になる。
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(DPAPI_PREFIX) || value.starts_with(KEYCHAIN_PREFIX)
}

/// シークレット値を保存用文字列にする。空・保護済み・非対応時はそのまま返す。
///
/// `key` は設定キー名。macOSではキーチェーン項目のアカウント名になるため、同じ設定を
/// 上書き保存しても項目が増えない（古い秘密が孤児として残らない）。
pub fn encrypt(key: &str, plain: &str) -> Result<String, String> {
    if plain.is_empty() || is_encrypted(plain) {
        return Ok(plain.to_string());
    }
    #[cfg(windows)]
    {
        let _ = key;
        if let Some(bytes) = win::protect(plain.as_bytes()) {
            return Ok(format!(
                "{DPAPI_PREFIX}{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ));
        }
        Err("Windowsの資格情報保護で暗号化できないため、シークレットを保存しません".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        if !mac::is_valid_account(key) {
            return Err(format!("キーチェーンへ預けられない設定キー名です: {key}"));
        }
        mac::store(key, plain).map_err(|error| {
            format!("macOSのキーチェーンへ保存できないため、シークレットを保存しません（{error}）")
        })?;
        Ok(format!("{KEYCHAIN_PREFIX}{key}"))
    }
    // フォールバック許可: DPAPIもキーチェーンも無いため平文で保存する。
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = key;
        Ok(plain.to_string())
    }
}

/// 保存用文字列を平文へ復号する。保護されていなければそのまま返す。
pub fn decrypt(value: &str) -> String {
    if !is_encrypted(value) {
        return value.to_string();
    }
    #[cfg(windows)]
    if let Some(encoded) = value.strip_prefix(DPAPI_PREFIX) {
        if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(encoded) {
            if let Some(plain) = win::unprotect(&raw) {
                if let Ok(text) = String::from_utf8(plain) {
                    return text;
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(account) = value.strip_prefix(KEYCHAIN_PREFIX) {
        if mac::is_valid_account(account) {
            if let Some(plain) = mac::load(account) {
                return plain;
            }
        }
    }
    // フォールバック許可: 別ユーザー/別PC/別OS/破損で復号できない場合は元の文字列を返す。
    // 呼び出し側は `is_encrypted()` が真のままであることで「この端末では使えない値」と判断する。
    value.to_string()
}

/// 保存済みシークレットの実体を消す。設定画面で値を空にしたとき、OS側へ古い秘密を残さない。
pub fn forget(key: &str) {
    // macOSは秘密がキーチェーン側にあるため、`config.json`を空にするだけでは実体が残る。
    #[cfg(target_os = "macos")]
    if mac::is_valid_account(key) {
        mac::remove(key);
    }
    // Windows DPAPIとLinuxは値が`config.json`内にあり、空にすれば実体も消える。
    #[cfg(not(target_os = "macos"))]
    let _ = key;
}

/// macOSのログインキーチェーンへシークレットを預ける。
#[cfg(target_os = "macos")]
mod mac {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    /// キーチェーン上のサービス名。「キーチェーンアクセス」で PicoAgent の項目としてまとまる。
    const SERVICE: &str = "PicoAgent";

    /// 参照に使えるアカウント名か。設定キーはASCIIの `PICOAGENT_*` だけなので、
    /// `config.json` を手で書き換えても想定外の項目を引かないよう文字種を絞る。
    pub fn is_valid_account(account: &str) -> bool {
        !account.is_empty()
            && account.len() <= 256
            && account
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    }

    /// 同じサービス・アカウントの項目があれば上書きする（増殖しない）。
    pub fn store(account: &str, plain: &str) -> Result<(), String> {
        set_generic_password(SERVICE, account, plain.as_bytes()).map_err(|error| error.to_string())
    }

    pub fn load(account: &str) -> Option<String> {
        String::from_utf8(get_generic_password(SERVICE, account).ok()?).ok()
    }

    pub fn remove(account: &str) {
        // 項目が無い場合も errSecItemNotFound で失敗するため、結果は捨てる。
        let _ = delete_generic_password(SERVICE, account);
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    fn out_blob() -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: core::ptr::null_mut(),
        }
    }

    fn in_blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8, // DPAPI は入力を変更しない
        }
    }

    // 出力 BLOB を Vec へコピーし、DPAPI が確保したメモリを LocalFree で解放する。
    unsafe fn take_output(output: CRYPT_INTEGER_BLOB) -> Option<Vec<u8>> {
        if output.pbData.is_null() {
            return None;
        }
        let bytes = core::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut core::ffi::c_void);
        Some(bytes)
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        let input = in_blob(data);
        let mut output = out_blob();
        let ok = unsafe {
            CryptProtectData(
                &input,
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            return None;
        }
        unsafe { take_output(output) }
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        let input = in_blob(data);
        let mut output = out_blob();
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                core::ptr::null_mut(),
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            return None;
        }
        unsafe { take_output(output) }
    }
}

#[cfg(test)]
mod tests {
    use super::{decrypt, encrypt, is_encrypted};

    #[test]
    fn 暗号化往復で元の値に戻る() {
        // macOSはキーチェーン（OSの実体）へ書き込むため、単体テストでは往復させない。
        // 未署名バイナリのキーチェーン許可ダイアログでテストが止まる。実機QAで確認する。
        #[cfg(not(target_os = "macos"))]
        {
            let plain = "sk-test-APIKEY-0123456789-あいう";
            let stored = encrypt("TEST_API_KEY", plain).unwrap();
            assert_eq!(decrypt(&stored), plain);
            // Windows では実際に暗号化され、平文とは別の文字列になる。
            #[cfg(windows)]
            {
                assert!(is_encrypted(&stored));
                assert_ne!(stored, plain);
            }
        }
    }

    #[test]
    fn 別環境で作られた保護値は復号せず保護済みのまま返す() {
        // Windowsの`config.json`をmacOSへ持ち込んだ場合とその逆。復号できないので
        // 保護済み判定を保ったまま元の文字列を返し、呼び出し側が未設定として扱う。
        for stored in [
            "enc:dpapi:v1:これはbase64ではない",
            "enc:keychain:v1:UNSTORED_API_KEY",
        ] {
            assert!(is_encrypted(stored));
            assert!(is_encrypted(&decrypt(stored)));
        }
    }

    #[test]
    fn キー名から秘密かどうかを判定する() {
        use super::is_secret_key;
        // 実際に平文保存されていたキー（どのスキーマにも載っていなかった）。
        assert!(is_secret_key("PICOAGENT_API_KEY"));
        assert!(is_secret_key("PICOAGENT_OPENAI_API_KEY"));
        assert!(is_secret_key("PICOAGENT_SLACK_WEBHOOK_URL"));
        assert!(is_secret_key("PICOAGENT_GITHUB_TOKEN"));
        // ヘッダー名は値そのものではないので除外する。
        assert!(!is_secret_key("PICOAGENT_HTTP_API_KEY_HEADER"));
        assert!(!is_secret_key("PICOAGENT_ALLOWED_ROOT"));
        assert!(!is_secret_key("PICOAGENT_MODEL"));
    }

    #[test]
    fn 非暗号文字列はそのまま扱う() {
        assert!(!is_encrypted("plain-token"));
        assert_eq!(decrypt("plain-token"), "plain-token");
        assert_eq!(encrypt("TEST_API_KEY", "").unwrap(), "");
    }
}

/// 設計ガード: シークレット設定が `password` 型で申告されているかを検査する。
///
/// `type="password"` の値だけが DPAPI 暗号化の経路に乗る（`config.rs`）。
/// 新しいスキルが API キーを `"text"` で申告すると、警告も出ないまま
/// `config.json` へ平文で保存されてしまう。人のレビューに頼らずテストで検出する。
#[cfg(test)]
mod guard {
    use std::path::Path;

    use super::is_secret_key;

    use crate::guard_scan;

    #[test]
    fn シークレット設定はpassword型で申告する() {
        let mut violations = Vec::new();
        // スキルもプロバイダも、`Field::new` を書きうる場所はすべて走査する。
        for source in guard_scan::all_sources() {
            let file = source.path.display().to_string();
            // Field::new("KEY", "ラベル", "型", ...) の 1・3 引数を素朴に取り出す。
            for chunk in source.body().split("Field::new(").skip(1) {
                let mut parts = chunk.split('"').skip(1).step_by(2);
                let (Some(key), Some(_label), Some(kind)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                if kind == "password" {
                    continue;
                }
                if is_secret_key(key) {
                    violations.push(format!("{key} (型: {kind}) in {file}"));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "シークレット設定が password 型で申告されていません（平文で config.json へ保存されます）:\n{}",
            violations.join("\n")
        );
    }

    /// 設定画面が直接組み立てる `field("KEY", …, "password")` も暗号化経路に乗ること。
    ///
    /// スキーマ（`Field::new`）に載らない項目はコネクタ走査では見つからない。実際に
    /// ローカル API の `PICOAGENT_API_KEY` がこの経路で、平文のまま保存されていた。
    /// 暗号化の要否はキー名で判定するようにしたので、**設定画面の password 項目は
    /// すべて `is_secret_key()` が真になる名前でなければならない**。
    #[test]
    fn 設定画面のpassword項目も暗号化経路に乗る() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri の親");
        let source =
            std::fs::read_to_string(root.join("ui/settings.js")).expect("settings.js を読めない");

        let mut found = 0;
        let mut violations = Vec::new();
        // field("PICOAGENT_XXX", "ラベル", "password", …) の 1・3 引数を取り出す。
        for chunk in source.split("field(\"PICOAGENT_").skip(1) {
            let mut parts = chunk.split('"');
            let Some(rest_of_key) = parts.next() else {
                continue;
            };
            let key = format!("PICOAGENT_{rest_of_key}");
            let mut quoted = parts.skip(1).step_by(2);
            let (Some(_label), Some(kind)) = (quoted.next(), quoted.next()) else {
                continue;
            };
            if kind != "password" {
                continue;
            }
            found += 1;
            if !is_secret_key(&key) {
                violations.push(key);
            }
        }
        assert!(found > 0, "設定画面の password 項目を検出できていません");
        assert!(
            violations.is_empty(),
            "設定画面の password 項目が暗号化経路に乗りません（平文で config.json へ保存されます）。\
             キー名に API_KEY / TOKEN / SECRET / PASSWORD / WEBHOOK / CREDENTIAL を含めるか、\
             secret::is_secret_key を拡張してください: {violations:?}"
        );
    }
}
