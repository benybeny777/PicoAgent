// skills/web_search/mod.rs - Web検索結果をモデルへ返すエージェント機能コネクタ。
//
// DuckDuckGo の軽量HTML検索を利用し、検索語・件数・応答サイズ・待ち時間に境界を設ける。

use std::time::Duration;

use crate::tools::search_url::direct_url;
use async_trait::async_trait;
use scraper::{Html, Selector};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const SEARCH_URL: &str = "https://html.duckduckgo.com/html/";
const MAX_QUERY_CHARS: usize = 500;
const MAX_RESULTS_LIMIT: usize = crate::config::ints::WEB_SEARCH_MAX_RESULTS.max as usize;
const MAX_RESPONSE_BYTES: usize = 32 * 1024;

pub struct WebSearchTool;

const RESULT_MATERIAL_PROMPT: &str = include_str!("prompts/result-material.md");
const WEB_SEARCH_REQUESTS: [&str; 17] = [
    "WEB検索して",
    "Web検索して",
    "web検索して",
    "ウェブ検索して",
    "ウェブで検索して",
    "Webで検索して",
    "webで検索して",
    "ネットで調べて",
    "サイトを探して",
    "サイト探して",
    "ページを探して",
    "ページ探して",
    "記事を探して",
    "記事探して",
    "リンクを探して",
    "リンク探して",
    "ウェブで探して",
];

fn render_result_material(user_text: &str, tool_result: &str) -> String {
    crate::tools::render_intent_prompt(RESULT_MATERIAL_PROMPT, user_text, tool_result)
}

/// Webを指す語。単独の「web」は「webpack」などを拾うため、助詞や動作と続く形で判定する。
const WEB_CONTEXT_MARKERS: &[&str] = &[
    "webで",
    "webから",
    "web検索",
    "ウェブで",
    "ウェブから",
    "ウェブ検索",
    "ネットで",
    "ネットから",
    "ネット検索",
    "インターネット",
    "ググ",
    "ぐぐ",
];
/// 調べる動作を表す語。ひらがな書きも含める。
const SEARCH_ACTION_MARKERS: &[&str] = &["検索", "調べ", "しらべ", "探", "さが", "ググ", "ぐぐ"];
/// 検索語を作るときに取り除く依頼・催促の言い回し（長いものから順に除く）。
const REQUEST_PHRASES: &[&str] = &[
    "インターネットで",
    "インターネットから",
    "ネットで",
    "ネットから",
    "webで",
    "webから",
    "ウェブで",
    "ウェブから",
    "検索して",
    "調べて",
    "しらべて",
    "ググって",
    "ぐぐって",
    "探して",
    "さがして",
    "くれてないの",
    "くれないの",
    "くれない",
    "ください",
    "ちゃんと",
    "実際に",
    "教えて",
    "おしえて",
];

pub(crate) fn is_explicit_request(text: &str) -> bool {
    if WEB_SEARCH_REQUESTS
        .iter()
        .any(|marker| text.contains(marker))
    {
        return true;
    }
    // 「WEBからしらべて」「ネットでググって」のように、Webを指す語と調べる動作が揃えば明示依頼とみなす。
    let lower = text.to_lowercase();
    WEB_CONTEXT_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
        && SEARCH_ACTION_MARKERS
            .iter()
            .any(|marker| lower.contains(marker))
}

