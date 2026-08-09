// productivity_tools/mod.rs - すぐ使えるローカル実用ツール集。

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;

const MAX_TEXT_CHARS: usize = 20_000;

pub struct ProductivityTools;

fn text_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} が指定されていません。"))?;
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!(
            "{key} は {MAX_TEXT_CHARS} 文字以内で指定してください。"
        ));
    }
    Ok(text)
}

fn text_stats(text: &str) -> String {
    let chars = text.chars().count();
    let bytes = text.len();
    let lines = if text.is_empty() {
        0
    } else {
        text.lines().count()
    };
    let words = text.split_whitespace().count();
    format!("文字数: {chars}\nバイト数: {bytes}\n行数: {lines}\n単語数: {words}")
}

fn line_stats(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let non_empty = lines.iter().filter(|line| !line.trim().is_empty()).count();
    let max_len = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    format!(
        "行数: {}\n空でない行: {non_empty}\n最長行: {max_len} 文字",
        lines.len()
    )
}

fn to_title_case(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn reverse_text(text: &str) -> String {
    text.chars().rev().collect()
}

fn sort_lines(text: &str, descending: bool) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_by_key(|line| line.to_lowercase());
    if descending {
        lines.reverse();
    }
    lines.join("\n")
}

fn unique_lines(text: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .filter(|line| seen.insert(line.trim().to_string()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn trim_lines(text: &str) -> String {
    text.lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

fn wrap_text(text: &str, width: usize) -> String {
    let width = width.clamp(10, 200);
    let mut out = String::new();
    let mut line_len = 0usize;
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if line_len > 0 && line_len + 1 + word_len > width {
            out.push('\n');
            line_len = 0;
        } else if line_len > 0 {
            out.push(' ');
            line_len += 1;
        }
        out.push_str(word);
        line_len += word_len;
    }
    out
}

fn extract_urls(text: &str) -> String {
    text.split_whitespace()
        .filter_map(|token| {
            let trimmed = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | '。'
                )
            });
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_emails(text: &str) -> String {
    text.split_whitespace()
        .filter_map(|token| {
            let trimmed = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | '。'
                )
            });
            if trimmed.contains('@') && trimmed.rsplit_once('.').is_some() {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn keyword_count(text: &str, keyword: &str) -> Result<String, String> {
    if keyword.is_empty() {
        return Err("keyword が指定されていません。".to_string());
    }
    let count = text.matches(keyword).count();
    Ok(format!("{keyword}: {count} 件"))
}

fn slug_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn text_case(text: &str, mode: &str) -> Result<String, String> {
    match mode {
        "upper" => Ok(text.to_uppercase()),
        "lower" => Ok(text.to_lowercase()),
        "title" => Ok(to_title_case(text)),
        "snake" => Ok(slug_words(text).join("_")),
        "kebab" | "slug" => Ok(slug_words(text).join("-")),
        _ => Err(
            "mode は upper/lower/title/snake/kebab/slug のいずれかを指定してください。".to_string(),
        ),
    }
}

fn json_format(text: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("JSONの解析に失敗しました: {error}"))?;
    serde_json::to_string_pretty(&value).map_err(|error| format!("JSON整形に失敗しました: {error}"))
}

fn json_minify(text: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("JSONの解析に失敗しました: {error}"))?;
    serde_json::to_string(&value).map_err(|error| format!("JSON圧縮に失敗しました: {error}"))
}

fn url_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

fn url_decode(text: &str) -> String {
    url::form_urlencoded::parse(text.as_bytes())
        .map(|(key, value)| {
            if value.is_empty() {
                key.into_owned()
            } else {
                format!("{key}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn fnv1a_checksum(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn csv_to_markdown(text: &str) -> String {
    let mut rows: Vec<Vec<String>> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split(',')
                .map(|cell| cell.trim().replace('|', "｜"))
                .collect()
        })
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    let mut out = String::new();
    out.push_str("| ");
    out.push_str(&rows[0].join(" | "));
    out.push_str(" |\n| ");
    out.push_str(&vec!["---"; width].join(" | "));
    out.push_str(" |\n");
    for row in rows.iter().skip(1) {
        out.push_str("| ");
        out.push_str(&row.join(" | "));
        out.push_str(" |\n");
    }
    out
}

fn unit_convert(value: f64, from: &str, to: &str) -> Result<String, String> {
    let normalized_from = from.trim().to_lowercase();
    let normalized_to = to.trim().to_lowercase();
    let result = match (normalized_from.as_str(), normalized_to.as_str()) {
        ("c", "f") | ("celsius", "fahrenheit") => value * 9.0 / 5.0 + 32.0,
        ("f", "c") | ("fahrenheit", "celsius") => (value - 32.0) * 5.0 / 9.0,
        ("km", "mile") | ("km", "mi") => value * 0.621_371,
        ("mile", "km") | ("mi", "km") => value / 0.621_371,
        ("m", "ft") | ("meter", "feet") => value * 3.280_84,
        ("ft", "m") | ("feet", "meter") => value / 3.280_84,
        ("kg", "lb") | ("kg", "lbs") => value * 2.204_62,
        ("lb", "kg") | ("lbs", "kg") => value / 2.204_62,
        ("g", "oz") => value * 0.035_274,
        ("oz", "g") => value / 0.035_274,
        _ => return Err("対応単位は c/f, km/mi, m/ft, kg/lb, g/oz です。".to_string()),
    };
    Ok(format!("{value} {from} = {:.4} {to}", result))
}

fn percent_calc(value: f64, total: f64) -> Result<String, String> {
    if total == 0.0 {
        return Err("total は 0 以外を指定してください。".to_string());
    }
    Ok(format!("{value} / {total} = {:.2}%", value / total * 100.0))
}

fn bmi_calc(weight_kg: f64, height_cm: f64) -> Result<String, String> {
    if weight_kg <= 0.0 || height_cm <= 0.0 {
        return Err("weight_kg と height_cm は正の数で指定してください。".to_string());
    }
    let height_m = height_cm / 100.0;
    Ok(format!("BMI: {:.2}", weight_kg / (height_m * height_m)))
}

fn date_calc(args: &Value) -> Result<String, String> {
    let base = args
        .get("date")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|error| format!("date は YYYY-MM-DD で指定してください: {error}"))
        })
        .unwrap_or_else(|| Ok(Local::now().date_naive()))?;
    let days = args.get("days").and_then(Value::as_i64).unwrap_or(0);
    let result = base + Duration::days(days);
    Ok(format!(
        "{} から {days} 日: {}",
        base.format("%Y-%m-%d"),
        result.format("%Y-%m-%d")
    ))
}

fn date_diff(args: &Value) -> Result<String, String> {
    let from = args
        .get("from_date")
        .or_else(|| args.get("from"))
        .and_then(Value::as_str)
        .ok_or_else(|| "from_date が指定されていません。".to_string())
        .and_then(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|error| format!("from_date は YYYY-MM-DD で指定してください: {error}"))
        })?;
    let to = args
        .get("to_date")
        .or_else(|| args.get("to"))
        .and_then(Value::as_str)
        .ok_or_else(|| "to_date が指定されていません。".to_string())
        .and_then(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|error| format!("to_date は YYYY-MM-DD で指定してください: {error}"))
        })?;
    Ok(format!(
        "{} から {} まで: {} 日",
        from,
        to,
        (to - from).num_days()
    ))
}

