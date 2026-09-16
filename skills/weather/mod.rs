// skills/weather/mod.rs - Open-Meteo による天気取得スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

/// 既定地点の設定キー。保存とロック名で同じ文字列を使う。
const DEFAULT_LOCATION_KEY: &str = "PICOAGENT_WEATHER_DEFAULT_LOCATION";
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

/// 都道府県名だけを言われたときに使う代表地点（県庁所在地）。
/// Open-Meteo の地名検索は「神奈川県」のような都道府県名では候補を1件も返さないため、
/// ここで市へ読み替える。読み替えた事実は表示名（「横浜市（神奈川県）」）から分かる。
const PREFECTURE_CAPITALS: &[(&str, &str)] = &[
    ("北海道", "札幌市"),
    ("青森県", "青森市"),
    ("岩手県", "盛岡市"),
    ("宮城県", "仙台市"),
    ("秋田県", "秋田市"),
    ("山形県", "山形市"),
    ("福島県", "福島市"),
    ("茨城県", "水戸市"),
    ("栃木県", "宇都宮市"),
    ("群馬県", "前橋市"),
    ("埼玉県", "さいたま市"),
    ("千葉県", "千葉市"),
    ("東京都", "新宿区"),
    ("神奈川県", "横浜市"),
    ("新潟県", "新潟市"),
    ("富山県", "富山市"),
    ("石川県", "金沢市"),
    ("福井県", "福井市"),
    ("山梨県", "甲府市"),
    ("長野県", "長野市"),
    ("岐阜県", "岐阜市"),
    ("静岡県", "静岡市"),
    ("愛知県", "名古屋市"),
    ("三重県", "津市"),
    ("滋賀県", "大津市"),
    ("京都府", "京都市"),
    ("大阪府", "大阪市"),
    ("兵庫県", "神戸市"),
    ("奈良県", "奈良市"),
    ("和歌山県", "和歌山市"),
    ("鳥取県", "鳥取市"),
    ("島根県", "松江市"),
    ("岡山県", "岡山市"),
    ("広島県", "広島市"),
    ("山口県", "山口市"),
    ("徳島県", "徳島市"),
    ("香川県", "高松市"),
    ("愛媛県", "松山市"),
    ("高知県", "高知市"),
    ("福岡県", "福岡市"),
    ("佐賀県", "佐賀市"),
    ("長崎県", "長崎市"),
    ("熊本県", "熊本市"),
    ("大分県", "大分市"),
    ("宮崎県", "宮崎市"),
    ("鹿児島県", "鹿児島市"),
    ("沖縄県", "那覇市"),
];

