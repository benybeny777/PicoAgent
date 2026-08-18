// skills/glob_files/mod.rs - 許可ルート配下のファイルを glob パターンで探すスキル。
//
// 「そのファイルはどこにあるか」を名前の形だけで引く道具。内容を読む `doc_search` や
// `grep_search` と違い、走査結果はパスの一覧だけなので、目的のファイルを絞り込む
// 最初の一手として安い。既定は更新時刻の新しい順で、作業中のファイルが上に来る。
//
// 走査は上限（深さ・件数）付きで、シンボリックリンクは辿らない。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::permissions;

const MAX_DEPTH: usize = 12;
/// 走査を打ち切るファイル数（ホーム直下など巨大なルートでも暴走させない安全弁）。
const MAX_SCAN_FILES: usize = 20_000;
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;
/// 走査から除外するディレクトリ名（巨大・生成物）。隠しディレクトリも既定で除外する。
const SKIP_DIRS: [&str; 8] = [
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
];

/// 1階層分のワイルドカード一致（`*` は任意長、`?` は1文字。大文字小文字は区別しない）。
/// `grep_search` の内蔵走査でも同じ判定を使う（glob の解釈を2箇所に持たない）。
///
/// バックトラックせずに済むよう、「パターンを1文字ずつ進めながら、名前のどこまで
/// 一致し得るか」を真偽値の表で持ち回る（`*` が複数あっても指数時間にならない）。
pub fn match_name(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    let mut reachable = vec![false; name.len() + 1];
    reachable[0] = true;
    for pattern_char in pattern {
        let mut next = vec![false; name.len() + 1];
        if pattern_char == '*' {
            let mut seen = false;
            for index in 0..=name.len() {
                seen |= reachable[index];
                next[index] = seen;
            }
        } else {
            for index in 1..=name.len() {
                if reachable[index - 1]
                    && (pattern_char == '?' || pattern_char == name[index - 1])
                {
                    next[index] = true;
                }
            }
        }
        reachable = next;
    }
    reachable[name.len()]
}

/// パターン全体（`**` を含む複数階層）の一致。
fn match_segments(pattern: &[&str], parts: &[&str]) -> bool {
    match pattern.first() {
        None => parts.is_empty(),
        // `**` は 0 階層以上に一致する。
        Some(&"**") => (0..=parts.len()).any(|skip| {
            match_segments(
                pattern.get(1..).unwrap_or_default(),
                parts.get(skip..).unwrap_or_default(),
            )
        }),
        Some(segment) => {
            !parts.is_empty()
                && match_name(segment, parts[0])
                && match_segments(
                    pattern.get(1..).unwrap_or_default(),
                    parts.get(1..).unwrap_or_default(),
                )
        }
    }
}

fn matches(pattern: &str, relative: &Path) -> bool {
    let pattern: Vec<&str> = pattern
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    let parts: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    match_segments(&pattern, &parts)
}

struct Hit {
    relative: PathBuf,
    modified: SystemTime,
    size: u64,
}

fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// ルート配下を走査して、パターンに一致するファイルを集める。
/// 戻り値の bool は「上限で打ち切ったか」。
///
/// 上限は**見たファイル数**で数える（一致件数で数えると、一致が少ないまま巨大な
/// ツリーを最後まで歩き続けてしまう）。
fn collect(
    root: &Path,
    dir: &Path,
    pattern: &str,
    depth: usize,
    scanned: &mut usize,
    hits: &mut Vec<Hit>,
) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    if *scanned >= MAX_SCAN_FILES {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        // フォールバック許可: 読めないフォルダ（権限なし等）は結果から除くだけで走査は続ける。
        return false;
    };
    let mut truncated = false;
    for entry in entries.flatten() {
        if *scanned >= MAX_SCAN_FILES {
            return true;
        }
        let path = entry.path();
        // シンボリックリンク／ジャンクションは辿らない（許可ルート外へ出るのを防ぐ）。
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if meta.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            truncated |= collect(root, &path, pattern, depth + 1, scanned, hits);
            continue;
        }
        *scanned += 1;
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if matches(pattern, relative) {
            hits.push(Hit {
                relative: relative.to_path_buf(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                size: meta.len(),
            });
        }
    }
    truncated
}

pub struct GlobFilesTool;

