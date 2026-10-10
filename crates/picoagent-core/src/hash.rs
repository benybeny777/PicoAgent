// hash - 呼出元が渡す読取ストリームのSHA-256を扱う共通責務。
use sha2::{Digest, Sha256};
use std::io::{self, Read};

pub fn sha256_reader(reader: &mut impl Read) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn 空と通常内容と読取不能を同じ読取ストリーム検査で扱う() {
        assert_eq!(
            super::sha256_reader(&mut &b""[..]).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            super::sha256_reader(&mut &b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        struct Rejected;
        impl std::io::Read for Rejected {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "検査用読取拒否",
                ))
            }
        }
        assert_eq!(
            super::sha256_reader(&mut Rejected).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