fn should_search(text: &str) -> bool {
    if is_explicit_request(text) {
        return true;
    }
    let lower = text.to_lowercase();
    const INFO_MARKERS: &[&str] = &[
        "しらべ",
        "ググ",
        "ぐぐ",
        "最新",
        "ニュース",
        "今日",
        "昨日",
        "今週",
        "今月",
        "今年",
        "現在",
        "価格",
        "値段",
        "いくら",
        "相場",
        "株価",
        "ランキング",
        "発売",
        "リリース",
        "結果",
        "スコア",
        "順位",
        "誰",
        "だれ",
        "いつ",
        "どこ",
        "なぜ",
        "方法",
        "やり方",
        "調べ",
        "検索",
        "スペック",
        "比較",
        "おすすめ",
        "レビュー",
        "評判",
        "何年",
        "何月",
        "何時",
        "とは",
        "意味",
        "とは何",
        "history",
        "news",
        "price",
    ];
    INFO_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// 発話から依頼・催促の言い回しを除いた話題を返す。話題が残らなければ空文字を返す。
fn strip_request(text: &str) -> String {
    let mut phrases: Vec<String> = WEB_SEARCH_REQUESTS
        .iter()
        .chain(REQUEST_PHRASES)
        .map(|phrase| phrase.to_lowercase())
        .collect();
    phrases.sort_by_key(|phrase| std::cmp::Reverse(phrase.chars().count()));
    let mut query = text.to_lowercase();
    for phrase in &phrases {
        query = query.replace(phrase.as_str(), " ");
    }
    let query = query
        .trim_matches(|character: char| {
            character.is_whitespace() || "、。,.！？!?：:".contains(character)
        })
        .trim();
    // 「天気の サイト探して」の命令部を除いた後に残る末尾の孤立助詞だけを落とす。
    // 文中の助詞は検索語の意味を変えるため触らない。
    let query = ["について", "とは", "の", "を"]
        .iter()
        .find_map(|suffix| query.strip_suffix(suffix))
        .map(str::trim_end)
        .unwrap_or(query);
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 検索語を組み立てる。話題を含まない催促（「WEBから調べてくれてないの？」）では、
/// 直前の利用者発話（新しい順）から話題を最大2件補い、古い順に並べる。
fn search_query_in_conversation(text: &str, earlier_user_texts: &[&str]) -> String {
    let query = strip_request(text);
    if !query.is_empty() {
        return query;
    }
    let mut topics: Vec<String> = earlier_user_texts
        .iter()
        .map(|earlier| strip_request(earlier))
        .filter(|topic| !topic.is_empty())
        .take(2)
        .collect();
    topics.reverse();
    let combined = topics.join(" ");
    let combined = if combined.is_empty() {
        text.trim().to_string()
    } else {
        combined
    };
    combined.chars().take(MAX_QUERY_CHARS).collect()
}

fn extract_results(html: &str, query: &str, max_results: usize) -> Result<String, String> {
    let document = Html::parse_document(html);
    let result_selector = Selector::parse(".result")
        .map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;
    let title_selector = Selector::parse(".result__a")
        .map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;
    let snippet_selector = Selector::parse(".result__snippet")
        .map_err(|e| format!("検索結果の解析準備に失敗しました: {}", e))?;

    let mut output = format!("Web検索結果（検索語: {}）\n", query);
    let mut count = 0;
    for result in document.select(&result_selector) {
        let Some(title_node) = result.select(&title_selector).next() else {
            continue;
        };
        let Some(href) = title_node.value().attr("href") else {
            continue;
        };
        let title = title_node
            .text()
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();
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
        return Err(
            "検索結果を取得できませんでした。検索語を変えて再試行してください。".to_string(),
        );
    }
    crate::text::truncate_bytes_with_notice(
        &mut output,
        MAX_RESPONSE_BYTES,
        "\n…（応答サイズ上限で打ち切り）",
    );
    Ok(output)
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn intent_priority(&self) -> i32 {
        100
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        self.intent_invocation_in_conversation(text, &[])
    }

    fn intent_invocation_in_conversation(
        &self,
        text: &str,
        earlier_user_texts: &[&str],
    ) -> Option<crate::tools::IntentInvocation> {
        should_search(text).then(|| {
            crate::tools::IntentInvocation::material(
                json!({ "query": search_query_in_conversation(text, earlier_user_texts) }),
                render_result_material,
                true,
            )
        })
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
            Field::int(&crate::config::ints::WEB_SEARCH_MAX_RESULTS),
            Field::int(&crate::config::ints::WEB_SEARCH_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let query = match args.get("query").and_then(Value::as_str).map(str::trim) {
            Some(query) if !query.is_empty() => query,
            _ => return "エラー: query が指定されていません。".to_string(),
        };
        if query.chars().count() > MAX_QUERY_CHARS {
            return format!(
                "エラー: query は {} 文字以内で指定してください。",
                MAX_QUERY_CHARS
            );
        }

        let configured_results =
            crate::tool_try!(cfg.int(&crate::config::ints::WEB_SEARCH_MAX_RESULTS)) as usize;
        let max_results = match args.get("max_results") {
            None => configured_results,
            Some(value) => match value
                .as_u64()
                .and_then(|number| usize::try_from(number).ok())
            {
                Some(number) if (1..=MAX_RESULTS_LIMIT).contains(&number) => number,
                _ => {
                    return format!(
                        "エラー: max_results は 1〜{} の整数で指定してください。",
                        MAX_RESULTS_LIMIT
                    )
                }
            },
        };
        let timeout_secs =
            crate::tool_try!(cfg.int(&crate::config::ints::WEB_SEARCH_TIMEOUT_SECS)) as u64;

        let request_timeout = Duration::from_secs(timeout_secs as u64);
        let response = match crate::tool_try!(crate::tools::http::service(
            reqwest::Method::GET,
            SEARCH_URL,
            request_timeout
        ))
        .query(&[("q", query)])
        .send()
        .await
        {
            Ok(response) => response,
            Err(error) => {
                return format!(
                    "エラー: Web検索への接続に失敗しました: {}",
                    error.without_url()
                )
            }
        };
        let status = response.status();
        if !status.is_success() {
            return crate::tools::http::service_reply(cfg, Ok(response), "Web検索").await;
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
    use super::{
        direct_url, extract_results, is_explicit_request, search_query_in_conversation,
        WebSearchTool,
    };
    use crate::tools::Tool;

    #[test]
    fn サイトを探す要求を明示的なweb検索として扱う() {
        assert!(is_explicit_request("天気のサイト探して"));
        assert_eq!(
            search_query_in_conversation("天気のサイト探して", &[]),
            "天気"
        );
    }

    #[test]
    fn ひらがなや催促の言い回しでもweb検索を明示依頼として扱う() {
        assert!(is_explicit_request("WEBからしらべてくれてないの？"));
        assert!(is_explicit_request("ネットでググって"));
        assert!(!is_explicit_request("webpackの設定ファイルを開いて"));
    }

    #[test]
    fn 話題のない催促は直前の発話から検索語を補う() {
        let earlier = [
            "考えるじゃなくて実際のやつおしえて",
            "みこの歌のタイトルを教えて",
        ];
        assert_eq!(
            search_query_in_conversation("WEBからしらべてくれてないの？", &earlier),
            "みこの歌のタイトル 考えるじゃなくて実際のやつ"
        );
        assert_eq!(
            search_query_in_conversation("ネットでRustの最新版を調べて", &earlier),
            "rustの最新版"
        );
        let invocation = WebSearchTool
            .intent_invocation_in_conversation("WEBからしらべてくれてないの？", &earlier[1..])
            .expect("明示的な検索依頼は先回り実行する");
        assert_eq!(invocation.args["query"], "みこの歌のタイトル");
    }

    #[test]
    fn duckduckgoの転送urlを直接urlへ戻す() {
        let url =
            direct_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fx%3D1&rut=abc");
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
