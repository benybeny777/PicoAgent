// skills/rss_feed/mod.rs - RSS/Atom フィードの最新記事取得スキル。

use std::io::Cursor;
use std::time::Duration;

use async_trait::async_trait;
use feed_rs::parser;
use serde_json::{json, Value};
use url::Url;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_FEED_BYTES: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

pub struct RssFeedTool;

#[async_trait]
impl Tool for RssFeedTool {
    fn name(&self) -> &'static str {
        "rss_feed"
    }

    fn label(&self) -> &'static str {
        "RSS/Atom"
    }

    fn description(&self) -> String {
        "RSS/Atom フィードから最新記事を取得する。ニュース、ブログ、更新確認に使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "RSS/Atom フィードURL。省略時は設定値。" },
                "max_items": { "type": "integer", "minimum": 1, "maximum": 20, "description": "取得件数。省略時5件。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_RSS_DEFAULT_URL", "RSS既定URL", "text", ""),
            Field::new("PICOAGENT_RSS_TIMEOUT_SECS", "RSS取得待ち時間（秒、3〜60）", "text", "15"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_RSS_DEFAULT_URL", ""));
        if url.is_empty() {
            return super::config_guide("RSS取得", &["PICOAGENT_RSS_DEFAULT_URL"], "依頼で url を渡すか、既定のRSS/Atom URLを設定してください。");
        }
        let max_items = args
            .get("max_items")
            .and_then(Value::as_u64)
            .filter(|value| (1..=20).contains(value))
            .unwrap_or(5) as usize;
        let timeout = cfg
            .get("PICOAGENT_RSS_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let mut current = match Url::parse(&url) {
            Ok(url) => url,
            Err(error) => return format!("エラー: RSS URLを解析できません: {error}"),
        };
        let mut redirects = 0usize;
        let response = loop {
            let addresses = match crate::tools::http::public_http_addresses(&current).await {
                Ok(addresses) => addresses,
                Err(error) => return format!("エラー: RSS取得先を拒否しました: {error}"),
            };
            let host = current.host_str().unwrap_or_default();
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout))
                .user_agent("PicoAgent/1.0 (RSS tool)")
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, &addresses)
                .build()
            {
                Ok(client) => client,
                Err(error) => return format!("エラー: RSS取得の準備に失敗しました: {error}"),
            };
            let response = match client.get(current.clone()).send().await {
                Ok(response) => response,
                Err(error) => return format!("エラー: RSS取得に失敗しました: {error}"),
            };
            if !response.status().is_redirection() {
                break response;
            }
            let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                return "エラー: RSSのリダイレクト先がありません。".to_string();
            };
            let location = match location.to_str() {
                Ok(location) => location,
                Err(_) => return "エラー: RSSのリダイレクト先が不正です。".to_string(),
            };
            current = match current.join(location) {
                Ok(next) => next,
                Err(error) => return format!("エラー: RSSのリダイレクト先を解析できません: {error}"),
            };
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return format!("エラー: RSSのリダイレクトが{MAX_REDIRECTS}回を超えました。");
            }
        };
        let status = response.status();
        if !status.is_success() {
            return format!("エラー: RSS URL が HTTP {} を返しました。", status);
        }
        let mut response = response;
        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) > MAX_FEED_BYTES => {
                    return format!("エラー: RSS本文が大きすぎます（上限 {MAX_FEED_BYTES} バイト）。");
                }
                Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
                Ok(None) => break,
                Err(error) => return format!("エラー: RSS本文の読み取りに失敗しました: {error}"),
            }
        }
        let feed = match parser::parse(Cursor::new(bytes)) {
            Ok(feed) => feed,
            Err(error) => return format!("エラー: RSS/Atom の解析に失敗しました: {}", error),
        };
        let title = feed.title.map(|text| text.content).unwrap_or_else(|| url.clone());
        let mut out = format!("フィード: {}\n", title);
        for entry in feed.entries.iter().take(max_items) {
            let entry_title = entry
                .title
                .as_ref()
                .map(|text| text.content.as_str())
                .unwrap_or("（無題）");
            let link = entry.links.first().map(|link| link.href.as_str()).unwrap_or("");
            let updated = entry
                .updated
                .or(entry.published)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            out.push_str(&format!("- {} {} {}\n", updated, entry_title, link));
        }
        out
    }
}

inventory::submit! { ToolReg { make: || Box::new(RssFeedTool) } }

#[cfg(test)]
mod tests {
    use super::RssFeedTool;
    use crate::tools::Tool;

    #[tokio::test]
    async fn ローカルrss取得を拒否する() {
        let result = RssFeedTool
            .run(
                &crate::config::Config::new(),
                serde_json::json!({"url": "http://127.0.0.1/feed.xml"}),
            )
            .await;
        assert!(result.contains("取得先を拒否"), "result: {result}");
    }

    #[test]
    fn リダイレクトごとにssrf検査する構造である() {
        let source = include_str!("mod.rs");
        assert!(source.contains("redirect(reqwest::redirect::Policy::none())"));
        assert!(source.contains("public_http_addresses(&current).await"));
        assert!(source.contains("current.join(location)"));
        assert!(source.contains("resolve_to_addrs(host, &addresses)"));
    }
}
