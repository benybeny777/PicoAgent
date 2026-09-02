// skills/market_indices/mod.rs - Yahoo Finance の chart API による市場指数取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const YAHOO_CHART_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";

pub struct MarketIndicesTool;

fn parse_days(text: &str) -> Option<i64> {
    let chars: Vec<char> = text.chars().collect();
    for (index, character) in chars.iter().enumerate() {
        if character.is_ascii_digit() {
            continue;
        }
        let unit_month = *character == 'ヶ' || *character == 'か' || *character == '月';
        if !unit_month && *character != '日' {
            continue;
        }
        let mut cursor = index;
        let mut number = String::new();
        while cursor > 0 && chars[cursor - 1].is_ascii_digit() {
            number.insert(0, chars[cursor - 1]);
            cursor -= 1;
        }
        if let Ok(number) = number.parse::<i64>() {
            return Some(if unit_month { number * 30 } else { number }.clamp(2, 365));
        }
    }
    None
}

fn direct_query(text: &str) -> Option<Value> {
    let lower = text.to_lowercase();
    let market_words = [
        "日経平均",
        "日系平均",
        "nikkei",
        "n225",
        "topix",
        "トピックス",
        "ダウ",
        "dow",
        "nasdaq",
        "ナスダック",
        "s&p",
        "sp500",
        "s&p500",
        "株価指数",
        "市場",
    ];
    if !market_words
        .iter()
        .any(|word| lower.contains(&word.to_lowercase()))
    {
        return None;
    }
    let symbol = if lower.contains("topix") || text.contains("トピックス") {
        "topix"
    } else if lower.contains("dow") || text.contains("ダウ") {
        "dow"
    } else if lower.contains("nasdaq") || text.contains("ナスダック") {
        "nasdaq"
    } else if lower.contains("s&p") || lower.contains("sp500") {
        "sp500"
    } else {
        "nikkei225"
    };
    let chart = ["推移", "チャート", "グラフ", "trend"]
        .iter()
        .any(|word| lower.contains(word));
    let mut args = serde_json::Map::new();
    args.insert("symbol".to_string(), json!(symbol));
    if chart {
        args.insert("chart".to_string(), json!(true));
        if let Some(days) = parse_days(text) {
            args.insert("days".to_string(), json!(days));
        }
    }
    Some(Value::Object(args))
}

