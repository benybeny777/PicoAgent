// text.rs - 文字列を安全に切り詰めるための共通層。
//
// Rust の文字列は UTF-8 バイト列で、`&s[..120]` のようなバイト位置での切り出しは
// **その位置が文字の途中だと panic する**。日本語は 1 文字 3 バイトなので、固定長で
// 切ると高い確率で境界を外す。ログ・プレビュー・出力の打ち切りは全体を通して
// ここの関数を使うこと（直接のバイト添字による切り出しは `text::guard` が検出する）。

/// 表示用に先頭 `max_chars` 文字までへ切り詰める。切り詰めたときだけ `…` を付ける。
///
/// 文字数で数えるので、日本語でも見た目の長さが予想どおりになる。
pub fn preview(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if out.chars().count() < text.chars().count() {
        out.push('…');
    }
    out
}

/// バイト数の上限で切り詰める（文字の途中では切らない）。
///
/// 保存サイズやプロトコル上の上限のように「バイト数」で制限したい場合に使う。
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

/// 固定長のバイト添字による文字列の切り出しが残っていないかを走査するガード。
#[cfg(test)]
mod guard {
    fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

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
        let mut files = Vec::new();
        rs_files(std::path::Path::new("src"), &mut files);
        assert!(files.len() > 20, "ソース走査に失敗しています");

        let mut offenders = Vec::new();
        for file in files {
            if file.file_name().is_some_and(|name| name == "text.rs") {
                continue; // 定義元（このガード自身の説明文を含む）
            }
            // フォールバック許可: 読めないファイルは走査対象外（存在しない想定）。
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (number, line) in source.lines().enumerate() {
                if fixed_byte_slice(line) {
                    offenders.push(format!("{}:{}", file.display(), number + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "文字列は text::preview / text::truncate_bytes で切ってください（バイト添字は文字の途中で panic します）: {offenders:?}"
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
