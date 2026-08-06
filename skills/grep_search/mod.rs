// skills/grep_search/mod.rs - 許可ルート配下のファイル内容を正規表現で検索するスキル。
//
// 「この関数はどこで使われているか」「この文字列を含むファイルはどれか」を引く道具。
// 語のスコアで抜粋を返す `doc_search` が調べもの向けなのに対し、こちらはコード作業向けに
// 行番号付きの一致行をそのまま返す（出力形式も content / files / count から選べる）。
//
// 検索本体は同梱 ripgrep（`cargo xtask setup tools`）→ PATH の順に解決して使う。
// ripgrep が無い環境では内蔵走査へ落ちるが、その場合は正規表現ではなく固定文字列一致に
// なるため、結果の先頭にその旨を明記する（黙って別物の検索をしない）。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::permissions;
use crate::text;

const DEFAULT_LIMIT: usize = 60;
const MAX_LIMIT: usize = 300;
const MAX_CONTEXT_LINES: u64 = 5;
const MAX_LINE_CHARS: usize = 400;
const TIMEOUT_SECS: u64 = 30;
/// 内蔵走査（ripgrep が無いとき）の安全弁。
const MAX_SCAN_FILES: usize = 5_000;
const MAX_SCAN_DEPTH: usize = 12;
const MAX_SCAN_FILE_BYTES: u64 = 2 * 1024 * 1024;
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

/// ripgrep の実行パスを解決する。優先順: 明示設定 → 同梱（setup tools）→ PATH。
fn ripgrep_path(cfg: &Config) -> String {
    crate::tools::resolve_cli_path(cfg, "PICOAGENT_RIPGREP_PATH", "ripgrep", "rg")
}

#[derive(Clone, Copy, PartialEq)]
enum OutputMode {
    Content,
    Files,
    Count,
}

impl OutputMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "" | "content" => Ok(Self::Content),
            "files" | "files_with_matches" => Ok(Self::Files),
            "count" => Ok(Self::Count),
            other => Err(format!(
                "output_mode は content / files / count のいずれかを指定してください: {other}"
            )),
        }
    }
}

struct Query {
    pattern: String,
    glob: String,
    ignore_case: bool,
    fixed: bool,
    context: u64,
    mode: OutputMode,
    limit: usize,
}

/// ripgrep で検索する。実行できない・rg 自体が失敗した場合は None を返し、内蔵走査へ落とす。
async fn search_with_ripgrep(cfg: &Config, root: &Path, query: &Query) -> Option<Vec<String>> {
    let mut command = crate::tools::process::command(ripgrep_path(cfg));
    command.args([
        "--line-number",
        "--with-filename",
        "--no-messages",
        // .gitignore の有無で結果が変わらないようにする（内蔵走査と対象を揃える）。
        "--no-ignore-vcs",
        "--max-filesize",
        "2M",
        "--max-depth",
        "12",
    ]);
    match query.mode {
        OutputMode::Content => {
            if query.context > 0 {
                command.arg("--context").arg(query.context.to_string());
            }
        }
        OutputMode::Files => {
            command.arg("--files-with-matches");
        }
        OutputMode::Count => {
            command.arg("--count-matches");
        }
    }
    if query.ignore_case {
        command.arg("--ignore-case");
    }
    if query.fixed {
        command.arg("--fixed-strings");
    }
    if !query.glob.is_empty() {
        command.arg("--glob").arg(&query.glob);
    }
    for dir in SKIP_DIRS {
        command.arg("--glob").arg(format!("!{dir}/"));
    }
    command.arg("--").arg(&query.pattern).arg(root);

    // フォールバック許可: ripgrep が無い・起動できない環境では内蔵走査へ落とす
    // （落ちたことは呼び出し側が結果へ明記する）。
    let output = crate::tools::process::output(command, TIMEOUT_SECS, "ファイル内容検索")
        .await
        .ok()?;
    // rg の終了コード: 0=一致あり / 1=一致なし / 2=エラー（エラー時は内蔵走査へ）。
    if output.status.code() == Some(2) {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| text::preview(line, MAX_LINE_CHARS))
            .collect(),
    )
}

/// 内蔵走査（ripgrep が無い環境向け）。固定文字列一致だけを行う。
fn search_builtin(root: &Path, query: &Query) -> Vec<String> {
    let needle = if query.ignore_case {
        query.pattern.to_lowercase()
    } else {
        query.pattern.clone()
    };
    let mut files = Vec::new();
    collect_files(root, &mut files, 0);
    let mut out = Vec::new();
    for path in files {
        if !query.glob.is_empty() {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            if !super::glob_files::match_name(query.glob.trim_start_matches("**/"), &name) {
                continue;
            }
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            // フォールバック許可: バイナリや権限なしのファイルは検索対象から外すだけ。
            continue;
        };
        let mut count = 0usize;
        let mut lines = Vec::new();
        for (number, line) in content.lines().enumerate() {
            let haystack = if query.ignore_case {
                line.to_lowercase()
            } else {
                line.to_string()
            };
            if haystack.contains(&needle) {
                count += 1;
                lines.push(format!(
                    "{}:{}:{}",
                    path.display(),
                    number + 1,
                    text::preview(line, MAX_LINE_CHARS)
                ));
            }
        }
        if count == 0 {
            continue;
        }
        match query.mode {
            OutputMode::Content => out.extend(lines),
            OutputMode::Files => out.push(path.display().to_string()),
            OutputMode::Count => out.push(format!("{}:{}", path.display(), count)),
        }
        if out.len() >= MAX_LIMIT {
            break;
        }
    }
    out
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_SCAN_DEPTH || files.len() >= MAX_SCAN_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        // フォールバック許可: 読めないフォルダは走査対象から外すだけ。
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_SCAN_FILES {
            return;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        // シンボリックリンク／ジャンクションは辿らない（許可ルート外へ出るのを防ぐ）。
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_files(&path, files, depth + 1);
        } else if meta.len() <= MAX_SCAN_FILE_BYTES {
            files.push(path);
        }
    }
}

