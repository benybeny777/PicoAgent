// doc_search/mod.rs - 許可ルート配下のローカル文書をキーワード検索するスキル。
// 埋め込みや外部送信を使わない軽量検索（RAG-lite）。本文とファイル名にスコアを付け、
// 一致箇所の前後を抜粋して返す。「あのフォルダの資料から探して」に応える。

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

pub struct DocSearchTool;

// 検索対象とみなすテキスト系拡張子。
const TEXT_EXTS: [&str; 16] = [
    "md", "txt", "csv", "tsv", "json", "yaml", "yml", "toml", "ini", "log", "rs", "py", "js", "ts",
    "html", "css",
];
// 走査上限（ホームディレクトリ全体などでも暴走しないための安全弁）。
const MAX_FILES: usize = 3000;
const MAX_DEPTH: usize = 8;
const MAX_FILE_BYTES: u64 = 512 * 1024;
// 走査から除外するディレクトリ名（巨大・生成物・隠し）。
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

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| TEXT_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// ルート配下のテキストファイルを上限付きで収集する（シンボリックリンクは辿らない）。
fn collect_files(root: &Path, files: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_DEPTH || files.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !should_skip_dir(&name) {
                collect_files(&path, files, depth + 1);
            }
        } else if meta.is_file() && meta.len() <= MAX_FILE_BYTES && is_text_file(&path) {
            files.push(path);
        }
    }
}

/// ripgrep の実行パスを解決する。優先順: 明示設定 → 同梱（setup tools）→ PATH。
fn ripgrep_path(cfg: &Config) -> String {
    crate::tools::resolve_cli_path(cfg, "PICOAGENT_RIPGREP_PATH", "ripgrep", "rg")
}

