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

    #[test]
    fn 短い読取を最後まで計算し途中失敗を成功した部分hashへ降格しない() {
        struct Pieces {
            remaining: &'static [u8],
            width: usize,
            fail_after: Option<usize>,
            reads: usize,
        }
        impl std::io::Read for Pieces {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.fail_after == Some(self.reads) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "検査用途中失敗",
                    ));
                }
                self.reads += 1;
                let count = self.remaining.len().min(self.width).min(buffer.len());
                buffer[..count].copy_from_slice(&self.remaining[..count]);
                self.remaining = &self.remaining[count..];
                Ok(count)
            }
        }
        for width in [1, 2, 3] {
            let mut reader = Pieces {
                remaining: b"abc",
                width,
                fail_after: None,
                reads: 0,
            };
            assert_eq!(
                super::sha256_reader(&mut reader).unwrap(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
        }
        let mut reader = Pieces {
            remaining: b"abc",
            width: 1,
            fail_after: Some(1),
            reads: 0,
        };
        let error = super::sha256_reader(&mut reader).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(error.to_string().contains("検査用途中失敗"));
    }
}
