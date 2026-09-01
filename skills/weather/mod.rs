// skills/weather/mod.rs - Open-Meteo による天気取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";

pub struct WeatherTool;

const DIRECT_WEATHER_WORDS: [&str; 7] = ["天気", "気温", "降水", "予報", "天候", "気象", "weather"];

fn is_direct_query(text: &str) -> bool {
    let lower = text.to_lowercase();
    DIRECT_WEATHER_WORDS
        .iter()
        .any(|word| lower.contains(&word.to_lowercase()))
        && !is_weather_source_search(&lower)
}

/// 天気そのものではなく、天気を掲載するWebページを探している要求。
/// weatherを先に実行すると検索要求を横取りするため、ここでは明示的に辞退する。
fn is_weather_source_search(text: &str) -> bool {
    const SOURCE_WORDS: &[&str] = &[
        "サイト",
        "ページ",
        "記事",
        "リンク",
        "web",
        "ウェブ",
        "ネット",
    ];
    const SEARCH_WORDS: &[&str] = &["探して", "検索", "調べて"];
    SOURCE_WORDS.iter().any(|word| text.contains(word))
        && SEARCH_WORDS.iter().any(|word| text.contains(word))
}

fn configured_location(value: &str) -> Option<&str> {
    let location = value.trim();
    (!location.is_empty() && !location.eq_ignore_ascii_case("Tokyo")).then_some(location)
}

fn extract_location(text: &str) -> Option<String> {
    let mut query = text.to_string();
    for word in [
        "天気",
        "天候",
        "気象",
        "気温",
        "降水確率",
        "降水",
        "予報",
        "weather",
        "明後日",
        "明日",
        "あした",
        "今日",
        "きょう",
        "現在",
        "いまの",
        "今の",
        "今",
        "この",
        "教えて",
        "おしえて",
        "ですか",
        "を",
        "は",
        "の",
        "が",
        "って",
        "?",
        "？",
        "。",
        "、",
        "!",
        "！",
        " ",
        "　",
    ] {
        query = query.replace(word, "");
    }
    let query = query.trim();
    (query.chars().count() >= 2).then(|| query.to_string())
}

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &'static str {
        "weather"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "天気", "天候", "気象", "気温", "雨", "降水", "予報", "weather",
        ]
    }

    fn intent_priority(&self) -> i32 {
        1_200
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        if !is_direct_query(text) {
            return None;
        }
        let args = match extract_location(text) {
            Some(location) => json!({ "location": location }),
            None => json!({}),
        };
        Some(crate::tools::IntentInvocation::direct(args))
    }

    fn label(&self) -> &'static str {
        "天気"
    }

    fn description(&self) -> String {
        "Open-Meteo で現在の天気と予報を取得する。天気、気温、降水確率など普段使いの確認に使う。APIキー不要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "location": { "type": "string", "description": "地名。例: Tokyo, Osaka, Sapporo。" },
                "latitude": { "type": "number", "description": "緯度。location の代わりに指定可能。" },
                "longitude": { "type": "number", "description": "経度。location の代わりに指定可能。" },
                "days": { "type": "integer", "minimum": 1, "maximum": 7, "description": "日別予報の日数。省略時3日。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_WEATHER_DEFAULT_LOCATION",
                "天気の地点（市区町村・都道府県。空欄なら現在地を自動取得）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION",
                "現在地の自動取得を無効にする（既定は自動取得ON）",
                "checkbox",
                "",
            ),
            Field::new(
                "PICOAGENT_WEATHER_TIMEOUT_SECS",
                "天気API待ち時間（秒、3〜60）",
                "text",
                "15",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let timeout = cfg
            .get("PICOAGENT_WEATHER_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Weather tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: 天気APIの準備に失敗しました: {}", error),
        };
        let days = args
            .get("days")
            .and_then(Value::as_u64)
            .filter(|value| (1..=7).contains(value))
            .unwrap_or(3)
            .to_string();
        let explicit_location = args
            .get("location")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let (lat, lon, name) = if let (Some(lat), Some(lon)) = (
            args.get("latitude").and_then(Value::as_f64),
            args.get("longitude").and_then(Value::as_f64),
        ) {
            (lat, lon, format!("{},{}", lat, lon))
        } else if let Some(location) = &explicit_location {
            // 利用者が明示した地点を別地点へ黙って置き換えない。
            match geocode(&client, location).await {
                Ok(found) => found,
                Err(error) => return format!("エラー: {}", error),
            }
        } else {
            // 保存済み地点を正本にする。未設定の場合だけIP現在地を初期候補として使う。
            let saved_location = cfg.get("PICOAGENT_WEATHER_DEFAULT_LOCATION", "");
            // 旧版のスキーマ既定値 `Tokyo` は利用者が選んだ地点と区別できないため、
            // 新版では未設定として一度だけ現在地推定へ戻す。東京を固定したい場合は
            // 日本語の「東京都」等を保存すれば正本として扱われる。
            if let Some(saved_location) = configured_location(&saved_location) {
                match geocode(&client, saved_location).await {
                    Ok(found) => found,
                    Err(error) => return format!("エラー: {}", error),
                }
            } else if !cfg.get_bool("PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION", false) {
                match ip_geolocate(&client).await {
                    Some(found) => found,
                    None => return "エラー: 現在地を取得できませんでした。設定で天気の地点を入力してください。".to_string(),
                }
            } else {
                return "エラー: 天気の地点が未設定です。設定で市区町村または都道府県を入力してください。".to_string();
            }
        };
        let response = match client
            .get(FORECAST_URL)
            .query(&[
                ("latitude", lat.to_string()),
                ("longitude", lon.to_string()),
                ("current", "temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m".to_string()),
                ("hourly", "temperature_2m".to_string()),
                ("daily", "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max".to_string()),
                ("forecast_days", days),
                ("timezone", "auto".to_string()),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return format!("エラー: 天気APIへの接続に失敗しました: {}", error),
        };
        let status = response.status();
        let body = crate::tool_response_text!(response);
        if !status.is_success() {
            return format!("エラー: 天気APIが HTTP {} を返しました。", status);
        }
        let parsed: Value = crate::tool_response_json!(&body);
        summarize_weather(&name, &parsed)
    }
}