fn week_number(args: &Value) -> Result<String, String> {
    let date = args
        .get("date")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|error| format!("date は YYYY-MM-DD で指定してください: {error}"))
        })
        .unwrap_or_else(|| Ok(Local::now().date_naive()))?;
    let iso = date.iso_week();
    Ok(format!(
        "{} は ISO週 {}-W{:02}",
        date,
        iso.year(),
        iso.week()
    ))
}

fn timestamp_now() -> String {
    let now = Local::now();
    format!("unix: {}\niso: {}", now.timestamp(), now.to_rfc3339())
}

fn timestamp_to_date(value: i64) -> Result<String, String> {
    let dt: DateTime<Local> = Local
        .timestamp_opt(value, 0)
        .single()
        .ok_or_else(|| "timestamp を日付に変換できません。".to_string())?;
    Ok(dt.to_rfc3339())
}

fn color_hex_to_rgb(text: &str) -> Result<String, String> {
    let text = text.trim();
    let hex = text.strip_prefix('#').unwrap_or(text);
    // `len()` はバイト長なので、16進数字であることまで確かめる。3バイト文字2つの
    // 「あい」でも長さ6を満たすため、この検査が無いとバイト添字が文字の途中に落ちる。
    if hex.len() != 6 || !hex.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("hex は #RRGGBB 形式（16進6桁）で指定してください。".to_string());
    }
    let mut channels = [0_u8; 3];
    for (index, channel) in channels.iter_mut().enumerate() {
        let pair: String = hex.chars().skip(index * 2).take(2).collect();
        *channel = u8::from_str_radix(&pair, 16)
            .map_err(|_| format!("{}値を解釈できません。", ["R", "G", "B"][index]))?;
    }
    let [r, g, b] = channels;
    Ok(format!("rgb({r}, {g}, {b})"))
}

