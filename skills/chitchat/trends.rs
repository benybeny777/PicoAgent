use std::io::Cursor;

use feed_rs::parser;
use serde_json::Value;

use crate::config::Config;

const DEFAULT_NEWS_RSS: &str = "https://news.google.com/rss?hl=ja&gl=JP&ceid=JP:ja";
const DEFAULT_TRENDS_RSS: &str = "https://trends.google.com/trending/rss?geo=JP";
const X_SEARCH_URL: &str = "https://api.x.com/2/tweets/search/recent";
const MAX_FEED_BYTES: usize = 1024 * 1024;

// 話題メモ1件。`memo` はモデルへ渡す素材文、`sources` は出典（記事タイトルとURL）。
pub struct TopicBrief {
    pub memo: String,
    pub sources: Vec<TopicSource>,
}

pub struct TopicSource {
    pub title: String,
    pub url: String,
}

pub async fn topic_brief(cfg: &Config) -> Result<Option<TopicBrief>, String> {
    let timeout = cfg.duration_secs(&crate::config::ints::CHATTER_TOPIC_TIMEOUT_SECS)?;
    let mode = cfg
        .get("PICOAGENT_CHATTER_TOPIC_MODE", "auto")
        .trim()
        .to_ascii_lowercase();
    if mode == "off" || mode == "none" {
        return Ok(None);
    }
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("PicoAgent/1.0 (idle chatter topics)")
        .build()
        .map_err(|error| format!("話題取得の準備に失敗しました: {error}"))?;

    if mode != "news" && mode != "trends" {
        if let Some(topic) = x_topic(cfg, &client).await {
            return Ok(Some(topic));
        }
    }
    if mode != "x" && mode != "news" {
        if let Some(topic) = rss_topics(
            cfg,
            &client,
            "PICOAGENT_CHATTER_TRENDS_RSS_URLS",
            DEFAULT_TRENDS_RSS,
            "急上昇ワード候補",
        )
        .await
        {
            return Ok(Some(topic));
        }
    }
    if mode != "x" {
        return Ok(rss_topics(
            cfg,
            &client,
            "PICOAGENT_CHATTER_NEWS_RSS_URLS",
            DEFAULT_NEWS_RSS,
            "トップニュース候補",
        )
        .await);
    }
    Ok(None)
}

async fn x_topic(cfg: &Config, client: &reqwest::Client) -> Option<TopicBrief> {
    let token = cfg.get("PICOAGENT_X_BEARER_TOKEN", "");
    if token.is_empty() {
        return None;
    }
    let query = cfg.get(
        "PICOAGENT_CHATTER_X_QUERY",
        "lang:ja -is:retweet min_faves:1000",
    );
    let response = client
        .get(X_SEARCH_URL)
        .bearer_auth(token)
        .query(&[
            ("query", query.as_str()),
            ("max_results", "10"),
            ("tweet.fields", "created_at,public_metrics"),
        ])
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let parsed: Value = response.json().await.ok()?;
    let tweets = parsed.get("data")?.as_array()?;
    let best = tweets.iter().max_by_key(|tweet| {
        let metrics = tweet.get("public_metrics").unwrap_or(&Value::Null);
        let likes = metrics
            .get("like_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let reposts = metrics
            .get("retweet_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let replies = metrics
            .get("reply_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        likes + reposts * 2 + replies
    })?;
    let text = best.get("text")?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(TopicBrief {
            memo: format!("Xで反応が多い話題候補: {}", compact_line(text, 180)),
            sources: Vec::new(),
        })
    }
}

async fn rss_topics(
    cfg: &Config,
    client: &reqwest::Client,
    key: &str,
    default_urls: &str,
    heading: &str,
) -> Option<TopicBrief> {
    let urls = cfg.get(key, default_urls);
    for url in urls.split(',').map(str::trim).filter(|url| !url.is_empty()) {
        let response = match client.get(url).send().await {
            Ok(response) if response.status().is_success() => response,
            _ => continue,
        };
        let bytes = match response.bytes().await {
            Ok(bytes) if bytes.len() <= MAX_FEED_BYTES => bytes,
            _ => continue,
        };
        let feed = match parser::parse(Cursor::new(bytes)) {
            Ok(feed) => feed,
            Err(_) => continue,
        };
        let mut sources: Vec<TopicSource> = Vec::new();
        for entry in feed.entries.iter() {
            let Some(title) = entry.title.as_ref().map(|title| title.content.trim()) else {
                continue;
            };
            if title.is_empty() {
                continue;
            }
            let link = entry
                .links
                .iter()
                .find(|link| link.href.starts_with("http"))
                .map(|link| link.href.clone())
                .unwrap_or_default();
            sources.push(TopicSource {
                title: compact_line(title, 90),
                url: link,
            });
            if sources.len() >= 5 {
                break;
            }
        }
        if !sources.is_empty() {
            let mut memo = format!("{heading}:");
            for source in &sources {
                memo.push_str("\n- ");
                memo.push_str(&source.title);
            }
            return Some(TopicBrief { memo, sources });
        }
    }
    None
}

fn compact_line(text: &str, max_chars: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max_chars {
        return line;
    }
    let mut out = line.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}
