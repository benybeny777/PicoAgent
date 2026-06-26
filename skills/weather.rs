// tools/weather.rs - Open-Meteo による天気取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";

pub struct WeatherTool;

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &'static str {
        "weather"
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
            Field::new("PICOAGENT_WEATHER_DEFAULT_LOCATION", "天気の既定地点", "text", "Tokyo"),
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
        let (lat, lon, name) = if let (Some(lat), Some(lon)) = (
            args.get("latitude").and_then(Value::as_f64),
            args.get("longitude").and_then(Value::as_f64),
        ) {
            (lat, lon, format!("{},{}", lat, lon))
        } else {
            let location = args
                .get("location")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| cfg.get("PICOAGENT_WEATHER_DEFAULT_LOCATION", "Tokyo"));
            match geocode(&client, &location).await {
                Ok(found) => found,
                Err(error) => return format!("エラー: {}", error),
            }
        };
        let response = match client
            .get(FORECAST_URL)
            .query(&[
                ("latitude", lat.to_string()),
                ("longitude", lon.to_string()),
                ("current", "temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m".to_string()),
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
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return format!("エラー: 天気APIが HTTP {} を返しました。", status);
        }
        let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
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
    let body = response.text().await.unwrap_or_default();
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

fn summarize_weather(name: &str, parsed: &Value) -> String {
    let current = parsed.get("current").unwrap_or(&Value::Null);
    let mut out = format!("天気: {}\n", name);
    out.push_str(&format!(
        "現在: 気温 {}°C / 体感 {}°C / 湿度 {}% / 降水 {}mm / 風速 {}km/h / 天気コード {}\n",
        current.get("temperature_2m").and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        current.get("apparent_temperature").and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        current.get("relative_humidity_2m").and_then(Value::as_u64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        current.get("precipitation").and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        current.get("wind_speed_10m").and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        current.get("weather_code").and_then(Value::as_i64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())
    ));
    if let Some(daily) = parsed.get("daily") {
        let times = daily.get("time").and_then(Value::as_array).cloned().unwrap_or_default();
        let maxes = daily.get("temperature_2m_max").and_then(Value::as_array).cloned().unwrap_or_default();
        let mins = daily.get("temperature_2m_min").and_then(Value::as_array).cloned().unwrap_or_default();
        let pops = daily.get("precipitation_probability_max").and_then(Value::as_array).cloned().unwrap_or_default();
        out.push_str("予報:\n");
        for index in 0..times.len().min(7) {
            out.push_str(&format!(
                "- {}: {}°C / {}°C / 降水確率 {}%\n",
                times.get(index).and_then(Value::as_str).unwrap_or("-"),
                maxes.get(index).and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
                mins.get(index).and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
                pops.get(index).and_then(Value::as_u64).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())
            ));
        }
    }
    out
}

inventory::submit! { ToolReg { make: || Box::new(WeatherTool) } }