async fn geocode(client: &reqwest::Client, location: &str) -> Result<(f64, f64, String), String> {
    let response = client
        .get(GEOCODE_URL)
        .query(&[
            ("name", location),
            ("count", "1"),
            ("language", "ja"),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|error| format!("地名検索に失敗しました: {}", error))?;
    let status = response.status();
    let body = crate::tools::http::response_text(response).await?;
    if !status.is_success() {
        return Err(format!("地名検索APIが HTTP {} を返しました。", status));
    }
    let parsed: Value = serde_json::from_str(&body)
        .map_err(|error| format!("地名検索結果を解析できません: {}", error))?;
    let first = parsed
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| format!("地点が見つかりません: {}", location))?;
    let lat = first
        .get("latitude")
        .and_then(Value::as_f64)
        .ok_or_else(|| "緯度を取得できません。".to_string())?;
    let lon = first
        .get("longitude")
        .and_then(Value::as_f64)
        .ok_or_else(|| "経度を取得できません。".to_string())?;
    let name = first
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(location)
        .to_string();
    Ok((lat, lon, name))
}

// IP から現在地（緯度経度・市名）をHTTPSで推定する。失敗時は None。
async fn ip_geolocate(client: &reqwest::Client) -> Option<(f64, f64, String)> {
    let response = client.get("https://ipconfig.io/json").send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let data: Value = response.json().await.ok()?;
    parse_ip_location(&data)
}

fn parse_ip_location(data: &Value) -> Option<(f64, f64, String)> {
    let lat = data.get("latitude").and_then(Value::as_f64)?;
    let lon = data.get("longitude").and_then(Value::as_f64)?;
    let name = data
        .get("city")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            data.get("region_name")
                .and_then(Value::as_str)
                .filter(|region| !region.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| format!("{lat:.2},{lon:.2}"));
    Some((lat, lon, name))
}

// WMO 天気コードを絵文字と日本語ラベルへ。
fn weather_icon(code: i64) -> (&'static str, &'static str) {
    match code {
        0 => ("☀️", "快晴"),
        1 => ("🌤️", "晴れ"),
        2 => ("⛅", "薄曇り"),
        3 => ("☁️", "曇り"),
        45 | 48 => ("🌫️", "霧"),
        51 | 53 | 55 | 56 | 57 => ("🌦️", "霧雨"),
        61 | 63 | 65 | 66 | 67 => ("🌧️", "雨"),
        71 | 73 | 75 | 77 => ("❄️", "雪"),
        80..=82 => ("🌦️", "にわか雨"),
        85 | 86 => ("🌨️", "にわか雪"),
        95 | 96 | 99 => ("⛈️", "雷雨"),
        _ => ("🌡️", "—"),
    }
}

