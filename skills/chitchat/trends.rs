use std::io::Cursor;
use std::time::Duration;

use feed_rs::parser;
use serde_json::Value;

use crate::config::Config;

const DEFAULT_NEWS_RSS: &str = "https://news.google.com/rss?hl=ja&gl=JP&ceid=JP:ja";
const DEFAULT_TRENDS_RSS: &str = "https://trends.google.com/trending/rss?geo=JP";
const X_SEARCH_URL: &str = "https://api.x.com/2/tweets/search/recent";
const MAX_FEED_BYTES: usize = 1024 * 1024;

pub async fn topic_brief(cfg: &Config) -> Option<String> {
    let mode = cfg
        .get("PICOAGENT_CHATTER_TOPIC_MODE", "auto")
        .trim()
        .to_ascii_lowercase();
    if mode == "off" || mode == "none" {
        return None;
    }
    let timeout = timeout_secs(cfg);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout))
        .user_agent("PicoAgent/1.0 (idle chatter topics)")
        .build()
        .ok()?;

    if mode != "news" && mode != "trends" {
        if let Some(topic) = x_topic(cfg, &client).await {
            return Some(topic);
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
            return Some(topic);
        }
    }
    if mode != "x" {
        return rss_topics(
            cfg,
            &client,
            "PICOAGENT_CHATTER_NEWS_RSS_URLS",
            DEFAULT_NEWS_RSS,
            "トップニュース候補",
        )
        .await;
    }
    None
}

fn timeout_secs(cfg: &Config) -> u64 {
    cfg.get("PICOAGENT_CHATTER_TOPIC_TIMEOUT_SECS", "8")
        .parse::<u64>()
        .ok()
        .filter(|value| (3..=30).contains(value))
        .unwrap_or(8)
}

async fn x_topic(cfg: &Config, client: &reqwest::Client) -> Option<String> {
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
        Some(format!(
            "Xで反応が多い話題候補: {}",
            compact_line(text, 180)
        ))
    }
}

async fn rss_topics(
    cfg: &Config,
    client: &reqwest::Client,
    key: &str,
    default_urls: &str,
    heading: &str,
) -> Option<String> {
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
        let titles: Vec<String> = feed
            .entries
            .iter()
            .filter_map(|entry| entry.title.as_ref().map(|title| title.content.trim()))
            .filter(|title| !title.is_empty())
            .take(5)
            .map(|title| compact_line(title, 90))
            .collect();
        if !titles.is_empty() {
            let mut out = format!("{heading}:");
            for title in titles {
                out.push_str("\n- ");
                out.push_str(&title);
            }
            return Some(out);
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
