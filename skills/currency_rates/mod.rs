// skills/currency_rates/mod.rs - Frankfurter API による為替レート取得・換算スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const FRANKFURTER_BASE: &str = "https://api.frankfurter.app";

pub struct CurrencyRatesTool;

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
    const NON_JPY: &[(&str, &str)] = &[
        ("豪ドル", "AUD"),
        ("カナダドル", "CAD"),
        ("人民元", "CNY"),
        ("ドル", "USD"),
        ("ユーロ", "EUR"),
        ("ポンド", "GBP"),
        ("ウォン", "KRW"),
        ("フラン", "CHF"),
        ("元", "CNY"),
        ("usd", "USD"),
        ("eur", "EUR"),
        ("gbp", "GBP"),
        ("aud", "AUD"),
        ("cny", "CNY"),
        ("krw", "KRW"),
        ("chf", "CHF"),
        ("cad", "CAD"),
    ];
    let base = NON_JPY
        .iter()
        .filter_map(|(token, code)| lower.find(token).map(|position| (position, *code)))
        .min_by_key(|(position, _)| *position)
        .map(|(_, code)| code);
    let has_jpy = lower.contains('円') || lower.contains("jpy");
    let strong = ["為替", "かわせ", "レート", "相場"]
        .iter()
        .any(|word| lower.contains(word));
    let chart = ["推移", "チャート", "グラフ", "trend"]
        .iter()
        .any(|word| lower.contains(word));
    if !(strong || base.is_some() || (has_jpy && (chart || lower.contains("換算")))) {
        return None;
    }

    let mut args = serde_json::Map::new();
    if let Some(base) = base {
        args.insert("base".to_string(), json!(base));
    }
    if has_jpy {
        args.insert("symbols".to_string(), json!("JPY"));
    }
    if chart {
        args.insert("chart".to_string(), json!(true));
        if let Some(days) = parse_days(text) {
            args.insert("days".to_string(), json!(days));
        }
    }
    Some(Value::Object(args))
}

#[async_trait]
impl Tool for CurrencyRatesTool {
    fn name(&self) -> &'static str {
        "currency_rates"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["為替", "換算", "円", "ドル", "ユーロ", "usd", "jpy", "eur"]
    }

    fn intent_priority(&self) -> i32 {
        1_100
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        direct_query(text).map(crate::tools::IntentInvocation::direct)
    }

    fn label(&self) -> &'static str {
        "為替"
    }

    fn description(&self) -> String {
        "Frankfurter API で為替レート取得・通貨換算・推移チャート表示を行う。旅行、買い物、請求額確認、相場の推移確認に使う。APIキー不要。チャート（推移・グラフ）を求められたら chart=true を指定する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "base": { "type": "string", "description": "換算元通貨。例: USD, JPY, EUR。" },
                "symbols": { "type": "string", "description": "換算先通貨。複数はカンマ区切り。例: JPY,EUR。チャート時は先頭1通貨を使う。" },
                "amount": { "type": "number", "description": "換算金額。省略時1。" },
                "chart": { "type": "boolean", "description": "推移チャート（スパークライン）を出すなら true。" },
                "days": { "type": "integer", "description": "チャートの対象日数（2〜365、既定30）。指定するとチャートになる。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_CURRENCY_DEFAULT_BASE",
                "為替の既定換算元",
                "text",
                "USD",
            ),
            Field::new(
                "PICOAGENT_CURRENCY_DEFAULT_SYMBOLS",
                "為替の既定換算先",
                "text",
                "JPY",
            ),
            Field::int(&crate::config::ints::CURRENCY_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base = args
            .get("base")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or_else(|| {
                cfg.get("PICOAGENT_CURRENCY_DEFAULT_BASE", "USD")
                    .to_ascii_uppercase()
            });
        let symbols = args
            .get("symbols")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or_else(|| {
                cfg.get("PICOAGENT_CURRENCY_DEFAULT_SYMBOLS", "JPY")
                    .to_ascii_uppercase()
            });
        let amount = args.get("amount").and_then(Value::as_f64).unwrap_or(1.0);
        let timeout = crate::tool_try!(cfg.int(&crate::config::ints::CURRENCY_TIMEOUT_SECS)) as u64;
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Currency tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: 為替APIの準備に失敗しました: {}", error),
        };

        // チャート（推移）要求なら時系列を取得してスパークラインで返す。
        let want_chart = args.get("chart").and_then(Value::as_bool).unwrap_or(false)
            || args.get("days").is_some();
        if want_chart {
            let days = args
                .get("days")
                .and_then(Value::as_i64)
                .unwrap_or(30)
                .clamp(2, 365);
            let symbol = symbols
                .split(',')
                .map(str::trim)
                .find(|s| !s.is_empty())
                .unwrap_or("JPY")
                .to_string();
            return currency_chart(cfg, &client, &base, &symbol, days).await;
        }

        let response = match client
            .get(format!("{}/latest", FRANKFURTER_BASE))
            .query(&[
                ("from", base.as_str()),
                ("to", symbols.as_str()),
                ("amount", &amount.to_string()),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return format!(
                    "エラー: 為替APIへの接続に失敗しました: {}",
                    error.without_url()
                )
            }
        };
        let status = response.status();
        let body = crate::tool_response_text!(response);
        if !status.is_success() {
            return format!(
                "エラー: {}",
                crate::tools::http::safe_service_error(cfg, "為替API", status, &body)
            );
        }
        let data: Value = crate::tool_response_json!(&body);
        let date = data.get("date").and_then(Value::as_str).unwrap_or("");
        let Some(rates) = data.get("rates").and_then(Value::as_object) else {
            return body; // 想定外の形式はそのまま返す。
        };
        let mut lines: Vec<String> = rates
            .iter()
            .filter_map(|(sym, value)| {
                value
                    .as_f64()
                    .map(|rate| format!("{amount:.0} {base} = {rate:.3} {sym}"))
            })
            .collect();
        lines.sort();
        format!("為替レート（{date}）\n{}", lines.join("\n"))
    }
}