/// 都道府県名なら県庁所在地へ読み替える。市区町村名はそのまま返す。
fn resolvable_location(location: &str) -> &str {
    let trimmed = location.trim();
    PREFECTURE_CAPITALS
        .iter()
        .find(|(prefecture, _)| *prefecture == trimmed)
        .map(|(_, capital)| *capital)
        .unwrap_or(trimmed)
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
    // 「今」は地名の一部にもなる（今治市・今別町など）。落として2文字以上残るときだけ
    // 時刻の言い回しとして外す。無条件に外すと「今治の天気」が地名なしの依頼になり、
    // 保存済みの地点や現在地の天気を、聞かれた街の天気として黙って返してしまう。
    let without_now = query.replace('今', "");
    if without_now.trim().chars().count() >= 2 {
        query = without_now;
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
                "例: 横浜市（一覧から選ぶか入力。会話で地名を言うと自動で保存されます）",
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
                Ok(found) => {
                    // 会話で言われた地点は既定として覚え、次から地名を聞き直さない。
                    // 「読んで比べてから書き戻す」ので、同時に複数の問い合わせが走っても
                    // 片方の保存が消えないよう名前付きロックの中で行う（await を挟まない）。
                    // 保存に失敗しても今回の天気は返す（覚えられないだけで回答は作れる）。
                    {
                        let _guard = crate::store::lock_state(DEFAULT_LOCATION_KEY);
                        if cfg.get(DEFAULT_LOCATION_KEY, "") != *location {
                            if let Err(error) =
                                cfg.set_string(DEFAULT_LOCATION_KEY, location.clone())
                            {
                                eprintln!("天気の既定地点を保存できませんでした: {error}");
                            }
                        }
                    }
                    found
                }
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
                // 自動取得を切っているために地点が無い、という理由まで伝える。
                // 「未設定です」だけだと、利用者は設定で自分が無効にしたことへ到達できない。
                return "エラー: 天気の地点が未設定で、現在地の自動取得も設定で無効になっています。設定で市区町村を入力するか、現在地の自動取得を有効にしてください。".to_string();
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

/// 候補名が、利用者の言った地名と同じ場所を指しているか。
///
/// Open-Meteo は別名・読み・近い綴りにも一致するため、候補には言われていない地名が
/// 混ざる。漢字の地名では「今治」と「今治市」のように一方がもう一方を含む形になるので、
/// その関係があるものだけを「言われた地名」として扱う。かな・ローマ字で尋ねた場合は
/// どちらにも当てはまらないため、この判定では絞らず人口順の選び方へ任せる。
fn names_same_place(query: &str, name: &str) -> bool {
    let query = query.trim();
    let name = name.trim();
    !query.is_empty() && !name.is_empty() && (name.contains(query) || query.contains(name))
}

/// 候補の中から実際に使う地点を選ぶ。
///
/// Open-Meteo の地名検索は一致順で返すため、「横浜」では青森県横浜町（人口4,412）が
/// 先頭に来て、横浜市の天気を聞いたのに別の町の天気を返していた。利用者が地名だけを
/// 言うときは大きい自治体を指すことがほとんどなので、人口の多い候補を選ぶ。
/// ただし**人口だけで選ぶと、言われていない大都市が言われた小さな街に勝つ**ため、
/// 名前が一致する候補を先に立てる（一致が1件も無いときだけ従来どおり人口順）。
/// 同じ地名が複数県にあることを利用者が確かめられるよう、表示名へ都道府県を添える。
fn pick_location(query: &str, candidates: &[Value]) -> Option<(f64, f64, String)> {
    let best = candidates.iter().max_by_key(|item| {
        let matched = item
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| names_same_place(query, name));
        (
            matched,
            item.get("population").and_then(Value::as_u64).unwrap_or(0),
        )
    })?;
    let lat = best.get("latitude").and_then(Value::as_f64)?;
    let lon = best.get("longitude").and_then(Value::as_f64)?;
    let name = best
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let label = match best.get("admin1").and_then(Value::as_str) {
        Some(area) if !area.is_empty() && area != name => format!("{name}（{area}）"),
        _ => name,
    };
    Some((lat, lon, label))
}

async fn fetch_candidates(client: &reqwest::Client, name: &str) -> Result<Vec<Value>, String> {
    let response = client
        .get(GEOCODE_URL)
        .query(&[
            ("name", name),
            ("count", "10"),
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
    Ok(parsed
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

async fn geocode(client: &reqwest::Client, location: &str) -> Result<(f64, f64, String), String> {
    // 都道府県名は候補が返らないので県庁所在地へ読み替える。
    let location = resolvable_location(location);
    let mut candidates = fetch_candidates(client, location).await?;
    // 「横浜」のように自治体の種別を省いた言い方では、小さな同名の町が先に来る。
    // 「市」を補った候補も集めてから人口で選ぶ。
    if !location.ends_with(['市', '区', '町', '村']) {
        if let Ok(extra) = fetch_candidates(client, &format!("{location}市")).await {
            candidates.extend(extra);
        }
    }
    pick_location(location, &candidates)
        .ok_or_else(|| format!("地点が見つかりません: {location}。市区町村まで指定してください。"))
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
    #[test]
    fn 都道府県名は県庁所在地へ読み替える() {
        // Open-Meteo の地名検索は都道府県名では候補を1件も返さない。
        assert_eq!(super::resolvable_location("神奈川県"), "横浜市");
        assert_eq!(super::resolvable_location(" 大阪府 "), "大阪市");
        assert_eq!(super::resolvable_location("東京都"), "新宿区");
        // 市区町村はそのまま使う。
        assert_eq!(super::resolvable_location("横浜市"), "横浜市");
        assert_eq!(super::resolvable_location("横浜"), "横浜");
    }

    #[test]
    fn 同名の地点では人口の多い方を選ぶ() {
        // 「横浜」は青森県横浜町（人口4,412）が先に返るが、利用者が指すのは横浜市。
        let candidates = vec![
            serde_json::json!({"name":"横浜","latitude":41.08,"longitude":141.25,"population":4412,"admin1":"青森県"}),
            serde_json::json!({"name":"横浜市","latitude":35.43,"longitude":139.65,"population":3777491,"admin1":"神奈川県"}),
        ];
        let (lat, _, label) = super::pick_location("横浜", &candidates).expect("地点を選べる");
        assert_eq!(label, "横浜市（神奈川県）");
        assert!((lat - 35.43).abs() < 0.01);
    }

    #[test]
    fn 言われていない大都市を人口だけで選ばない() {
        // 別名一致などで混ざった無関係の大都市が、言われた小さな街に勝たないこと。
        // 選んだ地点は既定として保存されるため、ここで取り違えると次回以降も引きずる。
        let candidates = vec![
            serde_json::json!({"name":"松山市","latitude":33.83,"longitude":132.76,"population":505000,"admin1":"愛媛県"}),
            serde_json::json!({"name":"今治市","latitude":34.06,"longitude":132.99,"population":150000,"admin1":"愛媛県"}),
        ];
        let (_, _, label) = super::pick_location("今治", &candidates).expect("地点を選べる");
        assert_eq!(label, "今治市（愛媛県）");
    }

    #[test]
    fn 名前で一致しない尋ね方では人口順のままにする() {
        // かな・ローマ字では候補名と文字が重ならない。ここで絞ると選べなくなるため、
        // 一致が1件も無いときは従来どおり人口の多い候補を選ぶ。
        let candidates = vec![
            serde_json::json!({"name":"横浜","latitude":41.08,"longitude":141.25,"population":4412,"admin1":"青森県"}),
            serde_json::json!({"name":"横浜市","latitude":35.43,"longitude":139.65,"population":3777491,"admin1":"神奈川県"}),
        ];
        let (_, _, label) = super::pick_location("よこはま", &candidates).expect("地点を選べる");
        assert_eq!(label, "横浜市（神奈川県）");
    }

    #[test]
    fn 人口不明の候補しかなくても選べる() {
        let candidates = vec![
            serde_json::json!({"name":"神奈川","latitude":35.47,"longitude":139.62,"admin1":"神奈川県"}),
        ];
        let (_, _, label) = super::pick_location("神奈川", &candidates).expect("地点を選べる");
        assert_eq!(label, "神奈川（神奈川県）");
    }

    #[test]
    fn 候補が空なら選ばない() {
        assert!(super::pick_location("横浜", &[]).is_none());
    }

    use super::{
        configured_location, extract_location, is_direct_query, is_weather_source_search,
        parse_ip_location,
    };

    #[test]
    fn 地名に含まれる今を時刻の言い回しとして消さない() {
        // 「今」を無条件に外すと今治が1文字になり、別の地点の天気を黙って返していた。
        assert_eq!(extract_location("今治の天気は？").as_deref(), Some("今治"));
        assert_eq!(
            extract_location("今、東京の天気を教えて").as_deref(),
            Some("東京")
        );
        assert_eq!(extract_location("今日の天気"), None);
        assert_eq!(extract_location("今の天気"), None);
    }

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
