// skills/news_scrape/mod.rs - Google News / Yahoo!ニュースのHTMLから見出し候補を取得する。

use std::time::Duration;

use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::{json, Value};
use url::Url;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_HTML_BYTES: usize = 1024 * 1024;
const MAX_ITEMS_LIMIT: usize = crate::config::ints::NEWS_SCRAPE_MAX_ITEMS.max as usize;

struct NewsSource {
    name: &'static str,
    url: &'static str,
}

const SOURCES: &[NewsSource] = &[
    NewsSource {
        name: "Google News",
        url: "https://news.google.com/topstories?hl=ja&gl=JP&ceid=JP:ja",
    },
    NewsSource {
        name: "Yahoo!ニュース",
        url: "https://news.yahoo.co.jp/",
    },
];

pub struct NewsScrapeTool;

const SUMMARY_MATERIAL_PROMPT: &str = include_str!("prompts/summary-material.md");

fn render_summary_material(user_text: &str, tool_result: &str) -> String {
    crate::tools::render_intent_prompt(SUMMARY_MATERIAL_PROMPT, user_text, tool_result)
}

fn is_summary_query(text: &str) -> bool {
    let lower = text.to_lowercase();
    let normalized = lower
        .replace("ニューす", "ニュース")
        .replace("にゅーす", "ニュース")
        .replace("ﾆｭｰｽ", "ニュース");
    if lower.contains("ニュースリリース")
        || lower.contains("リリースノート")
        || lower.contains("release note")
        || ["詳しく", "詳細", "深掘り", "深堀り", "掘り下げ", "背景を"]
            .iter()
            .any(|marker| normalized.contains(marker))
    {
        return false;
    }
    let has_news_word = normalized.contains("ニュース")
        || normalized.contains("news")
        || normalized.contains("出来事")
        || normalized.contains("時事")
        || normalized.contains("ヘッドライン");
    if !has_news_word {
        return false;
    }
    let asks_current_or_summary = normalized.contains("今日")
        || normalized.contains("本日")
        || normalized.contains("最新")
        || normalized.contains("いま")
        || normalized.contains("今の")
        || normalized.contains("最近")
        || normalized.contains("主要")
        || normalized.contains("トップ")
        || normalized.contains("まとめ")
        || normalized.contains("要約")
        || normalized.contains("教えて")
        || normalized.contains("知りたい");
    asks_current_or_summary || text.chars().count() <= 16
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_noise_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    title.chars().count() < 8
        || title.chars().count() > 120
        || lower.contains("javascript")
        || title.contains("ログイン")
        || title.contains("もっと見る")
        || title.contains("一覧")
        || title.contains("設定")
        || title.contains("ヘルプ")
        || title.contains("利用規約")
        || title.contains("プライバシー")
}

fn source_allows_url(source_name: &str, url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    match source_name {
        "Google News" => {
            host == "news.google.com"
                && (url.path().contains("/articles/") || url.path().contains("/read/"))
        }
        "Yahoo!ニュース" => {
            host == "news.yahoo.co.jp"
                && (url.path().contains("/articles/") || url.path().contains("/pickup/"))
        }
        _ => false,
    }
}

fn extract_items(
    source: &NewsSource,
    html: &str,
    max_items: usize,
) -> Result<Vec<(String, String)>, String> {
    let base = Url::parse(source.url)
        .map_err(|error| format!("ニュースURLの解析に失敗しました: {error}"))?;
    let document = Html::parse_document(html);
    let selector = Selector::parse("a")
        .map_err(|error| format!("ニュースHTMLの解析準備に失敗しました: {error}"))?;
    let mut items: Vec<(String, String)> = Vec::new();
    for node in document.select(&selector) {
        let Some(href) = node.value().attr("href") else {
            continue;
        };
        let Ok(url) = base.join(href) else {
            continue;
        };
        if !source_allows_url(source.name, &url) {
            continue;
        }
        let title = normalize_text(&node.text().collect::<Vec<_>>().join(" "));
        if is_noise_title(&title) || items.iter().any(|(existing, _)| existing == &title) {
            continue;
        }
        items.push((title, url.to_string()));
        if items.len() >= max_items {
            break;
        }
    }
    Ok(items)
}