/// ripgrep で語に一致する候補ファイルを高速収集する（内蔵走査と同じ拡張子・除外・上限）。
/// rg が使えない、または実行に失敗した場合は None を返し、呼び出し側は内蔵走査へ
/// フォールバックする（グレースフルフォールバック）。
async fn rg_collect(cfg: &Config, roots: &[PathBuf], terms: &[String]) -> Option<Vec<PathBuf>> {
    let program = ripgrep_path(cfg);
    let mut found: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for term in terms {
        let mut command = crate::tools::process::command(&program);
        command.args([
            "--files-with-matches",
            "--ignore-case",
            "--fixed-strings",
            "--no-messages",
            // .gitignore 等を無視して内蔵走査と同じ対象を見る（バックエンドで結果が変わらないように）。
            "--no-ignore-vcs",
            "--max-filesize",
            "512K",
            "--max-depth",
            "8",
        ]);
        for ext in TEXT_EXTS {
            command.arg("--glob").arg(format!("*.{ext}"));
        }
        for dir in SKIP_DIRS {
            command.arg("--glob").arg(format!("!{dir}/"));
        }
        command.arg("--").arg(term);
        for root in roots {
            command.arg(root);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // 打ち切り時に子プロセスをツリーごと回収させるため、共通層を通す。
        // ripgrep が見つからない・失敗した場合は内蔵走査へ落ちるので None で返す。
        let output = crate::tools::process::output(command, 20, "ファイル横断検索（ripgrep）")
            .await
            .ok()?;
        // rg の終了コード: 0=一致あり / 1=一致なし / 2=エラー（エラー時は内蔵走査へ）。
        if output.status.code() == Some(2) {
            return None;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if found.len() >= MAX_FILES {
                return Some(found);
            }
            let path = PathBuf::from(line);
            if seen.insert(path.clone()) {
                found.push(path);
            }
        }
    }
    Some(found)
}

/// クエリ語の一致数からスコアを計算し、最初の一致行の抜粋を返す。
fn score_file(path: &Path, terms: &[String]) -> Option<(usize, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let lower = content.to_lowercase();
    let name_lower = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut score = 0usize;
    let mut first_pos: Option<usize> = None;
    for term in terms {
        let hits = lower.matches(term.as_str()).count().min(20);
        if hits > 0 {
            score += hits;
            let pos = lower.find(term.as_str()).unwrap_or(0);
            first_pos = Some(first_pos.map_or(pos, |current| current.min(pos)));
        }
        if name_lower.contains(term.as_str()) {
            score += 5;
        }
    }
    if score == 0 {
        return None;
    }
    // 最初の一致行の前後1行を抜粋（1行が長すぎる場合は240字で切る）。
    let snippet = first_pos
        .map(|pos| {
            let prefix = &content[..content
                .char_indices()
                .map(|(i, _)| i)
                .find(|&i| i >= pos)
                .unwrap_or(0)];
            let line_index = prefix.matches('\n').count();
            let lines: Vec<&str> = content.lines().collect();
            let from = line_index.saturating_sub(1);
            let to = (line_index + 1).min(lines.len().saturating_sub(1));
            let mut text = lines[from..=to].join("\n");
            if text.chars().count() > 240 {
                text = text.chars().take(240).collect::<String>() + "…";
            }
            text
        })
        .unwrap_or_default();
    Some((score, snippet))
}

#[async_trait]
impl Tool for DocSearchTool {
    fn name(&self) -> &'static str {
        "doc_search"
    }
    fn label(&self) -> &'static str {
        "ローカル文書検索"
    }
    fn description(&self) -> String {
        "許可ルート配下のローカル文書（md/txt/csv/コード等）をキーワード検索し、\
         一致したファイルと抜粋を返します。ripgrep（同梱→PATH）があれば高速検索、無ければ内蔵走査。\
         外部送信なし。パラメータ: query（必須）、max_results。"
            .to_string()
    }
    fn examples(&self) -> Vec<&'static str> {
        vec![
            "作業フォルダから会議メモを探して",
            "ローカルの資料で「予算」に触れているファイルは？",
            "プロジェクトのREADMEからセットアップ手順を探して",
        ]
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "検索キーワード（空白区切りで複数可）" },
                "max_results": { "type": "integer", "description": "返す件数（1〜10、既定5）", "default": 5 }
            },
            "required": ["query"]
        })
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_RIPGREP_PATH",
            "ripgrep 実行ファイルパス（高速検索用。空なら同梱→PATH、無ければ内蔵走査）",
            "text",
            "",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if query.is_empty() {
            return "エラー: query が指定されていません。".to_string();
        }
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| t.to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 10) as usize;

        // まず ripgrep（同梱→PATH）で候補を高速収集し、使えなければ内蔵走査へフォールバック。
        // rg は本文一致のみ拾うため、一致ゼロのときも内蔵走査でファイル名一致を補う。
        let roots: Vec<PathBuf> = crate::permissions::allowed_roots(cfg);
        let mut files = match rg_collect(cfg, &roots, &terms).await {
            Some(found) if !found.is_empty() => found,
            _ => Vec::new(),
        };
        if files.is_empty() {
            for root in &roots {
                collect_files(root, &mut files, 0);
            }
        }
        if files.is_empty() {
            return "許可ルート配下に検索対象のテキストファイルが見つかりませんでした。"
                .to_string();
        }

        let mut hits: Vec<(usize, PathBuf, String)> = files
            .iter()
            .filter(|path| !crate::permissions::is_protected(cfg, path))
            .filter_map(|path| {
                score_file(path, &terms).map(|(score, snippet)| (score, path.clone(), snippet))
            })
            .collect();
        hits.sort_by_key(|item| std::cmp::Reverse(item.0));
        hits.truncate(max_results);

        if hits.is_empty() {
            return format!(
                "「{query}」に一致するローカル文書は見つかりませんでした（{}ファイルを走査）。",
                files.len()
            );
        }
        let mut out = format!("「{query}」のローカル文書検索結果（{}件）:\n", hits.len());
        for (score, path, snippet) in hits {
            out.push_str(&format!(
                "\n### {} （一致度 {}）\n{}\n",
                path.display(),
                score,
                snippet
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::rg_collect;

    // rg がある環境だけで走る E2E（無ければ即成功で抜ける）。
    #[test]
    fn ripgrepで候補ファイルを収集できる() {
        if std::process::Command::new("rg")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("rg が無いためスキップ");
            return;
        }
        let dir = std::env::temp_dir().join(format!("picoagent-rg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("memo.md"), "会議の予算メモです。").unwrap();
        std::fs::write(dir.join("other.md"), "無関係の内容。").unwrap();
        std::fs::write(dir.join("skip.bin"), "予算").unwrap(); // 対象外拡張子は拾わない

        let cfg = crate::config::Config::new();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let roots = vec![dir.clone()];
        let terms = vec!["予算".to_string()];
        let found = runtime
            .block_on(rg_collect(&cfg, &roots, &terms))
            .expect("rg は実行できるはず");
        assert_eq!(found.len(), 1, "found: {found:?}");
        assert!(found[0].ends_with("memo.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

inventory::submit! { ToolReg { make: || Box::new(DocSearchTool) } }
