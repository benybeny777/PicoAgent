// research_report/mod.rs - Web調査を束ね、出典付きMarkdownレポートを生成するスキル。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Local;
use scraper::{Html, Selector};
use serde_json::{json, Value};
use url::Url;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const SEARCH_URL: &str = "https://html.duckduckgo.com/html/";
const MAX_TOPIC_CHARS: usize = 160;
const MAX_QUESTIONS: usize = 6;
const MAX_RESULTS_PER_QUERY: usize = 5;
const MAX_REPORT_BYTES: usize = 96 * 1024;
const MAX_SOURCES_CSV_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
    query: String,
}

struct SavedArtifacts {
    report: PathBuf,
    brief: Option<PathBuf>,
    slides: Option<PathBuf>,
    sources: Option<PathBuf>,
}

pub struct ResearchReportTool;

fn home() -> String {
    dirs::home_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn workspace_root(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.get("PICOAGENT_WORKSPACE_ROOT", &home()))
}

fn normalize(base: &Path, input: &str) -> PathBuf {
    let joined = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        base.join(input)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => {
                out.pop();
            }
            CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn bounded_path(base: &Path, path: &str) -> Result<PathBuf, String> {
    let target = normalize(base, path);
    if target == base || target.starts_with(base) {
        Ok(target)
    } else {
        Err(format!(
            "許可ルート（{}）の外には保存できません: {}",
            base.display(),
            target.display()
        ))
    }
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars().take(48) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || "-_.".contains(ch) {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "research-report".to_string()
    } else {
        trimmed.to_string()
    }
}

fn direct_url(href: &str) -> String {
    let absolute = if href.starts_with("//") {
        format!("https:{}", href)
    } else {
        href.to_string()
    };
    if let Ok(parsed) = Url::parse(&absolute) {
        if let Some((_, destination)) = parsed.query_pairs().find(|(key, _)| key == "uddg") {
            return destination.into_owned();
        }
    }
    absolute
}

fn text_from_node<'a>(node: scraper::ElementRef<'a>) -> String {
    node.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_results(
    html: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    let document = Html::parse_document(html);
    let result_selector = Selector::parse(".result")
        .map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;
    let title_selector = Selector::parse(".result__a")
        .map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;
    let snippet_selector = Selector::parse(".result__snippet")
        .map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;

    let mut output = Vec::new();
    for result in document.select(&result_selector) {
        let Some(title_node) = result.select(&title_selector).next() else {
            continue;
        };
        let Some(href) = title_node.value().attr("href") else {
            continue;
        };
        let title = text_from_node(title_node);
        if title.is_empty() {
            continue;
        }
        let snippet = result
            .select(&snippet_selector)
            .next()
            .map(text_from_node)
            .unwrap_or_default();
        output.push(SearchResult {
            title,
            url: direct_url(href),
            snippet,
            query: query.to_string(),
        });
        if output.len() >= max_results {
            break;
        }
    }
    if output.is_empty() {
        Err("検索結果を取得できませんでした。検索語を変えて再試行してください。".to_string())
    } else {
        Ok(output)
    }
}

fn trim_to_bytes(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n\n…（レポートサイズ上限で打ち切り）\n");
}

fn question_list(topic: &str, args: &Value) -> Vec<String> {
    let mut out: Vec<String> = args
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .take(MAX_QUESTIONS)
        .map(ToString::to_string)
        .collect();
    if out.is_empty() {
        out = vec![
            format!("{topic} 概要"),
            format!("{topic} 最新動向"),
            format!("{topic} 主要プレイヤー 比較"),
            format!("{topic} 課題 リスク"),
        ];
    }
    out
}

async fn search(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    let response = client
        .get(SEARCH_URL)
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|error| format!("Web検索への接続に失敗しました: {}", error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Web検索が HTTP {} を返しました。時間を置いて再試行してください。",
            status
        ));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("Web検索結果の読み取りに失敗しました: {}", error))?;
    extract_results(&body, query, max_results)
}

fn markdown_link(title: &str, url: &str) -> String {
    format!("[{}]({})", title.replace('[', "［").replace(']', "］"), url)
}

fn csv_cell(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('"', "\"\"")
            .replace('\r', " ")
            .replace('\n', " ")
    )
}

