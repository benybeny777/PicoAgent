// tools/web_search.rs - Web検索結果をモデルへ返すエージェント機能コネクタ。
//
// DuckDuckGo の軽量HTML検索を利用し、検索語・件数・応答サイズ・待ち時間に境界を設ける。

use std::time::Duration;

use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::{json, Value};
use url::Url;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const SEARCH_URL: &str = "https://html.duckduckgo.com/html/";
const MAX_QUERY_CHARS: usize = 500;
const MAX_RESULTS_LIMIT: usize = 8;
const MAX_RESPONSE_BYTES: usize = 32 * 1024;

pub struct WebSearchTool;

fn parse_setting(cfg: &Config, key: &str, fallback: &str, min: usize, max: usize) -> Result<usize, String> {
    let raw = cfg.get(key, fallback);
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("設定 {} は {}〜{} の整数で指定してください。", key, min, max))?;
    if !(min..=max).contains(&value) {
        return Err(format!("設定 {} は {}〜{} の範囲で指定してください。", key, min, max));
    }
    Ok(value)
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

fn trim_to_bytes(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n…（応答サイズ上限で打ち切り）");
}

fn extract_results(html: &str, query: &str, max_results: usize) -> Result<String, String> {
    let document = Html::parse_document(html);
    let result_selector = Selector::parse(".result").map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;
    let title_selector = Selector::parse(".result__a").map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;
    let snippet_selector = Selector::parse(".result__snippet").map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;

    let mut output = format!("Web検索結果（検索語: {}）\n", query);
    let mut count = 0;
    for result in document.select(&result_selector) {
        let Some(title_node) = result.select(&title_selector).next() else {
            continue;
        };
        let Some(href) = title_node.value().attr("href") else {
            continue;
        };
        let title = title_node.text().collect::<Vec<_>>().join("").trim().to_string();
        if title.is_empty() {
            continue;
        }
        let snippet = result
            .select(&snippet_selector)
            .next()
            .map(|node| node.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        count += 1;
        output.push_str(&format!(
            "\n{}. {}\nURL: {}\n概要: {}\n",
            count,
            title,
            direct_url(href),
            snippet.split_whitespace().collect::<Vec<_>>().join(" ")
        ));
        if count >= max_results {
            break;
        }
    }

    if count == 0 {
        return Err("検索結果を取得できませんでした。検索語を変えて再試行してください。".to_string());
    }
    trim_to_bytes(&mut output, MAX_RESPONSE_BYTES);
    Ok(output)
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn label(&self) -> &'static str {
        "Web検索"
    }

    fn description(&self) -> String {
        "Webを検索して、タイトル・URL・概要を返す。最新情報、ニュース、現在の仕様、知らない事実、出典が必要な質問ではこのツールを使い、参照元をMarkdownハイパーリンクで示して回答する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Webで検索する具体的な検索語。"
                },
                "max_results": {
                    "type": "integer",
                    "description": "返す結果件数。省略時は設定値を使う。",
                    "minimum": 1,
                    "maximum": MAX_RESULTS_LIMIT
                }
            },
            "required": ["query"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("CODA_WEB_SEARCH_MAX_RESULTS", "Web検索の結果件数（1〜8）", "text", "5"),
            Field::new("CODA_WEB_SEARCH_TIMEOUT_SECS", "Web検索の待ち時間（秒、3〜60）", "text", "15"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let query = match args.get("query").and_then(Value::as_str).map(str::trim) {
            Some(query) if !query.is_empty() => query,
            _ => return "エラー: query が指定されていません。".to_string(),
        };
        if query.chars().count() > MAX_QUERY_CHARS {
            return format!("エラー: query は {} 文字以内で指定してください。", MAX_QUERY_CHARS);
        }

        let configured_results = match parse_setting(cfg, "CODA_WEB_SEARCH_MAX_RESULTS", "5", 1, MAX_RESULTS_LIMIT) {
            Ok(value) => value,
            Err(error) => return format!("エラー: {}", error),
        };
        let max_results = match args.get("max_results") {
            None => configured_results,
            Some(value) => match value.as_u64().and_then(|number| usize::try_from(number).ok()) {
                Some(number) if (1..=MAX_RESULTS_LIMIT).contains(&number) => number,
                _ => return format!("エラー: max_results は 1〜{} の整数で指定してください。", MAX_RESULTS_LIMIT),
            },
        };
        let timeout_secs = match parse_setting(cfg, "CODA_WEB_SEARCH_TIMEOUT_SECS", "15", 3, 60) {
            Ok(value) => value,
            Err(error) => return format!("エラー: {}", error),
        };

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs as u64))
            .user_agent("PicoAgent/1.0 (Web search tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: Web検索の準備に失敗しました: {}", error),
        };
        let response = match client.get(SEARCH_URL).query(&[("q", query)]).send().await {
            Ok(response) => response,
            Err(error) => return format!("エラー: Web検索への接続に失敗しました: {}", error),
        };
        let status = response.status();
        if !status.is_success() {
            return format!("エラー: Web検索が HTTP {} を返しました。時間を置いて再試行してください。", status);
        }
        let body = match response.text().await {
            Ok(body) => body,
            Err(error) => return format!("エラー: Web検索結果の読み取りに失敗しました: {}", error),
        };
        match extract_results(&body, query, max_results) {
            Ok(results) => results,
            Err(error) => format!("エラー: {}", error),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(WebSearchTool) } }

#[cfg(test)]
mod tests {
    use super::{direct_url, extract_results};

    #[test]
    fn duckduckgoの転送urlを直接urlへ戻す() {
        let url = direct_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fx%3D1&rut=abc");
        assert_eq!(url, "https://example.com/a?x=1");
    }

    #[test]
    fn htmlから検索結果を取り出す() {
        let html = r#"
            <div class="result">
              <a class="result__a" href="https://example.com/">例のページ</a>
              <a class="result__snippet">検索結果の概要です。</a>
            </div>
        "#;
        let output = extract_results(html, "例", 1).unwrap();
        assert!(output.contains("1. 例のページ"));
        assert!(output.contains("URL: https://example.com/"));
        assert!(output.contains("検索結果の概要です。"));
    }
}
