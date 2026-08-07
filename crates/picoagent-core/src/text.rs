// text.rs - UTF-8文字列とストリーミング受信を安全に扱う共通処理。

/// 表示用に先頭 `max_chars` 文字まで切り詰め、切り詰めた場合だけ `…` を付ける。
pub fn preview(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if out.chars().count() < text.chars().count() {
        out.push('…');
    }
    out
}

/// バイト上限で、UTF-8文字の途中を避けて切り詰める。
pub fn truncate_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// チャンク境界で分断されたUTF-8末尾を、次のチャンクまで保持する逐次デコーダ。
#[derive(Default)]
pub struct Utf8Stream {
    tail: Vec<u8>,
}

impl Utf8Stream {
    /// 確定した文字列だけを返し、不完全な末尾は内部へ保持する。
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.tail.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.tail) {
                Ok(text) => {
                    out.push_str(text);
                    self.tail.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if let Ok(text) = std::str::from_utf8(&self.tail[..valid]) {
                        out.push_str(text);
                    }
                    match error.error_len() {
                        Some(len) => {
                            out.push('\u{FFFD}');
                            self.tail.drain(..valid + len);
                        }
                        None => {
                            self.tail.drain(..valid);
                            break;
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{preview, truncate_bytes, Utf8Stream};

    #[test]
    fn 日本語を安全に切り詰める() {
        assert_eq!(preview("こんにちは", 3), "こんに…");
        assert_eq!(truncate_bytes("あああ", 4), "あ");
        assert_eq!(truncate_bytes("abc", 2), "ab");
    }

    #[test]
    fn 分断された日本語を逐次復元する() {
        let source = "こんにちは、今日はいい天気ですね。";
        let mut stream = Utf8Stream::default();
        let mut out = String::new();
        for byte in source.as_bytes() {
            out.push_str(&stream.push(&[*byte]));
        }
        assert_eq!(out, source);
        assert!(!out.contains('\u{FFFD}'));
    }

    #[test]
    fn 壊れたバイト列を置換して先へ進む() {
        let mut stream = Utf8Stream::default();
        assert_eq!(stream.push(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
    }
}