pub struct GrepSearchTool;

#[async_trait]
impl Tool for GrepSearchTool {
    fn name(&self) -> &'static str {
        "grep_search"
    }

    fn label(&self) -> &'static str {
        "ファイル内容検索"
    }

    fn description(&self) -> String {
        "許可ルート配下のファイル内容を正規表現で検索し、行番号付きの一致行・一致ファイル・件数のいずれかを返す。関数や設定の使用箇所を探すときに使う。ファイル名から探す場合は glob_files を使う。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "検索する正規表現。記号をそのまま探したい場合は fixed=true を指定する。"
                },
                "path": {
                    "type": "string",
                    "description": "検索の起点フォルダまたはファイル（許可ルート配下）。省略時は許可ルート直下から検索する。"
                },
                "glob": {
                    "type": "string",
                    "description": "対象ファイルの絞り込み（例: `*.rs`、`**/*.md`）。"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files", "count"],
                    "description": "content は一致行（既定）、files は一致ファイルのパス、count はファイルごとの件数。"
                },
                "ignore_case": { "type": "boolean", "description": "大文字小文字を区別しない。" },
                "fixed": { "type": "boolean", "description": "pattern を正規表現ではなく固定文字列として扱う。" },
                "context_lines": {
                    "type": "integer",
                    "description": "content のとき、一致行の前後に付ける行数（0〜5）。"
                },
                "limit": { "type": "integer", "description": "返す行数の上限（既定60、最大300）。" }
            },
            "required": ["pattern"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["この関数を使っている箇所を探して", "TODO が残っているファイルを教えて"]
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_RIPGREP_PATH",
            "ripgrep 実行ファイルパス（空なら同梱→PATH、無ければ内蔵走査）",
            "text",
            "rg",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let pattern = match args.get("pattern").and_then(Value::as_str) {
            Some(pattern) if !pattern.trim().is_empty() => pattern.trim().to_string(),
            _ => return "エラー: pattern が指定されていません。".to_string(),
        };
        let mode = match OutputMode::parse(
            args.get("output_mode")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim(),
        ) {
            Ok(mode) => mode,
            Err(error) => return format!("エラー: {error}"),
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
            return format!("エラー: 検索対象がありません: {}", root.display());
        }
        let query = Query {
            pattern,
            glob: args
                .get("glob")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            ignore_case: args
                .get("ignore_case")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            fixed: args.get("fixed").and_then(Value::as_bool).unwrap_or(false),
            context: args
                .get("context_lines")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(MAX_CONTEXT_LINES),
            mode,
            limit: args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|limit| (limit as usize).clamp(1, MAX_LIMIT))
                .unwrap_or(DEFAULT_LIMIT),
        };

        let mut note = String::new();
        let lines = match search_with_ripgrep(cfg, root.as_path(), &query).await {
            Some(lines) => lines,
            None => {
                // 黙って別物の検索に差し替えない。正規表現でなくなったことを結果に明示する。
                note = "注意: ripgrep が見つからないため内蔵走査で検索しました（pattern は正規表現ではなく固定文字列として扱っています）。\n".to_string();
                search_builtin(root.as_path(), &query)
            }
        };

        if lines.is_empty() {
            return format!("{note}一致はありませんでした（{} 配下 / {}）。", root.display(), query.pattern);
        }
        let total = lines.len();
        let mut out = format!("{note}{} 配下の一致: {total} 行", root.display());
        for line in lines.iter().take(query.limit) {
            out.push('\n');
            out.push_str(line);
        }
        if total > query.limit {
            out.push_str(&format!(
                "\n…（{} 行で表示を打ち切り。limit で増やすか、output_mode=files で絞り込めます）",
                query.limit
            ));
        }
        out
    }
}

inventory::submit! { ToolReg { make: || Box::new(GrepSearchTool) } }

#[cfg(test)]
mod tests {
    use super::{search_builtin, OutputMode, Query};

    fn query(pattern: &str, mode: OutputMode) -> Query {
        Query {
            pattern: pattern.to_string(),
            glob: String::new(),
            ignore_case: false,
            fixed: true,
            context: 0,
            mode,
            limit: 60,
        }
    }

    #[test]
    fn 出力形式の指定を解釈する() {
        assert!(OutputMode::parse("").is_ok());
        assert!(OutputMode::parse("files_with_matches").is_ok());
        assert!(OutputMode::parse("なにか").is_err());
    }

    /// ripgrep が無い環境でも内蔵走査で一致行・ファイル・件数を返せること。
    #[test]
    fn 内蔵走査で一致行を返す() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("picoagent-grep-{nanos}"));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hello\nピコエージェント\nhello\n").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "なし\n").unwrap();

        let lines = search_builtin(&dir, &query("hello", OutputMode::Content));
        assert_eq!(lines.len(), 2, "lines: {lines:?}");
        assert!(lines[0].ends_with(":1:hello"), "lines: {lines:?}");

        let files = search_builtin(&dir, &query("hello", OutputMode::Files));
        assert_eq!(files.len(), 1);

        let counts = search_builtin(&dir, &query("hello", OutputMode::Count));
        assert!(counts[0].ends_with(":2"), "counts: {counts:?}");

        // 日本語（マルチバイト）でも行の切り出しで panic しない。
        let japanese = search_builtin(&dir, &query("ピコ", OutputMode::Content));
        assert_eq!(japanese.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }
}
