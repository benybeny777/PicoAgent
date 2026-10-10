// skills/grep_search/mod.rs - 許可ルート配下のファイル内容を正規表現で検索するスキル。
//
// 「この関数はどこで使われているか」「この文字列を含むファイルはどれか」を引く道具。
// 語のスコアで抜粋を返す `doc_search` が調べもの向けなのに対し、こちらはコード作業向けに
// 行番号付きの一致行をそのまま返す（出力形式も content / files / count から選べる）。
//
// 検索本体は明示設定または同梱ripgrepを使い、欠損・失敗は理由を返す。

use std::path::Path;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::permissions;
use crate::text;

pub struct GrepSearchTool;

const DEFAULT_LIMIT: usize = 60;
const MAX_LIMIT: usize = 300;
const MAX_CONTEXT_LINES: u64 = 5;
const MAX_LINE_CHARS: usize = 400;
const TIMEOUT_SECS: u64 = 30;
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

/// ripgrep の実行パスを解決する。優先順: 明示設定 → 同梱（setup tools）。
fn ripgrep_path(cfg: &Config) -> Result<std::path::PathBuf, String> {
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

async fn search_with_ripgrep(
    cfg: &Config,
    root: &Path,
    query: &Query,
) -> Result<Vec<String>, String> {
    let mut command = crate::tools::process::command(ripgrep_path(cfg)?);
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

    let output = crate::tools::process::output(command, TIMEOUT_SECS, "ファイル内容検索").await?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(format!(
            "ripgrepで検索できません: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| text::preview(line, MAX_LINE_CHARS))
        .collect())
}
#[async_trait]
impl Tool for GrepSearchTool {
    fn name(&self) -> &'static str {
        "grep_search"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "使っている箇所",
            "使われている",
            "呼んでいる",
            "含むファイル",
            "中身を検索",
            "文字列を検索",
            "grep",
            "todo",
        ]
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
        vec![
            "この関数を使っている箇所を探して",
            "TODO が残っているファイルを教えて",
        ]
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_RIPGREP_PATH",
            "ripgrep 実行ファイルパス（空なら明示設定または同梱）",
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

        let lines = crate::tool_try!(search_with_ripgrep(cfg, root.as_path(), &query).await);
        if lines.is_empty() {
            return format!(
                "一致はありませんでした（{} 配下 / {}）。",
                root.display(),
                query.pattern
            );
        }
        let total = lines.len();
        let mut out = format!("{} 配下の一致: {total} 行", root.display());
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
    use super::OutputMode;

    #[tokio::test]
    async fn 正規表現と出力形式を維持し欠損や検索失敗で別検索へ進まない() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nピコエージェント\nhello\n").unwrap();
        let cfg = crate::config::Config::new();
        cfg.set(
            "PICOAGENT_RIPGREP_PATH",
            &dir.path().join("missing-rg").to_string_lossy(),
        );
        let mut query = super::Query {
            pattern: "hello".into(),
            glob: String::new(),
            ignore_case: false,
            fixed: true,
            context: 0,
            mode: OutputMode::Content,
            limit: 60,
        };
        assert!(super::search_with_ripgrep(&cfg, dir.path(), &query)
            .await
            .unwrap_err()
            .contains("PICOAGENT_RIPGREP_PATH"));
        let Some(rg) = crate::tools::process::resolve_executable("rg") else {
            return;
        };
        // 実機のCLIを使う場合も、テストでは明示パスとして指定する。
        cfg.set("PICOAGENT_RIPGREP_PATH", &rg.to_string_lossy());
        for (mode, count) in [
            (OutputMode::Content, 2),
            (OutputMode::Files, 1),
            (OutputMode::Count, 1),
        ] {
            query.mode = mode;
            assert_eq!(
                super::search_with_ripgrep(&cfg, dir.path(), &query)
                    .await
                    .unwrap()
                    .len(),
                count
            );
        }
        query.fixed = false;
        query.mode = OutputMode::Content;
        query.pattern = "^ピコ".into();
        assert_eq!(
            super::search_with_ripgrep(&cfg, dir.path(), &query)
                .await
                .unwrap()
                .len(),
            1
        );
        query.pattern = "[".into();
        assert!(super::search_with_ripgrep(&cfg, dir.path(), &query)
            .await
            .is_err());
    }

    #[test]
    fn 出力形式の指定を解釈する() {
        assert!(OutputMode::parse("").is_ok());
        assert!(OutputMode::parse("files_with_matches").is_ok());
        assert!(OutputMode::parse("なにか").is_err());
    }
}