fn color_rgb_to_hex(args: &Value) -> Result<String, String> {
    let r = args
        .get("r")
        .and_then(Value::as_u64)
        .ok_or_else(|| "r が指定されていません。".to_string())?;
    let g = args
        .get("g")
        .and_then(Value::as_u64)
        .ok_or_else(|| "g が指定されていません。".to_string())?;
    let b = args
        .get("b")
        .and_then(Value::as_u64)
        .ok_or_else(|| "b が指定されていません。".to_string())?;
    if r > 255 || g > 255 || b > 255 {
        return Err("r/g/b は 0〜255 で指定してください。".to_string());
    }
    Ok(format!("#{r:02X}{g:02X}{b:02X}"))
}

#[async_trait]
impl Tool for ProductivityTools {
    fn name(&self) -> &'static str {
        "productivity_tools"
    }

    fn label(&self) -> &'static str {
        "実用ツール集"
    }

    fn description(&self) -> String {
        "ローカルで文字列分析、JSON整形、Base64、URL変換、単位換算、日付計算、CSV表化、色変換など30種類の小さな実用処理を行う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "text_stats",
                        "text_case",
                        "json_format",
                        "json_minify",
                        "base64_encode",
                        "base64_decode",
                        "url_encode",
                        "url_decode",
                        "unit_convert",
                        "date_calc",
                        "date_diff",
                        "week_number",
                        "timestamp_now",
                        "timestamp_to_date",
                        "line_stats",
                        "reverse_text",
                        "sort_lines",
                        "unique_lines",
                        "trim_lines",
                        "wrap_text",
                        "extract_urls",
                        "extract_emails",
                        "keyword_count",
                        "checksum",
                        "csv_to_markdown",
                        "percent_calc",
                        "bmi_calc",
                        "color_hex_to_rgb",
                        "color_rgb_to_hex",
                        "slugify"
                    ],
                    "description": "実行する機能。"
                },
                "text": { "type": "string", "description": "対象テキスト。" },
                "mode": { "type": "string", "description": "text_case の mode。upper/lower/title/snake/kebab/slug。" },
                "value": { "type": "number", "description": "unit_convert の数値。" },
                "total": { "type": "number", "description": "percent_calc の母数。" },
                "weight_kg": { "type": "number", "description": "bmi_calc の体重kg。" },
                "height_cm": { "type": "number", "description": "bmi_calc の身長cm。" },
                "from": { "type": "string", "description": "unit_convert の変換元単位。" },
                "to": { "type": "string", "description": "unit_convert の変換先単位。" },
                "date": { "type": "string", "description": "date_calc の基準日。YYYY-MM-DD。省略時は今日。" },
                "from_date": { "type": "string", "description": "date_diff の開始日。YYYY-MM-DD。" },
                "to_date": { "type": "string", "description": "date_diff の終了日。YYYY-MM-DD。" },
                "days": { "type": "integer", "description": "date_calc で加減する日数。" },
                "width": { "type": "integer", "description": "wrap_text の折り返し幅。" },
                "keyword": { "type": "string", "description": "keyword_count の検索語。" },
                "descending": { "type": "boolean", "description": "sort_lines を降順にする。" },
                "r": { "type": "integer", "description": "color_rgb_to_hex の赤。" },
                "g": { "type": "integer", "description": "color_rgb_to_hex の緑。" },
                "b": { "type": "integer", "description": "color_rgb_to_hex の青。" }
            },
            "required": ["action"]
        })
    }

    async fn run(&self, _cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let result = match action {
            "text_stats" => text_arg(&args, "text").map(text_stats),
            "line_stats" => text_arg(&args, "text").map(line_stats),
            "text_case" => {
                let text = text_arg(&args, "text");
                let mode = args.get("mode").and_then(Value::as_str).unwrap_or("slug");
                text.and_then(|text| text_case(text, mode))
            }
            "slugify" => text_arg(&args, "text").and_then(|text| text_case(text, "slug")),
            "reverse_text" => text_arg(&args, "text").map(reverse_text),
            "sort_lines" => text_arg(&args, "text").map(|text| {
                sort_lines(
                    text,
                    args.get("descending")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )
            }),
            "unique_lines" => text_arg(&args, "text").map(unique_lines),
            "trim_lines" => text_arg(&args, "text").map(trim_lines),
            "wrap_text" => text_arg(&args, "text").map(|text| {
                wrap_text(
                    text,
                    args.get("width")
                        .and_then(Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .unwrap_or(80),
                )
            }),
            "extract_urls" => text_arg(&args, "text").map(extract_urls),
            "extract_emails" => text_arg(&args, "text").map(extract_emails),
            "keyword_count" => text_arg(&args, "text").and_then(|text| {
                let keyword = args.get("keyword").and_then(Value::as_str).unwrap_or("");
                keyword_count(text, keyword)
            }),
            "checksum" => text_arg(&args, "text").map(fnv1a_checksum),
            "json_format" => text_arg(&args, "text").and_then(json_format),
            "json_minify" => text_arg(&args, "text").and_then(json_minify),
            "base64_encode" => {
                text_arg(&args, "text").map(|text| BASE64_STANDARD.encode(text.as_bytes()))
            }
            "base64_decode" => text_arg(&args, "text").and_then(|text| {
                BASE64_STANDARD
                    .decode(text.trim())
                    .map_err(|error| format!("Base64デコードに失敗しました: {error}"))
                    .and_then(|bytes| {
                        String::from_utf8(bytes)
                            .map_err(|error| format!("UTF-8文字列に変換できません: {error}"))
                    })
            }),
            "url_encode" => text_arg(&args, "text").map(url_encode),
            "url_decode" => text_arg(&args, "text").map(url_decode),
            "csv_to_markdown" => text_arg(&args, "text").map(csv_to_markdown),
            "unit_convert" => {
                let value = args
                    .get("value")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| "value が指定されていません。".to_string());
                let from = args
                    .get("from")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "from が指定されていません。".to_string());
                let to = args
                    .get("to")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "to が指定されていません。".to_string());
                value.and_then(|value| {
                    from.and_then(|from| to.and_then(|to| unit_convert(value, from, to)))
                })
            }
            "percent_calc" => {
                let value = args
                    .get("value")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| "value が指定されていません。".to_string());
                let total = args
                    .get("total")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| "total が指定されていません。".to_string());
                value.and_then(|value| total.and_then(|total| percent_calc(value, total)))
            }
            "bmi_calc" => {
                let weight = args
                    .get("weight_kg")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| "weight_kg が指定されていません。".to_string());
                let height = args
                    .get("height_cm")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| "height_cm が指定されていません。".to_string());
                weight.and_then(|weight| height.and_then(|height| bmi_calc(weight, height)))
            }
            "date_calc" => date_calc(&args),
            "date_diff" => date_diff(&args),
            "week_number" => week_number(&args),
            "timestamp_now" => Ok(timestamp_now()),
            "timestamp_to_date" => args
                .get("value")
                .and_then(Value::as_i64)
                .ok_or_else(|| "value に Unix timestamp を指定してください。".to_string())
                .and_then(timestamp_to_date),
            "color_hex_to_rgb" => text_arg(&args, "text").and_then(color_hex_to_rgb),
            "color_rgb_to_hex" => color_rgb_to_hex(&args),
            _ => Err(
                "action は productivity_tools が対応する30機能のいずれかを指定してください。"
                    .to_string(),
            ),
        };
        result.unwrap_or_else(|error| format!("エラー: {error}"))
    }
}

