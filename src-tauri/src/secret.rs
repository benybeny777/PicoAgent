// secret.rs - APIキー等のシークレットを Windows DPAPI で暗号化して保存する。
//
// 同一 Windows ユーザーアカウントだけが復号でき、config.json への平文保存を避ける。
// 保存形式は "enc:dpapi:v1:" + base64(DPAPI出力)。`config.get()` が読み取り時に透過的に復号し、
// 設定保存時にパスワード型の項目だけを暗号化する。
// 非 Windows では DPAPI が無いため平文で保存する（フォールバック許可: OS機能が無いため）。

use base64::Engine;

const PREFIX: &str = "enc:dpapi:v1:";

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
    fn 非暗号文字列はそのまま扱う() {
        assert!(!is_encrypted("plain-token"));
        assert_eq!(decrypt("plain-token"), "plain-token");
        assert_eq!(encrypt(""), "");
    }
}