// 数値列を Unicode ブロックのスパークラインにする。
fn sparkline(values: &[f64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if max.partial_cmp(&min) != Some(std::cmp::Ordering::Greater) {
        return BARS[3].to_string().repeat(values.len());
    }
    values
        .iter()
        .map(|&v| {
            let idx = (((v - min) / (max - min)) * 7.0).round() as usize;
            BARS[idx.min(7)]
        })
        .collect()
}

fn summarize_weather(name: &str, parsed: &Value) -> String {
    let current = parsed.get("current").unwrap_or(&Value::Null);
    let code = current
        .get("weather_code")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let (icon, label) = weather_icon(code);
    let num = |v: Option<f64>| {
        v.map(|x| format!("{x:.0}"))
            .unwrap_or_else(|| "-".to_string())
    };
    let mut out = format!("天気: {name} {icon} {label}\n");
    out.push_str(&format!(
        "現在 {}°C（体感{}°C）湿度{}% 風{}km/h 降水{}mm\n",
        num(current.get("temperature_2m").and_then(Value::as_f64)),
        num(current.get("apparent_temperature").and_then(Value::as_f64)),
        current
            .get("relative_humidity_2m")
            .and_then(Value::as_u64)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        num(current.get("wind_speed_10m").and_then(Value::as_f64)),
        num(current.get("precipitation").and_then(Value::as_f64)),
    ));

    // 今日の気温推移（時間別の先頭24時間）をスパークラインで。
    if let Some(temps) = parsed
        .get("hourly")
        .and_then(|h| h.get("temperature_2m"))
        .and_then(Value::as_array)
    {
        let today: Vec<f64> = temps.iter().take(24).filter_map(Value::as_f64).collect();
        if today.len() >= 2 {
            let lo = today.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = today.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            out.push_str(&format!(
                "今日の気温 {:.0}〜{:.0}°C  {}\n",
                lo,
                hi,
                sparkline(&today)
            ));
        }
    }

    if let Some(daily) = parsed.get("daily") {
        let times = daily
            .get("time")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let codes = daily
            .get("weather_code")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let maxes = daily
            .get("temperature_2m_max")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mins = daily
            .get("temperature_2m_min")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let pops = daily
            .get("precipitation_probability_max")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        out.push_str("予報:\n");
        for index in 0..times.len().min(7) {
            let (day_icon, day_label) =
                weather_icon(codes.get(index).and_then(Value::as_i64).unwrap_or(-1));
            out.push_str(&format!(
                "- {} {day_icon} {day_label} {}/{}°C ☔{}%\n",
                times.get(index).and_then(Value::as_str).unwrap_or("-"),
                num(maxes.get(index).and_then(Value::as_f64)),
                num(mins.get(index).and_then(Value::as_f64)),
                pops.get(index)
                    .and_then(Value::as_u64)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
        }
        // 日別の最高気温の推移をスパークラインで添える。
        let highs: Vec<f64> = maxes.iter().take(7).filter_map(Value::as_f64).collect();
        if highs.len() >= 2 {
            out.push_str(&format!("最高気温の推移 {}\n", sparkline(&highs)));
        }
    }
    out
}

inventory::submit! { ToolReg { make: || Box::new(WeatherTool) } }

#[cfg(test)]
mod tests {
    use super::{
        configured_location, extract_location, is_direct_query, is_weather_source_search,
        parse_ip_location,
    };

    #[test]
    fn 天気そのものの質問だけを直接実行する() {
        assert!(is_direct_query("神奈川県の天気を教えて"));
        assert_eq!(
            extract_location("神奈川県の天気を教えて").as_deref(),
            Some("神奈川県")
        );
    }

    #[test]
    fn 天気サイト検索はweatherが横取りしない() {
        for text in [
            "天気のサイト探して",
            "天気予報のページを検索して",
            "気象の記事をWebで調べて",
        ] {
            assert!(is_weather_source_search(&text.to_lowercase()));
            assert!(!is_direct_query(text));
        }
    }

    #[test]
    fn 旧tokyo既定値は未設定へ移し日本語の東京都は保持する() {
        assert_eq!(configured_location("Tokyo"), None);
        assert_eq!(configured_location(" tokyo "), None);
        assert_eq!(configured_location("東京都"), Some("東京都"));
        assert_eq!(configured_location("神奈川県"), Some("神奈川県"));
    }

    #[test]
    fn https位置情報の緯度経度と表示名を読む() {
        let value = serde_json::json!({
            "latitude": 35.44,
            "longitude": 139.64,
            "city": "Yokohama",
            "region_name": "Kanagawa"
        });
        assert_eq!(
            parse_ip_location(&value),
            Some((35.44, 139.64, "Yokohama".to_string()))
        );
    }
}