fn sparkline(values: &[f64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if max.partial_cmp(&min) != Some(std::cmp::Ordering::Greater) {
        return BARS[0].to_string().repeat(values.len());
    }
    values
        .iter()
        .map(|&value| {
            let idx = (((value - min) / (max - min)) * 7.0).round() as usize;
            BARS[idx.min(7)]
        })
        .collect()
}

fn symbol_info(input: &str) -> (&'static str, &'static str) {
    match input.trim().to_lowercase().as_str() {
        "topix" | "トピックス" => ("^TOPX", "TOPIX"),
        "dow" | "dji" | "ダウ" => ("^DJI", "NYダウ"),
        "nasdaq" | "ixic" | "ナスダック" => ("^IXIC", "NASDAQ総合"),
        "sp500" | "s&p500" | "s&p" => ("^GSPC", "S&P 500"),
        _ => ("^N225", "日経平均"),
    }
}

fn encoded_symbol(symbol: &str) -> String {
    symbol.replace('^', "%5E")
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn close_values(result: &Value) -> Vec<f64> {
    value_at_path(result, &["indicators", "quote"])
        .and_then(Value::as_array)
        .and_then(|quotes| quotes.first())
        .and_then(|quote| quote.get("close"))
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

fn format_time(timestamp: Option<i64>) -> String {
    use chrono::{Local, TimeZone};
    timestamp
        .and_then(|ts| Local.timestamp_opt(ts, 0).single())
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "日時不明".to_string())
}

#[async_trait]
impl Tool for MarketIndicesTool {
    fn name(&self) -> &'static str {
        "market_indices"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "日経平均",
            "日系平均",
            "nikkei",
            "topix",
            "トピックス",
            "ダウ",
            "nasdaq",
            "ナスダック",
            "s&p",
            "株価指数",
        ]
    }

    fn intent_priority(&self) -> i32 {
        1_000
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        direct_query(text).map(crate::tools::IntentInvocation::direct)
    }

    fn label(&self) -> &'static str {
        "市場指数"
    }

    fn description(&self) -> String {
        "日経平均、TOPIX、NYダウ、NASDAQ、S&P500 などの市場指数を取得し、必要なら推移チャートを表示する。日経平均や株価指数を聞かれたら使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "指数名。nikkei225/topix/dow/nasdaq/sp500。" },
                "chart": { "type": "boolean", "description": "推移チャートを出すなら true。" },
                "days": { "type": "integer", "description": "チャート対象日数（2〜365、既定30）。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_MARKET_TIMEOUT_SECS",
            "市場指数API待ち時間（秒、3〜60）",
            "text",
            "15",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let requested = args
            .get("symbol")
            .and_then(Value::as_str)
            .unwrap_or("nikkei225");
        let (symbol, label) = symbol_info(requested);
        let want_chart = args.get("chart").and_then(Value::as_bool).unwrap_or(false)
            || args.get("days").is_some();
        let days = args
            .get("days")
            .and_then(Value::as_i64)
            .unwrap_or(if want_chart { 30 } else { 5 })
            .clamp(2, 365);
        let range = if days <= 5 {
            "5d".to_string()
        } else if days <= 31 {
            "1mo".to_string()
        } else if days <= 93 {
            "3mo".to_string()
        } else if days <= 186 {
            "6mo".to_string()
        } else {
            "1y".to_string()
        };
        let timeout = cfg
            .get("PICOAGENT_MARKET_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("Mozilla/5.0 PicoAgent/1.0 (Market indices tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: 市場指数APIの準備に失敗しました: {error}"),
        };
        let url = format!("{YAHOO_CHART_BASE}/{}", encoded_symbol(symbol));
        let response = match client
            .get(url)
            .query(&[("range", range.as_str()), ("interval", "1d")])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return format!("エラー: 市場指数APIへの接続に失敗しました: {error}"),
        };
        let status = response.status();
        if !status.is_success() {
            return format!("エラー: 市場指数APIが HTTP {status} を返しました。");
        }
        let data: Value = match response.json().await {
            Ok(data) => data,
            Err(error) => return format!("エラー: 市場指数APIの応答を読めませんでした: {error}"),
        };
        let Some(result) = value_at_path(&data, &["chart", "result"])
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        else {
            return "エラー: 市場指数データを取得できませんでした。".to_string();
        };
        let meta = result.get("meta").unwrap_or(&Value::Null);
        let price = meta
            .get("regularMarketPrice")
            .and_then(Value::as_f64)
            .or_else(|| close_values(result).last().copied())
            .unwrap_or(0.0);
        let previous = meta
            .get("chartPreviousClose")
            .and_then(Value::as_f64)
            .unwrap_or(price);
        let change = price - previous;
        let percent = if previous != 0.0 {
            change / previous * 100.0
        } else {
            0.0
        };
        let sign = if change >= 0.0 { "+" } else { "" };
        let time = format_time(meta.get("regularMarketTime").and_then(Value::as_i64));
        let mut out = format!(
            "{label}（{symbol}）\n{price:.2}（{sign}{change:.2} / {sign}{percent:.2}%）\n更新: {time}"
        );
        if want_chart {
            let values = close_values(result);
            if values.len() >= 2 {
                let first = values.first().copied().unwrap_or(0.0);
                let last = values.last().copied().unwrap_or(price);
                out.push_str(&format!(
                    "\n直近{}日の推移\n{}\n始値 {:.2} / 終値 {:.2}",
                    days,
                    sparkline(&values),
                    first,
                    last
                ));
            }
        }
        out
    }
}

inventory::submit! { ToolReg { make: || Box::new(MarketIndicesTool) } }