fn build_report(
    topic: &str,
    questions: &[String],
    results: &[SearchResult],
    failures: &[String],
) -> String {
    let generated_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut out = format!("# 調査レポート: {topic}\n\n生成日時: {generated_at}\n\n");
    out.push_str("## 要約\n\n");
    if results.is_empty() {
        out.push_str("- 検索結果を取得できなかったため、要約は作成できませんでした。\n");
    } else {
        for result in results.iter().take(5) {
            let snippet = if result.snippet.is_empty() {
                "概要なし"
            } else {
                result.snippet.as_str()
            };
            out.push_str(&format!(
                "- {}: {}\n",
                markdown_link(&result.title, &result.url),
                snippet
            ));
        }
    }

    out.push_str("\n## 調査観点別メモ\n\n");
    for question in questions {
        out.push_str(&format!("### {question}\n\n"));
        let mut count = 0;
        for result in results.iter().filter(|result| result.query == *question) {
            count += 1;
            let snippet = if result.snippet.is_empty() {
                "概要なし"
            } else {
                result.snippet.as_str()
            };
            out.push_str(&format!(
                "- {}: {}\n",
                markdown_link(&result.title, &result.url),
                snippet
            ));
        }
        if count == 0 {
            out.push_str("- この観点では検索結果を取得できませんでした。\n");
        }
        out.push('\n');
    }

    out.push_str("## 比較・整理テーブル\n\n");
    out.push_str("| 観点 | 出典 | メモ |\n|---|---|---|\n");
    for result in results.iter().take(24) {
        let snippet = if result.snippet.is_empty() {
            "概要なし".to_string()
        } else {
            result.snippet.chars().take(140).collect()
        };
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            result.query.replace('|', "｜"),
            markdown_link(&result.title, &result.url).replace('|', "｜"),
            snippet.replace('|', "｜")
        ));
    }

    out.push_str("\n## 次に深掘りするとよいこと\n\n");
    out.push_str("- 数値・価格・法規制など高精度が必要な項目は、一次情報を開いて確認する。\n");
    out.push_str("- 競合比較や意思決定に使う場合は、評価軸と重みを決めて表を再整理する。\n");
    out.push_str("- スライド化する場合は、要約・比較表・リスク・推奨アクションの4部構成にする。\n");

    out.push_str("\n## 取得失敗・注意点\n\n");
    if failures.is_empty() {
        out.push_str("- 検索取得の失敗はありません。\n");
    } else {
        for failure in failures {
            out.push_str(&format!("- {failure}\n"));
        }
    }
    out.push_str("- このレポートは検索結果のタイトルと概要から作った下書きです。重要な判断ではリンク先の一次情報を確認してください。\n");

    out.push_str("\n## 出典一覧\n\n");
    let mut seen = HashSet::new();
    for result in results {
        if seen.insert(result.url.clone()) {
            out.push_str(&format!(
                "- {} - {}\n",
                markdown_link(&result.title, &result.url),
                result.query
            ));
        }
    }
    trim_to_bytes(&mut out, MAX_REPORT_BYTES);
    out
}

fn build_slides_outline(topic: &str, questions: &[String], results: &[SearchResult]) -> String {
    let mut out = format!("# スライド下書き: {topic}\n\n");
    out.push_str("## 1. タイトル\n\n");
    out.push_str(&format!(
        "- {topic}\n- 調査サマリー / 生成日時: {}\n\n",
        Local::now().format("%Y-%m-%d")
    ));
    out.push_str("## 2. エグゼクティブサマリー\n\n");
    for result in results.iter().take(3) {
        out.push_str(&format!("- {}: {}\n", result.title, result.snippet));
    }
    out.push_str("\n## 3. 調査観点\n\n");
    for question in questions {
        out.push_str(&format!("- {question}\n"));
    }
    out.push_str("\n## 4. 主要な発見\n\n");
    for result in results.iter().take(6) {
        out.push_str(&format!("- {} ({})\n", result.title, result.url));
    }
    out.push_str("\n## 5. 比較表\n\n");
    out.push_str("| 観点 | 出典 | メモ |\n|---|---|---|\n");
    for result in results.iter().take(8) {
        let snippet: String = result.snippet.chars().take(90).collect();
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            result.query.replace('|', "｜"),
            result.title.replace('|', "｜"),
            snippet.replace('|', "｜")
        ));
    }
    out.push_str("\n## 6. リスク・未確認事項\n\n");
    out.push_str("- 検索結果概要ベースのため、重要な数値や主張は一次情報で再確認する。\n");
    out.push_str("- 古い記事や広告ページが混ざる可能性があるため、日付と発行元を確認する。\n");
    out.push_str("\n## 7. 推奨アクション\n\n");
    out.push_str("- 意思決定に使う評価軸を決める。\n- 一次情報を追加確認する。\n- 必要ならこの下書きを PowerPoint / Google Slides 用に整える。\n");
    out
}