#[async_trait]
impl Tool for NewsScrapeTool {
    fn name(&self) -> &'static str {
        "news_scrape"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "ニュース",
            "ニューす",
            "にゅーす",
            "news",
            "出来事",
            "時事",
            "ヘッドライン",
        ]
    }

    fn intent_priority(&self) -> i32 {
        1_300
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        is_summary_query(text).then(|| {
            crate::tools::IntentInvocation::material(
                json!({ "max_items": 8 }),
                render_summary_material,
                true,
            )
        })
    }

    fn label(&self) -> &'static str {
        "ニュース取得"
    }

    fn description(&self) -> String {
        "Google News と Yahoo!ニュースのHTMLから最新ニュース見出し候補を取得する。今日のニュース要約に使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "max_items": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_ITEMS_LIMIT,
                    "description": "各ニュースサイトから取得する最大件数。省略時は設定値。"
                }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::int(&crate::config::ints::NEWS_SCRAPE_MAX_ITEMS),
            Field::int(&crate::config::ints::NEWS_SCRAPE_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let configured_items =
            crate::tool_try!(cfg.int(&crate::config::ints::NEWS_SCRAPE_MAX_ITEMS)) as usize;
        let max_items = args
            .get("max_items")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (1..=MAX_ITEMS_LIMIT).contains(value))
            .unwrap_or(configured_items);
        let timeout_secs =
            crate::tool_try!(cfg.int(&crate::config::ints::NEWS_SCRAPE_TIMEOUT_SECS)) as u64;
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs as u64))
            .user_agent("PicoAgent/1.0 (news scrape tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: ニュース取得の準備に失敗しました: {error}"),
        };

        let mut output = String::from("ニュースHTML取得結果\n");
        let mut total = 0usize;
        for source in SOURCES {
            output.push_str(&format!("\n情報源: {}\n", source.name));
            let response = match client.get(source.url).send().await {
                Ok(response) => response,
                Err(error) => {
                    output.push_str(&format!("- エラー: 接続に失敗しました: {error}\n"));
                    continue;
                }
            };
            let status = response.status();
            if !status.is_success() {
                output.push_str(&format!("- エラー: HTTP {status}\n"));
                continue;
            }
            let bytes = match response.bytes().await {
                Ok(bytes) => bytes,
                Err(error) => {
                    output.push_str(&format!(
                        "- エラー: 本文の読み取りに失敗しました: {error}\n"
                    ));
                    continue;
                }
            };
            if bytes.len() > MAX_HTML_BYTES {
                output.push_str(&format!(
                    "- エラー: HTMLが大きすぎます（上限 {MAX_HTML_BYTES} バイト）\n"
                ));
                continue;
            }
            let html = String::from_utf8_lossy(&bytes);
            let items = match extract_items(source, &html, max_items) {
                Ok(items) => items,
                Err(error) => {
                    output.push_str(&format!("- エラー: {error}\n"));
                    continue;
                }
            };
            if items.is_empty() {
                output.push_str("- 見出し候補を取得できませんでした。\n");
                continue;
            }
            for (title, url) in items {
                total += 1;
                output.push_str(&format!("- {title}\n  URL: {url}\n"));
            }
        }
        if total == 0 {
            output.push_str("\nエラー: Google News / Yahoo!ニュースのどちらからも見出し候補を取得できませんでした。");
        }
        output
    }
}

inventory::submit! { ToolReg { make: || Box::new(NewsScrapeTool) } }

#[cfg(test)]
mod tests {
    use super::{extract_items, is_summary_query, NewsSource};

    #[test]
    fn google_newsの見出しを抽出する() {
        let source = NewsSource {
            name: "Google News",
            url: "https://news.google.com/topstories?hl=ja&gl=JP&ceid=JP:ja",
        };
        let html = r#"<a href="./articles/CBMiTest">政治ニュースの重要な見出し</a>"#;
        let items = extract_items(&source, html, 5).unwrap();
        assert_eq!(items[0].0, "政治ニュースの重要な見出し");
        assert!(items[0].1.contains("news.google.com/articles/CBMiTest"));
    }

    #[test]
    fn yahooニュースの見出しを抽出する() {
        let source = NewsSource {
            name: "Yahoo!ニュース",
            url: "https://news.yahoo.co.jp/",
        };
        let html = r#"<a href="/articles/abc123">社会ニュースの重要な見出し</a>"#;
        let items = extract_items(&source, html, 5).unwrap();
        assert_eq!(items[0].0, "社会ニュースの重要な見出し");
        assert!(items[0].1.contains("news.yahoo.co.jp/articles/abc123"));
    }

    #[test]
    fn ニュースの深掘り質問を一般ニュース一覧へ戻さない() {
        assert!(!is_summary_query("このニュースを詳しく教えて"));
        assert!(!is_summary_query("宇宙開発のニュースを深掘りして"));
        assert!(is_summary_query("今日のニュースは？"));
    }
}
