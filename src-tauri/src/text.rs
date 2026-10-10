// text.rs - 文字列を安全に切り詰めるための共通層と、スキル出力で共有する小さな表示整形。
//
// Rust の文字列は UTF-8 バイト列で、`&s[..120]` のようなバイト位置での切り出しは
// **その位置が文字の途中だと panic する**。日本語は 1 文字 3 バイトなので、固定長で
// 切ると高い確率で境界を外す。ログ・プレビュー・出力の打ち切りは全体を通して
// ここの関数を使うこと（直接のバイト添字による切り出しは `text::guard` が検出する）。

// UTF-8処理は非公開の内部共通crateであるpicoagent-coreへ置き、既存のアプリ内パスを保つ。
pub use picoagent_core::text::{command_output, preview, truncate_bytes, Utf8Stream};

/// 名前の文字種と、既存の入力・出力上限を保つ生成方針。
#[derive(Clone, Copy)]
pub enum SlugPolicy {
    AsciiOutput,
    AsciiInput,
    AsciiWordsInput,
    Unicode,
}

pub fn slug(text: &str, fallback: &str, max_chars: usize, policy: SlugPolicy) -> String {
    let limit_input = matches!(policy, SlugPolicy::AsciiInput | SlugPolicy::AsciiWordsInput);
    let mut out = String::new();
    for ch in text
        .chars()
        .take(if limit_input { max_chars } else { usize::MAX })
    {
        let accepted = if matches!(policy, SlugPolicy::Unicode) {
            ch.is_alphanumeric()
        } else {
            ch.is_ascii_alphanumeric()
        };
        if accepted {
            out.push(ch.to_ascii_lowercase());
        } else if (!matches!(policy, SlugPolicy::AsciiWordsInput)
            || ch.is_whitespace()
            || "-_.".contains(ch))
            && !out.is_empty()
            && !out.ends_with('-')
        {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        fallback.to_string()
    } else if limit_input {
        out
    } else {
        out.chars().take(max_chars).collect()
    }
}

fn escape_markup(text: &str, xml: bool, strip_controls: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str(if xml { "&apos;" } else { "&#39;" }),
            '\t' | '\n' | '\r' => out.push(ch),
            _ if !strip_controls || ch >= '\u{20}' => out.push(ch),
            _ => {}
        }
    }
    out
}

pub fn html_escape(text: &str) -> String {
    escape_markup(text, false, false)
}
/// XMLと同じ入力制限を持つ成果物HTMLでは制御文字も除く。
pub fn html_escape_clean(text: &str) -> String {
    escape_markup(text, false, true)
}
pub fn xml_escape(text: &str) -> String {
    escape_markup(text, true, false)
}
pub fn xml_escape_clean(text: &str) -> String {
    escape_markup(text, true, true)
}

/// 曜日の1文字表記（「月」〜「日」）。日付の横に添える表示と会話文脈で共通に使う。
pub fn weekday_ja(day: chrono::Weekday) -> &'static str {
    ["月", "火", "水", "木", "金", "土", "日"][day.num_days_from_monday() as usize]
}

/// 数値列を Unicode ブロック文字の簡易チャート（スパークライン）にする。
/// 全部同じ値なら中段の高さで横一直線にする（最下段だと「急落」に見える）。
pub fn sparkline(values: &[f64]) -> Result<String, String> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err("グラフの数値に有限でない値があります".to_string());
    }
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if max.partial_cmp(&min) != Some(std::cmp::Ordering::Greater) {
        return Ok(BARS[3].to_string().repeat(values.len()));
    }
    // 正負の最大値の差もoverflowさせないよう、同じ尺度へ正規化する。
    let difference = max - min;
    let scale = if difference.is_finite() {
        1.0
    } else {
        min.abs().max(max.abs())
    };
    let low = min / scale;
    let span = if difference.is_finite() {
        difference
    } else {
        max / scale - low
    };
    Ok(values
        .iter()
        .map(|&v| BARS[((((v / scale - low) / span) * 7.0).round() as usize).min(7)])
        .collect())
}

/// 固定長のバイト添字による文字列の切り出しが残っていないかを走査するガード。
#[cfg(test)]
mod guard {
    use crate::guard_scan;

