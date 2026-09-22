// text.rs - UTF-8文字列とストリーミング受信を安全に扱う共通処理。

/// 表示用に先頭 `max_chars` 文字まで切り詰め、切り詰めた場合だけ `…` を付ける。
///
/// 上限の直後にある文字境界を1回の走査で求める。文字数を数え直す実装だと、
/// 切り詰めの要らない短い文字列でも全体を3度走査することになる（この関数は
/// エージェントループのログ・引数プレビューなど、あらゆる経路から呼ばれる）。
pub fn preview(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((boundary, _)) => {
            let mut out = String::with_capacity(boundary + '…'.len_utf8());
            out.push_str(&text[..boundary]);
            out.push('…');
            out
        }
        None => text.to_string(),
    }
}

/// 末尾 `max_chars` 文字だけを残し、切り詰めた場合だけ先頭に `…` を付ける。
///
/// 外部コマンドのエラー出力は原因（例外名・最後のメッセージ）が末尾に出るため、
/// 先頭を残す `preview` では肝心の行が落ちる。
pub fn tail(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let mut out = String::from('…');
    out.extend(text.chars().skip(total - max_chars));
    out
}

/// 外部コマンドの出力をモデルへ返せる長さへ絞る上限（文字数）。
pub const COMMAND_OUTPUT_TAIL_CHARS: usize = 2000;

/// 外部コマンドの標準出力・標準エラーを文字列にし、末尾だけを残す。
///
/// 変換ツールは失敗時に進捗や警告を数千行出すことがあり、全文をツール結果へ入れると
/// 小型モデルの文脈を埋めて原因の行を読めなくする。
pub fn command_output(bytes: &[u8]) -> String {
    tail(&String::from_utf8_lossy(bytes), COMMAND_OUTPUT_TAIL_CHARS)
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
        let mut consumed = 0;
        loop {
            match std::str::from_utf8(&self.tail[consumed..]) {
                Ok(text) => {
                    out.push_str(text);
                    consumed = self.tail.len();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if let Ok(text) = std::str::from_utf8(&self.tail[consumed..consumed + valid]) {
                        out.push_str(text);
                    }
                    consumed += valid;
                    match error.error_len() {
                        Some(len) => {
                            out.push('\u{FFFD}');
                            consumed += len;
                        }
                        None => {
                            break;
                        }
                    }
                }
            }
        }
        // 不正バイトごとに残りを移動せず、次チャンクへ持ち越す末尾だけを一度で残す。
        self.tail.drain(..consumed);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{preview, tail, truncate_bytes, Utf8Stream};

    #[test]
    fn tailは末尾を残して先頭に省略記号を付ける() {
        assert_eq!(tail("あいうえお", 10), "あいうえお");
        assert_eq!(tail("あいうえお", 2), "…えお");
    }

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

    #[test]
    fn 不正バイトと分断された多バイト文字を混ぜても復元する() {
        let mut bytes = vec![0xff; 16_384];
        bytes.extend_from_slice("日本語😀".as_bytes());
        bytes.extend_from_slice(&[0xf0, 0x80, b'x', 0xe3, 0x81, 0x82]);
        let expected = String::from_utf8_lossy(&bytes);
        for chunk_size in [1, 2, 3, 7, 1024, bytes.len()] {
            let mut stream = Utf8Stream::default();
            let actual: String = bytes
                .chunks(chunk_size)
                .map(|chunk| stream.push(chunk))
                .collect();
            assert_eq!(actual, expected, "チャンク幅: {chunk_size}");
        }
    }

    #[test]
    fn 不完全な末尾を空チャンクで失わず次の入力へ残す() {
        let mut stream = Utf8Stream::default();
        assert_eq!(stream.push(&[0xff, 0xe3, 0x81]), "\u{FFFD}");
        assert_eq!(stream.push(&[]), "");
        assert_eq!(stream.push(&[0x82, b'z']), "あz");
        assert_eq!(stream.push(&[]), "");
    }
}
