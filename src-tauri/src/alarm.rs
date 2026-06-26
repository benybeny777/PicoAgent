// alarm.rs - 目覚まし時計の保存と照合の中核。
//
// アラーム一覧は設定キー `PICOAGENT_ALARMS` に JSON 文字列で保存する（設定の正本は
// config.json）。チャット（雑談ならぬ目覚ましスキル `skills/alarm`）と設定画面の両方から
// 編集でき、発火（声でやさしく起こす）はフロントの時刻ポーリングが `pet_alarm_wake` を呼ぶ。
// 設定として変えうる値なので config に置き、ここは読み書きと曜日・時刻の正規化だけを担う。

use serde::{Deserialize, Serialize};

use crate::config::Config;

pub const CONFIG_KEY: &str = "PICOAGENT_ALARMS";

fn default_true() -> bool {
    true
}

fn default_snooze_minutes() -> u32 {
    5
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alarm {
    pub id: String,
    /// "HH:MM"（24時間表記）。
    pub time: String,
    /// 曜日 mon..sun。空なら毎日。
    #[serde(default)]
    pub days: Vec<String>,
    /// 繰り返すか（false なら一度鳴ったら無効化する一度きり）。
    #[serde(default = "default_true")]
    pub repeat: bool,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 起床確認がないときに再度鳴らすまでの分数。
    #[serde(default = "default_snooze_minutes")]
    pub snooze_minutes: u32,
}

const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// 保存済みのアラーム一覧を読む。壊れていれば空一覧。
pub fn load(cfg: &Config) -> Vec<Alarm> {
    let raw = cfg.get(CONFIG_KEY, "");
    if raw.trim().is_empty() {
        return Vec::new();
    }
    // フォールバック許可: JSON が壊れていてもアプリ継続を優先し、空一覧で扱う。
    serde_json::from_str(&raw).unwrap_or_default()
}

/// アラーム一覧を保存する（config に JSON 文字列で書く）。
pub fn save(cfg: &Config, alarms: &[Alarm]) -> anyhow::Result<()> {
    let mut patch = serde_json::Map::new();
    patch.insert(
        CONFIG_KEY.to_string(),
        serde_json::Value::String(serde_json::to_string(alarms)?),
    );
    cfg.set_all(patch)?;
    Ok(())
}

/// 1トークンを曜日コード集合へ。毎日/平日/週末や日本語・英語表記に対応。
fn day_codes(token: &str) -> Vec<&'static str> {
    match token.trim().to_lowercase().as_str() {
        "everyday" | "daily" | "all" | "毎日" | "まいにち" => WEEKDAYS.to_vec(),
        "weekday" | "weekdays" | "平日" => vec!["mon", "tue", "wed", "thu", "fri"],
        "weekend" | "weekends" | "週末" => vec!["sat", "sun"],
        "mon" | "monday" | "月" | "月曜" | "月曜日" => vec!["mon"],
        "tue" | "tuesday" | "火" | "火曜" | "火曜日" => vec!["tue"],
        "wed" | "wednesday" | "水" | "水曜" | "水曜日" => vec!["wed"],
        "thu" | "thursday" | "木" | "木曜" | "木曜日" => vec!["thu"],
        "fri" | "friday" | "金" | "金曜" | "金曜日" => vec!["fri"],
        "sat" | "saturday" | "土" | "土曜" | "土曜日" => vec!["sat"],
        "sun" | "sunday" | "日" | "日曜" | "日曜日" => vec!["sun"],
        _ => vec![],
    }
}

/// 曜日表現を mon..sun の正規形へ。空指定や「毎日」は空ベクタ（=毎日）。
pub fn normalize_days(input: &[String]) -> Vec<String> {
    let mut set: Vec<&'static str> = Vec::new();
    let mut everyday = false;
    for raw in input {
        let codes = day_codes(raw);
        if codes.len() == 7 {
            everyday = true;
        }
        for code in codes {
            if !set.contains(&code) {
                set.push(code);
            }
        }
    }
    if everyday {
        return Vec::new();
    }
    WEEKDAYS
        .iter()
        .filter(|d| set.contains(d))
        .map(|d| d.to_string())
        .collect()
}

/// "H:MM" / "HH:MM" を "HH:MM" へ正規化（範囲外は None）。
pub fn normalize_time(input: &str) -> Option<String> {
    let (h, m) = input.trim().split_once(':')?;
    let hour: u32 = h.trim().parse().ok()?;
    let minute: u32 = m.trim().parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{hour:02}:{minute:02}"))
}

/// 曜日コードを日本語表示へ（一覧・確認用）。空=毎日。
pub fn days_label(days: &[String]) -> String {
    if days.is_empty() {
        return "毎日".to_string();
    }
    let jp = |code: &str| match code {
        "mon" => "月",
        "tue" => "火",
        "wed" => "水",
        "thu" => "木",
        "fri" => "金",
        "sat" => "土",
        "sun" => "日",
        _ => "?",
    };
    let ordered: Vec<&str> = WEEKDAYS
        .iter()
        .filter(|d| days.iter().any(|x| x == *d))
        .map(|d| jp(d))
        .collect();
    ordered.join("・")
}

#[cfg(test)]
mod tests {
    use super::{days_label, normalize_days, normalize_time, Alarm};

    #[test]
    fn 時刻を正規化する() {
        assert_eq!(normalize_time("7:5").as_deref(), Some("07:05"));
        assert_eq!(normalize_time("23:59").as_deref(), Some("23:59"));
        assert_eq!(normalize_time("24:00"), None);
        assert_eq!(normalize_time("おはよう"), None);
    }

    #[test]
    fn 曜日を正規化する() {
        assert_eq!(
            normalize_days(&["平日".to_string()]),
            vec!["mon", "tue", "wed", "thu", "fri"]
        );
        assert!(normalize_days(&["毎日".to_string()]).is_empty());
        assert_eq!(
            normalize_days(&["日".to_string(), "土".to_string()]),
            vec!["sat", "sun"]
        );
        assert_eq!(
            days_label(&["mon".to_string(), "fri".to_string()]),
            "月・金"
        );
        assert_eq!(days_label(&[]), "毎日");
    }

    #[test]
    fn 古い目覚まし_json_はスヌーズ五分で読む() {
        let raw =
            r#"[{"id":"a","time":"07:00","days":[],"repeat":true,"label":"朝","enabled":true}]"#;
        let alarms: Vec<Alarm> = serde_json::from_str(raw).unwrap();
        assert_eq!(alarms[0].snooze_minutes, 5);
    }
}