    #[test]
    fn 小道具の定義は責務の持ち主以外へ複写しない() {
        let owners = [
            ("slug", "src-tauri/src/text.rs"),
            ("html_escape", "src-tauri/src/text.rs"),
            ("xml_escape", "src-tauri/src/text.rs"),
            ("sparkline", "src-tauri/src/text.rs"),
            ("direct_url", "src-tauri/src/tools/search_url.rs"),
            ("parse_days", "src-tauri/src/tools/datetime_words.rs"),
            ("number_before", "src-tauri/src/tools/datetime_words.rs"),
            (
                "ensure_executable",
                "crates/picoagent-core/src/permissions.rs",
            ),
            ("sha256_reader", "crates/picoagent-core/src/hash.rs"),
            ("sha256_file", "src-tauri/src/installer/fs.rs"),
            ("file_sha256", "xtask/src/util.rs"),
            // 比較側は用途固有の理由を持ち、ファイルSHA計算を独自に再実装しない。
            ("verify_sha256", ""),
        ];
        let mut offenders = Vec::new();
        for source in guard_scan::all_sources()
            .into_iter()
            .chain(guard_scan::xtask_sources())
        {
            for (name, owner) in owners {
                if source.body().contains(&format!("fn {name}("))
                    && source.path != std::path::Path::new(owner)
                {
                    offenders.push(format!("{}: {name}", source.path.display()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "共通小道具の写しがあります: {offenders:?}"
        );
    }

    #[test]
    fn hashと実行属性の手順は共通所有者と用途固有の処理だけに置く() {
        // 関数名を変えて写した計算も検知する。複合版ID・受信中hash・メモリhashは別責務。
        let exceptions = [
            (
                "crates/picoagent-core/src/hash.rs",
                "sha256_reader",
                "共通の読取ストリーム",
            ),
            (
                "xtask/src/assets.rs",
                "character_content_version",
                "パスと正規化manifestを含む複合版ID",
            ),
            (
                "src-tauri/src/installer/download.rs",
                "receive_fixed",
                "受信と同時に検証し再読取しない",
            ),
            (
                "src-tauri/src/commands.rs",
                "run_agent_and_persist",
                "CLI用のメモリ人格識別子",
            ),
        ];
        let definitions =
            regex::Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*[<(]")
                .unwrap();
        let mut violations = Vec::new();
        for source in guard_scan::all_sources()
            .into_iter()
            .chain(guard_scan::xtask_sources())
        {
            let body = source.body();
            let functions: Vec<_> = definitions.captures_iter(body).collect();
            for (index, function) in functions.iter().enumerate() {
                let start = function.get(0).unwrap().start();
                let end = functions
                    .get(index + 1)
                    .map_or(body.len(), |next| next.get(0).unwrap().start());
                let section = &body[start..end];
                let name = &function[1];
                if section.contains("Sha256::new()")
                    && !exceptions.iter().any(|(path, allowed, reason)| {
                        source.path == std::path::Path::new(path)
                            && name == *allowed
                            && !reason.is_empty()
                    })
                {
                    violations.push(format!("{}: {name}のSHA計算", source.path.display()));
                }
                if (section.contains(".set_mode(") || section.contains("set_permissions("))
                    && !(source.path
                        == std::path::Path::new("crates/picoagent-core/src/permissions.rs")
                        && name == "ensure_executable"
                        || source.path == std::path::Path::new("src-tauri/src/installer/fs.rs")
                            && name == "set_safe_permissions")
                {
                    // 展開側の0644/0755への正確な安全属性は、既存属性へ実行bitを足す責務とは異なる。
                    violations.push(format!("{}: {name}の属性変更", source.path.display()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "共通hash・実行属性の手順を複写しています: {violations:?}"
        );
    }

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

    /// スパークラインは `text::sparkline` を正本にする。天気・為替・市場指数へ同じ関数が
    /// 3つ複写され、横一直線の扱いだけが食い違っていた。
    #[test]
    fn スパークラインを各所で複写しない() {
        let offenders: Vec<String> = guard_scan::all_sources()
            .iter()
            .filter(|source| source.name != "text.rs" && source.body().contains("'▁', '▂'"))
            .map(|source| source.path.display().to_string())
            .collect();
        assert!(
            offenders.is_empty(),
            "スパークラインは crate::text::sparkline を使ってください: {offenders:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{preview, truncate_bytes};

    #[test]
    fn スパークラインは最小を最下段に最大を最上段に置く() {
        for (values, expected) in [
            (&[1.0, 2.0, 8.0][..], "▁▂█"),
            (&[5.0, 5.0][..], "▄▄"),
            (&[][..], ""),
            (&[-f64::MAX, 0.0, f64::MAX][..], "▁▅█"),
            (&[0.0, f64::MIN_POSITIVE][..], "▁█"),
        ] {
            assert_eq!(super::sparkline(values).unwrap(), expected);
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(super::sparkline(&[1.0, value]).is_err());
        }
    }

    #[test]
    fn 各生成物の名前方針と上限を一つの表で保持する() {
        use super::SlugPolicy::*;
        for (input, fallback, limit, policy, expected) in [
            (
                "AI Suite / 日本語",
                "genspark-suite",
                64,
                AsciiOutput,
                "ai-suite",
            ),
            (
                "日本語",
                "spark-workspace",
                64,
                AsciiOutput,
                "spark-workspace",
            ),
            ("ABC 日本語 DEF", "project", 40, AsciiInput, "abc-def"),
            ("日ABC DEF", "project", 4, AsciiInput, "abc"),
            ("日ABC DEF", "research-report", 4, AsciiWordsInput, "abc"),
            ("AB日本CD", "research-report", 48, AsciiWordsInput, "abcd"),
            ("動画 制作", "", usize::MAX, Unicode, "動画-制作"),
            ("///", "", usize::MAX, Unicode, ""),
            ("AB CDEF", "", 3, AsciiOutput, "ab-"),
        ] {
            assert_eq!(super::slug(input, fallback, limit, policy), expected);
        }
    }

    #[test]
    fn マークアップを一度だけescapeし形式固有の制御文字制約を保つ() {
        let input = "<&>\"'日本語\u{0}\n";
        assert_eq!(
            super::html_escape(input),
            "&lt;&amp;&gt;&quot;&#39;日本語\u{0}\n"
        );
        assert_eq!(
            super::html_escape_clean(input),
            "&lt;&amp;&gt;&quot;&#39;日本語\n"
        );
        assert_eq!(
            super::xml_escape_clean(input),
            "&lt;&amp;&gt;&quot;&apos;日本語\n"
        );
        assert_eq!(super::html_escape("&amp;"), "&amp;amp;");
        assert_eq!(super::xml_escape("A\u{0}B"), "A\u{0}B");
    }

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
