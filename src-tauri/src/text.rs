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

    /// `[..120]` `[0..2]` `[4..]` のように**数値リテラル**で切り出している箇所を探す。
    /// 変数添字（`&chunk[..n]` など、バイト列の読み込み量で切る用途）は対象外。
    ///
    /// 以前は `[..N]`（先頭からの切り出し）しか見ておらず、`&hex[0..2]` のような
    /// 途中区間の切り出しを素通りさせていた。実際に `productivity_tools` の
    /// 色変換が「バイト長6の日本語2文字」で panic する状態で残っていた。
    fn fixed_byte_slice(line: &str) -> bool {
        fn numeric_literal(part: &str) -> bool {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_digit() || c == '_')
        }
        let bytes = line.as_bytes();
        for (open, _) in line.match_indices('[') {
            let Some(length) = line[open + 1..].find(']') else {
                continue;
            };
            let inner = line[open + 1..open + 1 + length].trim();
            let Some((start, end)) = inner.split_once("..") else {
                continue;
            };
            let end = end.strip_prefix('=').unwrap_or(end).trim();
            let start = start.trim();
            // 両側とも空（`[..]`）や、片側でも変数・式なら対象外。
            if !(numeric_literal(start) || start.is_empty())
                || !(numeric_literal(end) || end.is_empty())
                || (start.is_empty() && end.is_empty())
            {
                continue;
            }
            // 配列リテラルの型注釈（`[u8; 4]`）などと違い、直前が識別子・`)`・`]` の
            // ときだけ「何かを添字で切っている」と判断する。
            let before = bytes[..open]
                .iter()
                .rposition(|byte| !byte.is_ascii_whitespace());
            let Some(before) = before.map(|index| bytes[index] as char) else {
                continue;
            };
            if before.is_alphanumeric() || matches!(before, '_' | ')' | ']' | '"' | '#') {
                return true;
            }
        }
        false
    }

    #[test]
    fn 固定長のバイト添字で文字列を切らない() {
        let mut offenders = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "text.rs" {
                continue; // 定義元（このガード自身の説明文を含む）
            }
            for (number, line) in source.body().lines().enumerate() {
                // コメント行の説明文（このガード自身の例示を含む）は対象外。
                if line.trim_start().starts_with("//") {
                    continue;
                }
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

    #[test]
    fn 固定範囲の検出器は代表的な表記を見落とさない() {
        for source in [
            "&text[..120]",
            "&text[0..2]",
            "&text[4..]",
            "&\"日本語\"[0..3]",
            "&r#\"日本語\"#[0..3]",
            "method()[1..=2].to_vec()",
        ] {
            assert!(fixed_byte_slice(source), "検出できません: {source}");
        }
        for source in [
            "let bytes = [0_u8; 4];",
            "&chunk[..count]",
            "&items[start..]",
        ] {
            assert!(!fixed_byte_slice(source), "誤検出しました: {source}");
        }
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
