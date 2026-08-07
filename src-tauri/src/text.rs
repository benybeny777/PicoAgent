// text.rs - 文字列を安全に切り詰めるための共通層。
//
// Rust の文字列は UTF-8 バイト列で、`&s[..120]` のようなバイト位置での切り出しは
// **その位置が文字の途中だと panic する**。日本語は 1 文字 3 バイトなので、固定長で
// 切ると高い確率で境界を外す。ログ・プレビュー・出力の打ち切りは全体を通して
// ここの関数を使うこと（直接のバイト添字による切り出しは `text::guard` が検出する）。

// 実装はOSS切り出し準備中のpicoagent-coreへ置き、既存のアプリ内パスを保つ。
pub use picoagent_core::text::{preview, truncate_bytes, Utf8Stream};

/// 固定長のバイト添字による文字列の切り出しが残っていないかを走査するガード。
#[cfg(test)]
mod guard {
    use crate::guard_scan;

    /// `[..120]` のように**数値リテラル**で切り出している箇所を探す。
    /// 変数添字（`&chunk[..n]` など、バイト列の読み込み量で切る用途）は対象外。
    fn fixed_byte_slice(line: &str) -> bool {
        let Some(head) = line.find("[..") else {
            return false;
        };
        let rest = &line[head + 3..];
        let Some(close) = rest.find(']') else {
            return false;
        };
        let index = rest[..close].trim();
        !index.is_empty() && index.chars().all(|c| c.is_ascii_digit() || c == '_')
    }

    #[test]
    fn 固定長のバイト添字で文字列を切らない() {
        let mut offenders = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "text.rs" {
                continue; // 定義元（このガード自身の説明文を含む）
            }
            for (number, line) in source.body().lines().enumerate() {
                if fixed_byte_slice(line) {
                    offenders.push(format!("{}:{}", source.path.display(), number + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "文字列は text::preview / text::truncate_bytes で切ってください（バイト添字は文字の途中で panic します）: {offenders:?}"
        );
    }

    /// ストリーミング受信は `text::Utf8Stream` を通す。`from_utf8_lossy` を直接使うと、
    /// チャンク境界で分断された文字が `�` になって確定してしまう。
    #[test]
    fn ストリーミング受信で直接lossy変換しない() {
        let mut offenders = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "text.rs" {
                continue; // 定義元
            }
            let body = source.body();
            if body.contains("bytes_stream()") && body.contains("from_utf8_lossy") {
                offenders.push(source.path.display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "ストリーミング受信は text::Utf8Stream を使ってください（チャンク境界で日本語が化けます）: {offenders:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{preview, truncate_bytes};

    #[test]
    fn 日本語を文字数で切り詰める() {
        assert_eq!(preview("こんにちは", 10), "こんにちは");
        assert_eq!(preview("こんにちは", 3), "こんに…");
        assert_eq!(preview("", 3), "");
    }

    #[test]
    fn バイト上限でも文字の途中で切らない() {
        // 「あ」は 3 バイト。上限 4 バイトなら 1 文字だけ残る（panic しない）。
        assert_eq!(truncate_bytes("あああ", 4), "あ");
        assert_eq!(truncate_bytes("あああ", 9), "あああ");
        assert_eq!(truncate_bytes("あああ", 2), "");
        assert_eq!(truncate_bytes("abc", 2), "ab");
    }

    #[test]
    fn チャンク境界で分断された日本語を化けさせない() {
        use super::Utf8Stream;
        let source = "こんにちは、今日はいい天気ですね。";
        let bytes = source.as_bytes();
        // 1 バイトずつ流す＝毎回どこかの文字の途中で切れる、最悪ケース。
        let mut stream = Utf8Stream::default();
        let mut out = String::new();
        for byte in bytes {
            out.push_str(&stream.push(&[*byte]));
        }
        assert_eq!(out, source);
        assert!(!out.contains('\u{FFFD}'));

        // 参考: 同じ切り方を from_utf8_lossy でやると壊れる（これが元の不具合）。
        let broken: String = bytes
            .iter()
            .map(|byte| String::from_utf8_lossy(&[*byte]).into_owned())
            .collect();
        assert!(
            broken.contains('\u{FFFD}'),
            "旧実装なら化ける入力であること"
        );
    }

    #[test]
    fn 壊れたバイト列は置換文字にして先へ進む() {
        use super::Utf8Stream;
        let mut stream = Utf8Stream::default();
        // 0xFF は UTF-8 として不正。ここで止まらず後続を返せること。
        let out = stream.push(&[b'a', 0xFF, b'b']);
        assert_eq!(out, "a\u{FFFD}b");
        // 末尾が文字の途中で終わったストリームは、続きが来るまで何も返さない。
        let mut partial = Utf8Stream::default();
        assert_eq!(partial.push(&[0xE3, 0x81]), "");
        assert_eq!(partial.push(&[0x82]), "あ");
    }

    #[test]
    fn 壊れたjsonのプレビューでpanicしない() {
        // 実際に起きていた不具合の再現: モデルが返した不正な引数 JSON は日本語を含み、
        // 120 バイト目が文字の途中に来ると `&raw[..120]` が panic していた。
        let raw = format!("{{\"path\": \"{}\"", "デスクトップの資料".repeat(20));
        assert!(
            !raw.is_char_boundary(120),
            "この入力は 120 バイト目が文字の途中"
        );
        let shown = preview(&raw, 120);
        assert!(shown.chars().count() <= 121);
        assert!(shown.ends_with('…'));
    }
}