inventory::submit! { ToolReg { make: || Box::new(ProductivityTools) } }

#[cfg(test)]
mod tests {
    use super::{color_hex_to_rgb, json_format, text_case, unit_convert, url_decode, url_encode};

    #[test]
    fn 色指定は16進6桁だけを受け付ける() {
        assert_eq!(color_hex_to_rgb("#1A2B3C").unwrap(), "rgb(26, 43, 60)");
        // バイト長は6でも文字は2つ。長さだけで判定していた頃はここで panic していた。
        assert!(color_hex_to_rgb("あい").is_err());
        assert!(color_hex_to_rgb("#12345").is_err());
        assert!(color_hex_to_rgb("#12345g").is_err());
        assert!(color_hex_to_rgb("##123456").is_err());
    }

    #[test]
    fn jsonを整形する() {
        assert!(json_format(r#"{"a":1}"#).unwrap().contains("\n  \"a\""));
    }

    #[test]
    fn 文字列をスラッグ化する() {
        assert_eq!(
            text_case("Hello Pico Agent", "kebab").unwrap(),
            "hello-pico-agent"
        );
    }

    #[test]
    fn urlを往復変換する() {
        let encoded = url_encode("a b&c");
        assert_eq!(url_decode(&encoded), "a b&c");
    }

    #[test]
    fn 単位を換算する() {
        assert!(unit_convert(1.0, "km", "mi").unwrap().contains("0.6214"));
    }
}
