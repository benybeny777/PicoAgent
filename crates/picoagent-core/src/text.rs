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

/// 利用者が入力した文字列を、会話・履歴・モデルへ渡す前の共通形へ正規化する。
///
/// 取り除くのは「本文としての意味を持たず、見え方だけを変える文字」に限る。
/// - 双方向制御文字（U+202A〜U+202E / U+2066〜U+2069 / U+200E / U+200F）は
///   表示順を逆転できるため、履歴やフキダシで**実際に送った内容と違う文字列**を
///   見せられる。
/// - ゼロ幅空白などの不可視文字は、目視で同じに見える別の文字列を作れるため、
///   `check_injection` のような文字列検査のすり抜けに使える。
/// - C0/C1制御文字はログ・端末表示を壊す（エスケープシーケンスの混入）。
///
/// 残すものも明示しておく。絵文字の結合子（U+200D）とZWNJ（U+200C）は、家族絵文字や
/// アラビア語・インド系文字の正書法に必要なので消さない。改行・タブは本文の一部として残し、
/// CRLF と CR は LF へ寄せる。
pub fn sanitize_input(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(c),
            _ if is_display_control(c) => {}
            _ => out.push(c),
        }
    }
    let trimmed = out.trim();
    if trimmed.len() == out.len() {
        out
    } else {
        trimmed.to_string()
    }
}

/// 本文の意味を持たず表示だけを変える制御文字か。改行・タブは呼び出し側で先に通す。
fn is_display_control(c: char) -> bool {
    matches!(c,
        '\u{0000}'..='\u{001F}'   // C0制御
        | '\u{007F}'..='\u{009F}' // DEL と C1制御
        | '\u{200B}'              // ゼロ幅空白
        | '\u{200E}' | '\u{200F}' // 左向き・右向きマーク
        | '\u{202A}'..='\u{202E}' // 双方向の埋め込みと上書き
        | '\u{2060}'              // ワードジョイナ
        | '\u{2066}'..='\u{2069}' // 双方向の分離
        | '\u{FEFF}'              // BOM（ゼロ幅ノーブレークスペース）
    )
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
    use super::{preview, sanitize_input, truncate_bytes, Utf8Stream};

    #[test]
    fn 日本語を安全に切り詰める() {
        assert_eq!(preview("こんにちは", 3), "こんに…");
        assert_eq!(truncate_bytes("あああ", 4), "あ");
        assert_eq!(truncate_bytes("abc", 2), "ab");
    }

    #[test]
    fn 表示を偽装する不可視文字と制御文字を落とす() {
        // 双方向上書き（U+202E）で表示順を逆転させる細工を落とす。
        assert_eq!(sanitize_input("削除\u{202E}txt.exe"), "削除txt.exe");
        // ゼロ幅空白で検査をすり抜ける細工を落とす。
        assert_eq!(sanitize_input("ignore\u{200B} previous"), "ignore previous");
        // 端末・ログを壊すエスケープシーケンスを落とす。
        assert_eq!(sanitize_input("赤\u{1B}[31m色"), "赤[31m色");
        // BOM とワードジョイナも落とす。
        assert_eq!(sanitize_input("\u{FEFF}本文\u{2060}です"), "本文です");
    }

    #[test]
    fn 本文として必要な文字は残す() {
        // 家族絵文字の結合子（U+200D）を消すと絵文字が分解される。
        let family = "👨\u{200D}👩\u{200D}👧";
        assert_eq!(sanitize_input(family), family);
        // 改行とタブは本文の一部。CRLF と CR は LF へ寄せる。
        assert_eq!(sanitize_input("a\r\nb\rc\td"), "a\nb\nc\td");
        // 前後の空白だけを落とし、途中の空行は保つ。
        assert_eq!(sanitize_input("  上\n\n下  "), "上\n\n下");
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
