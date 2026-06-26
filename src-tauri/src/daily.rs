// daily.rs - リマインダー・集中タイマー・見守り通知・できたことメモの保存。
//
// 通知系の一覧は config の JSON 文字列へ保存する。チャットのスキルが追加/削除し、
// フロントは `pet_daily_events` で期限到来分を取り出して声で知らせる。

use chrono::{Duration, Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;

pub const EVENTS_KEY: &str = "PICOAGENT_DAILY_EVENTS";
pub const PRAISE_KEY: &str = "PICOAGENT_PRAISE_LOG";

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DailyEvent {
    pub id: String,
    pub kind: String,
    pub due_at: String,
    pub text: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub repeat_minutes: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PraiseEntry {
    pub id: String,
    pub at: String,
    pub text: String,
}

pub fn new_id(prefix: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}{millis}")
}

pub fn now_local_minute() -> String {
    Local::now().format("%Y-%m-%dT%H:%M").to_string()
}

pub fn after_minutes(minutes: i64) -> String {
    (Local::now() + Duration::minutes(minutes.max(1)))
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

pub fn parse_local_minute(input: &str) -> Option<NaiveDateTime> {
    let value = input.trim();
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M"))
        .ok()
}

pub fn load_events(cfg: &Config) -> Vec<DailyEvent> {
    let raw = cfg.get(EVENTS_KEY, "");
    if raw.trim().is_empty() {
        return Vec::new();
    }
    // フォールバック許可: JSON が壊れていてもアプリ継続を優先し、空一覧で扱う。
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_events(cfg: &Config, events: &[DailyEvent]) -> anyhow::Result<()> {
    let mut patch = serde_json::Map::new();
    patch.insert(
        EVENTS_KEY.to_string(),
        Value::String(serde_json::to_string(events)?),
    );
    cfg.set_all(patch)?;
    Ok(())
}

pub fn load_praise(cfg: &Config) -> Vec<PraiseEntry> {
    let raw = cfg.get(PRAISE_KEY, "");
    if raw.trim().is_empty() {
        return Vec::new();
    }
    // フォールバック許可: JSON が壊れていてもアプリ継続を優先し、空一覧で扱う。
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_praise(cfg: &Config, entries: &[PraiseEntry]) -> anyhow::Result<()> {
    let mut patch = serde_json::Map::new();
    patch.insert(
        PRAISE_KEY.to_string(),
        Value::String(serde_json::to_string(entries)?),
    );
    cfg.set_all(patch)?;
    Ok(())
}

pub fn due_events(cfg: &Config) -> anyhow::Result<Vec<DailyEvent>> {
    let now = Local::now().naive_local();
    let mut events = load_events(cfg);
    let mut due = Vec::new();
    for event in events.iter_mut() {
        if !event.enabled {
            continue;
        }
        let Some(due_at) = parse_local_minute(&event.due_at) else {
            continue;
        };
        if due_at > now {
            continue;
        }
        due.push(event.clone());
        if let Some(minutes) = event.repeat_minutes.filter(|m| *m > 0) {
            let mut next = due_at + Duration::minutes(minutes as i64);
            while next <= now {
                next += Duration::minutes(minutes as i64);
            }
            event.due_at = next.format("%Y-%m-%dT%H:%M").to_string();
        } else {
            event.enabled = false;
        }
    }
    save_events(cfg, &events)?;
    Ok(due)
}

#[cfg(test)]
mod tests {
    use super::parse_local_minute;

    #[test]
    fn ローカル分時刻を解析する() {
        assert!(parse_local_minute("2026-06-27T07:30").is_some());
        assert!(parse_local_minute("2026-06-27 07:30").is_some());
        assert!(parse_local_minute("明日の朝").is_none());
    }
}