fn build_brief(topic: &str, questions: &[String], results: &[SearchResult]) -> String {
    let mut out = format!("# 1ページ要約: {topic}\n\n");
    out.push_str(&format!(
        "- 生成日: {}\n- 調査観点: {}件\n- 参照候補: {}件\n\n",
        Local::now().format("%Y-%m-%d"),
        questions.len(),
        results.len()
    ));
    out.push_str("## 先に読む要点\n\n");
    for result in results.iter().take(5) {
        out.push_str(&format!("- {}: {}\n", result.title, result.snippet));
    }
    out.push_str("\n## 判断に使う観点\n\n");
    for question in questions.iter().take(6) {
        out.push_str(&format!("- {question}\n"));
    }
    out.push_str("\n## 追加確認が必要なこと\n\n");
    out.push_str("- 数値、日付、価格、規約はリンク先の一次情報で確認する。\n");
    out.push_str("- 検索結果だけでは判断しにくい比較軸は、公式資料や導入事例で補う。\n");
    out.push_str("- 古い記事や広告ページが混ざる可能性があるため、発行元と更新日を確認する。\n");
    out.push_str("\n## 代表的な出典候補\n\n");
    let mut seen = HashSet::new();
    for result in results
        .iter()
        .filter(|result| seen.insert(result.url.clone()))
        .take(5)
    {
        out.push_str(&format!(
            "- {} - {}\n",
            markdown_link(&result.title, &result.url),
            result.query
        ));
    }
    trim_to_bytes(&mut out, MAX_REPORT_BYTES / 4);
    out
}

fn build_sources_csv(results: &[SearchResult]) -> String {
    let mut out = String::from("query,title,url,snippet\n");
    let mut seen = HashSet::new();
    for result in results {
        if !seen.insert(result.url.clone()) {
            continue;
        }
        out.push_str(&format!(
            "{},{},{},{}\n",
            csv_cell(&result.query),
            csv_cell(&result.title),
            csv_cell(&result.url),
            csv_cell(&result.snippet)
        ));
        if out.len() > MAX_SOURCES_CSV_BYTES {
            out.push_str("\"truncated\",\"truncated\",\"\",\"CSV size limit reached\"\n");
            break;
        }
    }
    out
}

fn sibling_path(path: &Path, suffix: &str, extension: &str) -> Result<PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "保存先ファイル名を解釈できません。".to_string())?;
    Ok(path.with_file_name(format!("{stem}{suffix}.{extension}")))
}

fn write_text(path: &Path, content: &str, what: &str) -> Result<(), String> {
    std::fs::write(path, content).map_err(|error| format!("{what} の保存に失敗しました: {}", error))
}

fn save_report(
    cfg: &Config,
    topic: &str,
    args: &Value,
    report: &str,
    questions: &[String],
    results: &[SearchResult],
) -> Result<SavedArtifacts, String> {
    let base = workspace_root(cfg);
    let default_path = format!(
        "reports/{}-{}.md",
        Local::now().format("%Y%m%d-%H%M%S"),
        slug(topic)
    );
    let path = args
        .get("save_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&default_path);
    let target = bounded_path(&base, path)?;
    if target.extension().and_then(|ext| ext.to_str()) != Some("md") {
        return Err("save_path は .md ファイルにしてください。".to_string());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("保存先フォルダを作成できません: {}", error))?;
    }
    write_text(&target, report, "レポート")?;
    let bundle = args
        .get("artifact_bundle")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !bundle {
        return Ok(SavedArtifacts {
            report: target,
            brief: None,
            slides: None,
            sources: None,
        });
    }
    let brief_path = sibling_path(&target, ".brief", "md")?;
    let slides_path = sibling_path(&target, ".slides", "md")?;
    let sources_path = sibling_path(&target, ".sources", "csv")?;
    write_text(
        &brief_path,
        &build_brief(topic, questions, results),
        "1ページ要約",
    )?;
    write_text(
        &slides_path,
        &build_slides_outline(topic, questions, results),
        "スライド下書き",
    )?;
    write_text(&sources_path, &build_sources_csv(results), "出典CSV")?;
    Ok(SavedArtifacts {
        report: target,
        brief: Some(brief_path),
        slides: Some(slides_path),
        sources: Some(sources_path),
    })
}