#[async_trait]
impl Tool for GlobFilesTool {
    fn name(&self) -> &'static str {
        "glob_files"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["ファイルを探", "ファイルはどこ", "どこにある", "ファイル名", "拡張子", "一覧して", "glob"]
    }

    fn label(&self) -> &'static str {
        "ファイル名検索"
    }

    fn description(&self) -> String {
        "許可ルート配下のファイルを glob パターン（`**/*.rs`、`src/**/test_*.py` など）で探し、パスの一覧を返す。ファイルの場所を名前から特定したいときに使う。中身を探す場合は grep_search を使う。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "探すパターン。`**` は任意階層、`*` は任意文字列、`?` は1文字。例: `**/*.md`。"
                },
                "path": {
                    "type": "string",
                    "description": "探索の起点フォルダ（許可ルート配下）。省略時は許可ルート直下から探す。"
                },
                "limit": {
                    "type": "integer",
                    "description": "返す件数の上限（既定100、最大500）。"
                },
                "sort": {
                    "type": "string",
                    "enum": ["mtime", "path"],
                    "description": "並び順。mtime は更新の新しい順（既定）、path は名前順。"
                }
            },
            "required": ["pattern"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["プロジェクトの .rs ファイルを一覧して", "設定ファイルがどこにあるか探して"]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let pattern = match args.get("pattern").and_then(Value::as_str) {
            Some(pattern) if !pattern.trim().is_empty() => pattern.trim().to_string(),
            _ => return "エラー: pattern が指定されていません。".to_string(),
        };
        let base = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .unwrap_or(".");
        // 許可ルート内かつ保護パスでないことを共通ルールで検証する。
        let root = match permissions::resolve(cfg, base) {
            Ok(root) => root,
            Err(error) => return format!("エラー: {error}"),
        };
        if !root.exists() {
            return format!("エラー: フォルダがありません: {}", root.display());
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| (limit as usize).clamp(1, MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);
        let sort_by_path = args.get("sort").and_then(Value::as_str) == Some("path");

        let mut hits = Vec::new();
        let mut scanned = 0usize;
        let truncated = collect(
            root.as_path(),
            root.as_path(),
            &pattern,
            0,
            &mut scanned,
            &mut hits,
        );
        if hits.is_empty() {
            return format!(
                "一致するファイルはありませんでした（{} 配下 / パターン {}）。",
                root.display(),
                pattern
            );
        }
        if sort_by_path {
            hits.sort_by(|a, b| a.relative.cmp(&b.relative));
        } else {
            hits.sort_by_key(|hit| std::cmp::Reverse(hit.modified));
        }
        let total = hits.len();
        let mut out = format!("{} 配下の一致: {} 件", root.display(), total);
        if truncated {
            out.push_str(&format!("（走査上限 {MAX_SCAN_FILES} 件で打ち切ったため、これ以上ある可能性があります）"));
        }
        for hit in hits.iter().take(limit) {
            out.push_str(&format!(
                "\n{}\t{} バイト",
                hit.relative.to_string_lossy(),
                hit.size
            ));
        }
        if total > limit {
            out.push_str(&format!("\n…（{limit} 件で表示を打ち切り。limit で増やせます）"));
        }
        out
    }
}

inventory::submit! { ToolReg { make: || Box::new(GlobFilesTool) } }

#[cfg(test)]
mod tests {
    use super::{match_name, matches};
    use std::path::Path;

    #[test]
    fn ワイルドカードが1階層に一致する() {
        assert!(match_name("*.rs", "main.rs"));
        assert!(match_name("test_*.py", "test_a.py"));
        assert!(match_name("a?c", "abc"));
        assert!(!match_name("*.rs", "main.py"));
        assert!(!match_name("a?c", "ac"));
        // 大文字小文字は区別しない（Windows の実挙動に合わせる）。
        assert!(match_name("*.MD", "readme.md"));
    }

    /// `*` が複数あってもバックトラックで壊れないこと。
    #[test]
    fn 複数のワイルドカードを含むパターンに一致する() {
        assert!(match_name("*a*b*", "xxaybzz"));
        // b より後に a しか無い＝順序が違うので一致しない。
        assert!(!match_name("*a*b*", "xxbya"));
    }

    #[test]
    fn 任意階層は0階層以上に一致する() {
        assert!(matches("**/*.rs", Path::new("main.rs")));
        assert!(matches("**/*.rs", Path::new("src/tools/main.rs")));
        assert!(matches("src/**/test_*.py", Path::new("src/a/b/test_x.py")));
        assert!(!matches("src/**/*.py", Path::new("docs/a.py")));
        assert!(!matches("*.rs", Path::new("src/main.rs")));
    }

    #[test]
    fn 日本語のファイル名にも一致する() {
        assert!(matches("**/*.md", Path::new("資料/設計メモ.md")));
    }
}
