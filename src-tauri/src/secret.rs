// secret.rs - APIキー等のシークレットを Windows DPAPI で暗号化して保存する。
//
// 同一 Windows ユーザーアカウントだけが復号でき、config.json への平文保存を避ける。
// 保存形式は "enc:dpapi:v1:" + base64(DPAPI出力)。`config.get()` が読み取り時に透過的に復号し、
// 設定保存時にパスワード型の項目だけを暗号化する。
// 非 Windows では DPAPI が無いため平文で保存する（フォールバック許可: OS機能が無いため）。

#[cfg(windows)]
use base64::Engine;

const PREFIX: &str = "enc:dpapi:v1:";

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

/// 値が本モジュールで暗号化済みか。
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(PREFIX)
}

/// シークレット値を暗号化した保存用文字列にする。空・暗号化済み・非対応時はそのまま返す。
pub fn encrypt(plain: &str) -> String {
    if plain.is_empty() || is_encrypted(plain) {
        return plain.to_string();
    }
    #[cfg(windows)]
    {
        if let Some(bytes) = win::protect(plain.as_bytes()) {
            return format!(
                "{PREFIX}{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
        }
    }
    // フォールバック許可: DPAPI 非対応（非Windows）や失敗時は平文で保存する（保存自体は止めない）。
    plain.to_string()
}

/// 保存用文字列を平文へ復号する。暗号化されていなければそのまま返す。
pub fn decrypt(value: &str) -> String {
    if !is_encrypted(value) {
        return value.to_string();
    }
    #[cfg(windows)]
    {
        let encoded = &value[PREFIX.len()..];
        if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(encoded) {
            if let Some(plain) = win::unprotect(&raw) {
                if let Ok(text) = String::from_utf8(plain) {
                    return text;
                }
            }
        }
    }
    // フォールバック許可: 別ユーザー/別PC/破損で復号できない場合は元の文字列を返す。
    value.to_string()
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
        let plain = "sk-test-APIKEY-0123456789-あいう";
        let stored = encrypt(plain);
        assert_eq!(decrypt(&stored), plain);
        // Windows では実際に暗号化され、平文とは別の文字列になる。
        #[cfg(windows)]
        {
            assert!(is_encrypted(&stored));
            assert_ne!(stored, plain);
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
        assert_eq!(encrypt(""), "");
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

    fn sources() -> Vec<(String, String)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri の親");
        let mut out = Vec::new();
        for dir in [root.join("skills"), root.join("src-tauri/src/providers")] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let file = if path.is_dir() {
                    path.join("mod.rs")
                } else {
                    path
                };
                if file.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&file) {
                    out.push((file.to_string_lossy().to_string(), text));
                }
            }
        }
        assert!(
            out.len() > 40,
            "ソースの読み込みに失敗している可能性: {}",
            out.len()
        );
        out
    }

    #[test]
    fn シークレット設定はpassword型で申告する() {
        let mut violations = Vec::new();
        for (file, text) in sources() {
            // Field::new("KEY", "ラベル", "型", ...) の 1・3 引数を素朴に取り出す。
            for chunk in text.split("Field::new(").skip(1) {
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