fn saved_artifacts_text(artifacts: &SavedArtifacts) -> String {
    let mut out = format!("保存先:\n- レポート: {}", artifacts.report.display());
    if let Some(path) = &artifacts.brief {
        out.push_str(&format!("\n- 1ページ要約: {}", path.display()));
    }
    if let Some(path) = &artifacts.slides {
        out.push_str(&format!("\n- スライド下書き: {}", path.display()));
    }
    if let Some(path) = &artifacts.sources {
        out.push_str(&format!("\n- 出典CSV: {}", path.display()));
    }
    out
}

#[async_trait]
impl Tool for ResearchReportTool {
    fn name(&self) -> &'static str {
        "research_report"
    }

    fn label(&self) -> &'static str {
        "調査レポート生成"
    }

    fn description(&self) -> String {
        "複数のWeb検索を束ねて、出典付きMarkdown調査レポートを作る。市場調査、競合比較、技術調査、スライド下書きの前段に使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string", "description": "調査テーマ。" },
                "questions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "調査観点。省略時は概要・最新動向・主要プレイヤー・課題を自動生成。最大6件。"
                },
                "max_results_per_query": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS_PER_QUERY,
                    "description": "各観点で取得する検索結果数。省略時3件。"
                },
                "save": {
                    "type": "boolean",
                    "description": "true の場合、PICOAGENT_WORKSPACE_ROOT 配下へ Markdown を保存する。既定ではスライド下書きと出典CSVも同時保存する。"
                },
                "artifact_bundle": {
                    "type": "boolean",
                    "description": "保存時に .slides.md と .sources.csv も生成する。既定 true。"
                },
                "save_path": {
                    "type": "string",
                    "description": "保存先の .md パス。許可ルート配下の相対/絶対パスのみ。"
                }
            },
            "required": ["topic"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_RESEARCH_TIMEOUT_SECS",
                "調査レポート検索待ち時間（秒、3〜60）",
                "text",
                "15",
            ),
            Field::new(
                "PICOAGENT_WORKSPACE_ROOT",
                "調査レポート保存の許可ルート",
                "text",
                &home(),
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let topic = match args.get("topic").and_then(Value::as_str).map(str::trim) {
            Some(topic) if !topic.is_empty() => topic,
            _ => return "エラー: topic が指定されていません。".to_string(),
        };
        if topic.chars().count() > MAX_TOPIC_CHARS {
            return format!(
                "エラー: topic は {} 文字以内で指定してください。",
                MAX_TOPIC_CHARS
            );
        }
        let max_results = args
            .get("max_results_per_query")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (1..=MAX_RESULTS_PER_QUERY).contains(value))
            .unwrap_or(3);
        let timeout = cfg
            .get("PICOAGENT_RESEARCH_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let questions = question_list(topic, &args);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Research report tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: 調査レポートの準備に失敗しました: {}", error),
        };

        let mut results = Vec::new();
        let mut failures = Vec::new();
        for question in &questions {
            match search(&client, question, max_results).await {
                Ok(mut items) => results.append(&mut items),
                Err(error) => failures.push(format!("{question}: {error}")),
            }
        }
        let report = build_report(topic, &questions, &results, &failures);
        if args.get("save").and_then(Value::as_bool).unwrap_or(false)
            || args
                .get("save_path")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        {
            match save_report(cfg, topic, &args, &report, &questions, &results) {
                Ok(artifacts) => format!("{report}\n\n{}", saved_artifacts_text(&artifacts)),
                Err(error) => format!("エラー: {error}\n\n{report}"),
            }
        } else {
            report
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ResearchReportTool) } }

#[cfg(test)]
mod tests {
    use super::{build_sources_csv, extract_results, slug};

    #[test]
    fn ファイル名向けslugを作る() {
        assert_eq!(slug("AI Slides / Docs 調査"), "ai-slides-docs");
        assert_eq!(slug("日本語だけ"), "research-report");
    }

    #[test]
    fn 検索htmlから結果を取り出す() {
        let html = r#"
            <div class="result">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Freport&rut=x">調査ページ</a>
              <a class="result__snippet">概要 テキスト です。</a>
            </div>
        "#;
        let results = extract_results(html, "AI 調査", 3).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/report");
        assert!(results[0].snippet.contains("概要"));
    }

    #[test]
    fn 出典csvをエスケープする() {
        let rows = vec![super::SearchResult {
            title: "A \"quote\"".to_string(),
            url: "https://example.com/?a=1".to_string(),
            snippet: "line1\nline2".to_string(),
            query: "調査".to_string(),
        }];
        let csv = build_sources_csv(&rows);
        assert!(csv.contains("\"A \"\"quote\"\"\""));
        assert!(csv.contains("\"line1 line2\""));
    }
}
