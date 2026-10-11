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

/// バイト上限を超えた場合だけ文字境界で切り詰め、切ったことを示す `notice` を末尾へ付ける。
///
/// 黙って切ると、読み手（モデル）は途中までの出力を全文として扱ってしまう。
/// 上限は本文だけに適用し、呼び出し側が渡した注記は上限に含めない。
pub fn truncate_bytes_with_notice(text: &mut String, max_bytes: usize, notice: &str) {
    if text.len() <= max_bytes {
        return;
    }
    let end = truncate_bytes(text, max_bytes).len();
    text.truncate(end);
    text.push_str(notice);
}

/// バイト上限で切ったことを、上限を十進MBにして伝える末尾の注記。
pub fn byte_limit_notice(max_bytes: usize) -> String {
    // 小さい正の上限を0MBへ丸めず、音声処理の既存の小数3桁も保持する。
    let precision = if max_bytes > 0 && max_bytes < 1000 {
        6
    } else {
        3
    };
    format!(
        "\n…（約{:.precision$}MBで打ち切り）",
        max_bytes as f64 / 1_000_000.0
    )
}

/// 外部コマンドの出力をバイト上限まで文字列にし、切った場合は上限を添える。
pub fn command_output_within(bytes: &[u8], max_bytes: usize) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if text.len() > max_bytes {
        truncate_bytes_with_notice(&mut text, max_bytes, &byte_limit_notice(max_bytes));
    }
    text
}

/// ラベルと本文を分け、ラベル・注記を除いた本文の変換後サイズだけを制限する。
pub fn labeled_command_output_within(label: &str, bytes: &[u8], max_bytes: usize) -> String {
    format!("{label}:\n{}", command_output_within(bytes, max_bytes))
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
    use super::{
        byte_limit_notice, command_output_within, labeled_command_output_within, preview, tail,
        truncate_bytes, truncate_bytes_with_notice, Utf8Stream,
    };

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
    fn 上限を超えたときだけ文字境界で切って打ち切りを添える() {
        let mut fits = "あい".to_string();
        truncate_bytes_with_notice(&mut fits, 6, "[切]");
        assert_eq!(fits, "あい");
        let mut cut = "あいう".to_string();
        truncate_bytes_with_notice(&mut cut, 7, "[切]");
        assert_eq!(cut, "あい[切]");
        let mut empty = String::new();
        truncate_bytes_with_notice(&mut empty, 0, "[切]");
        assert_eq!(empty, "");
        let mut zero = "あ".to_string();
        truncate_bytes_with_notice(&mut zero, 0, "[切]");
        assert_eq!(zero, "[切]");
        assert_eq!(command_output_within("abc".as_bytes(), 3), "abc");
        assert!(command_output_within("あいう".as_bytes(), 4).starts_with("あ\n…（約"));
        assert_eq!(
            command_output_within(&vec![b'x'; 40_000], 32 * 1024).len(),
            32 * 1024 + "\n…（約0.033MBで打ち切り）".len()
        );
    }

    #[test]
    fn 注記は小さい正の上限もゼロへ丸めず十進の容量を伝える() {
        for (limit, expected) in [
            (1, "0.000001"),
            (0, "0.000"),
            (999, "0.000999"),
            (1000, "0.001"),
            (16 * 1024, "0.016"),
            (32 * 1024, "0.033"),
        ] {
            assert_eq!(
                byte_limit_notice(limit),
                format!("\n…（約{expected}MBで打ち切り）")
            );
        }
    }

    #[test]
    fn 置換文字で増えた変換後本文も上限を守りラベルと注記を保持する() {
        for (bytes, limit, expected) in [
            (&[0xff][..], 2, ""),
            (&[0xff, 0xff][..], 4, "�"),
            ("あ😀い".as_bytes(), 6, "あ"),
            (b"abcdef".as_slice(), 0, ""),
        ] {
            let notice = byte_limit_notice(limit);
            let output = command_output_within(bytes, limit);
            assert_eq!(output, format!("{expected}{notice}"));
            assert!(output.strip_suffix(&notice).unwrap().len() <= limit);
            for label in ["", "stdout", "標準エラー"] {
                assert_eq!(
                    labeled_command_output_within(label, bytes, limit),
                    format!("{label}:\n{expected}{notice}")
                );
            }
        }
        assert_eq!(labeled_command_output_within("", b"", 0), ":\n");
        assert_eq!(command_output_within(&[0xff], 3), "�");
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