// Frankfurter の時系列を取得し、推移をスパークラインで返す。
async fn currency_chart(
    cfg: &Config,
    client: &reqwest::Client,
    base: &str,
    symbol: &str,
    days: i64,
) -> String {
    use chrono::{Duration as ChronoDuration, Local};
    let end = Local::now().date_naive();
    let start = end - ChronoDuration::days(days);
    let url = format!("{FRANKFURTER_BASE}/{start}..{end}");
    let response = match client
        .get(&url)
        .query(&[("from", base), ("to", symbol)])
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return format!(
                "エラー: 為替APIへの接続に失敗しました: {}",
                error.without_url()
            )
        }
    };
    if !response.status().is_success() {
        return crate::tools::http::service_reply(cfg, Ok(response), "為替API").await;
    }
    let data: Value = match response.json().await {
        Ok(data) => data,
        Err(error) => return format!("エラー: 為替APIの応答を読めませんでした: {}", error),
    };
    let Some(rates) = data.get("rates").and_then(Value::as_object) else {
        return "エラー: 為替の時系列を取得できませんでした。".to_string();
    };
    let mut series: Vec<(String, f64)> = rates
        .iter()
        .filter_map(|(date, value)| {
            value
                .get(symbol)
                .and_then(Value::as_f64)
                .map(|rate| (date.clone(), rate))
        })
        .collect();
    series.sort_by(|a, b| a.0.cmp(&b.0));
    if series.len() < 2 {
        return format!("エラー: {base}→{symbol} の推移データが不足しています。");
    }
    let values: Vec<f64> = series.iter().map(|(_, v)| *v).collect();
    let first = values.first().copied().unwrap_or(0.0);
    let last = values.last().copied().unwrap_or(0.0);
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let change = if first != 0.0 {
        (last - first) / first * 100.0
    } else {
        0.0
    };
    let sign = if change >= 0.0 { "+" } else { "" };
    format!(
        "{base}→{symbol} 直近{}日の推移（{}〜{}）\n{}\n始値 {:.3} / 終値 {:.3} / 最安 {:.3} / 最高 {:.3} / 変化 {sign}{:.2}%",
        days,
        series.first().map(|(d, _)| d.as_str()).unwrap_or(""),
        series.last().map(|(d, _)| d.as_str()).unwrap_or(""),
        crate::text::sparkline(&values),
        first,
        last,
        min,
        max,
        change,
    )
}

inventory::submit! { ToolReg { make: || Box::new(CurrencyRatesTool) } }
