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
}

fn extract_location(text: &str) -> Option<String> {
    let mut query = text.to_string();
    for word in [
        "天気", "天候", "気象", "気温", "降水確率", "降水", "予報", "weather", "明後日",
        "明日", "あした", "今日", "きょう", "現在", "いまの", "今の", "今", "この",
        "教えて", "おしえて", "ですか", "を", "は", "の", "が", "って", "?", "？", "。",
        "、", "!", "！", " ", "　",
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
        &["天気", "天候", "気象", "気温", "雨", "降水", "予報", "weather"]
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
            Field::new("PICOAGENT_WEATHER_DEFAULT_LOCATION", "天気の既定地点（自動取得に失敗した時に使用）", "text", "Tokyo"),
            Field::new("PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION", "現在地の自動取得を無効にする（既定は自動取得ON）", "checkbox", ""),
            Field::new("PICOAGENT_WEATHER_TIMEOUT_SECS", "天気API待ち時間（秒、3〜60）", "text", "15"),
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
        } else if let Some(found) = match &explicit_location {
            // 地名が明示された場合はそれを優先（解決できなければ下のフォールバックへ）。
            Some(location) => geocode(&client, location).await.ok(),
            None => None,
        } {
            found
        } else {
            // 地名が無い／解決失敗時は、まず IP から現在地を推定（有効時）。最後は既定地点。
            let auto = !cfg.get_bool("PICOAGENT_WEATHER_DISABLE_AUTO_LOCATION", false);
            match if auto { ip_geolocate(&client).await } else { None } {
                Some(found) => found,
                None => {
                    let location = cfg.get("PICOAGENT_WEATHER_DEFAULT_LOCATION", "Tokyo");
                    match geocode(&client, &location).await {
                        Ok(found) => found,
                        Err(error) => return format!("エラー: {}", error),
                    }
                }
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
        .query(&[("name", location), ("count", "1"), ("language", "ja"), ("format", "json")])
        .send()
        .await
        .map_err(|error| format!("地名検索に失敗しました: {}", error))?;
    let status = response.status();
    let body = crate::tools::http::response_text(response).await?;
    if !status.is_success() {
        return Err(format!("地名検索APIが HTTP {} を返しました。", status));
    }
    let parsed: Value = serde_json::from_str(&body).map_err(|error| format!("地名検索結果を解析できません: {}", error))?;
    let first = parsed
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| format!("地点が見つかりません: {}", location))?;
    let lat = first.get("latitude").and_then(Value::as_f64).ok_or_else(|| "緯度を取得できません。".to_string())?;
    let lon = first.get("longitude").and_then(Value::as_f64).ok_or_else(|| "経度を取得できません。".to_string())?;
    let name = first.get("name").and_then(Value::as_str).unwrap_or(location).to_string();
    Ok((lat, lon, name))
}

// IP から現在地（緯度経度・市名）を推定する。失敗時は None（既定地点へフォールバック）。
async fn ip_geolocate(client: &reqwest::Client) -> Option<(f64, f64, String)> {
    let response = client
        .get("http://ip-api.com/json/?fields=status,lat,lon,city,country")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let data: Value = response.json().await.ok()?;
    if data.get("status").and_then(Value::as_str) != Some("success") {
        return None;
    }
    let lat = data.get("lat").and_then(Value::as_f64)?;
    let lon = data.get("lon").and_then(Value::as_f64)?;
    let name = data
        .get("city")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{:.2},{:.2}", lat, lon));
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
    let code = current.get("weather_code").and_then(Value::as_i64).unwrap_or(-1);
    let (icon, label) = weather_icon(code);
    let num = |v: Option<f64>| v.map(|x| format!("{x:.0}")).unwrap_or_else(|| "-".to_string());
    let mut out = format!("天気: {name} {icon} {label}\n");
    out.push_str(&format!(
        "現在 {}°C（体感{}°C）湿度{}% 風{}km/h 降水{}mm\n",
        num(current.get("temperature_2m").and_then(Value::as_f64)),
        num(current.get("apparent_temperature").and_then(Value::as_f64)),
        current.get("relative_humidity_2m").and_then(Value::as_u64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
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
        let times = daily.get("time").and_then(Value::as_array).cloned().unwrap_or_default();
        let codes = daily.get("weather_code").and_then(Value::as_array).cloned().unwrap_or_default();
        let maxes = daily.get("temperature_2m_max").and_then(Value::as_array).cloned().unwrap_or_default();
        let mins = daily.get("temperature_2m_min").and_then(Value::as_array).cloned().unwrap_or_default();
        let pops = daily.get("precipitation_probability_max").and_then(Value::as_array).cloned().unwrap_or_default();
        out.push_str("予報:\n");
        for index in 0..times.len().min(7) {
            let (day_icon, day_label) = weather_icon(codes.get(index).and_then(Value::as_i64).unwrap_or(-1));
            out.push_str(&format!(
                "- {} {day_icon} {day_label} {}/{}°C ☔{}%\n",
                times.get(index).and_then(Value::as_str).unwrap_or("-"),
                num(maxes.get(index).and_then(Value::as_f64)),
                num(mins.get(index).and_then(Value::as_f64)),
                pops.get(index).and_then(Value::as_u64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())
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
