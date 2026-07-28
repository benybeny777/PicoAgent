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

/// 保存済みの通知一覧を読む。壊れていた場合は空一覧にせずエラーを返す
/// （空から始めて書き戻すと、登録済みのリマインダーがすべて消えるため）。
pub fn load_events(cfg: &Config) -> Result<Vec<DailyEvent>, String> {
    crate::store::load_list(cfg, EVENTS_KEY)
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

/// 保存済みの「できたこと」を読む。壊れていた場合は空一覧にせずエラーを返す。
pub fn load_praise(cfg: &Config) -> Result<Vec<PraiseEntry>, String> {
    crate::store::load_list(cfg, PRAISE_KEY)
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
    // 読めなかった場合は空一覧で続けず、そのままエラーにする（空で書き戻さないため）。
    let mut events = load_events(cfg).map_err(|error| anyhow::anyhow!(error))?;
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
    // この関数はフロントから 20 秒ごとに呼ばれる。期限到来が無いときまで保存すると、
    // config.json 全体の書き戻しが延々と続いてしまうので、変化したときだけ保存する。
    if !due.is_empty() {
        save_events(cfg, &events)?; // 実際に変わったときだけ書く
    }
    Ok(due)
}

#[cfg(test)]
mod tests {
    use super::{after_minutes, due_events, parse_local_minute, save_events, DailyEvent};
    use crate::config::Config;
    use chrono::{Duration, Local};

    #[test]
    fn ローカル分時刻を解析する() {
        assert!(parse_local_minute("2026-06-27T07:30").is_some());
        assert!(parse_local_minute("2026-06-27 07:30").is_some());
        assert!(parse_local_minute("明日の朝").is_none());
    }

    fn temp_config(label: &str) -> (Config, std::path::PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("picoagent-daily-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let cfg = Config::new();
        cfg.load(path.clone());
        (cfg, path)
    }

    /// この関数は 20 秒ごとにポーリングされる。期限到来が無いのに毎回保存すると
    /// config.json 全体の書き戻しが延々と続く。
    #[test]
    fn 期限到来が無ければ保存しない() {
        let (cfg, path) = temp_config("idle");
        let future = DailyEvent {
            id: "d1".to_string(),
            kind: "reminder".to_string(),
            due_at: after_minutes(60),
            text: "お茶を飲む".to_string(),
            label: String::new(),
            enabled: true,
            repeat_minutes: None,
        };
        save_events(&cfg, &[future]).unwrap();
        assert!(path.exists());

        // 保存済みファイルを消してから呼ぶ。書き込みが起きればファイルが再生成される。
        std::fs::remove_file(&path).unwrap();
        let due = due_events(&cfg).unwrap();
        assert!(due.is_empty());
        assert!(!path.exists(), "期限到来が無いのに保存している");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn 期限到来したら保存して一度きりは無効化する() {
        let (cfg, path) = temp_config("due");
        let past = DailyEvent {
            id: "d1".to_string(),
            kind: "reminder".to_string(),
            // after_minutes は最短1分先へ丸めるため、過去時刻は直接組み立てる。
            due_at: (Local::now() - Duration::minutes(60))
                .format("%Y-%m-%dT%H:%M")
                .to_string(),
            text: "お茶を飲む".to_string(),
            label: String::new(),
            enabled: true,
            repeat_minutes: None,
        };
        save_events(&cfg, &[past]).unwrap();
        std::fs::remove_file(&path).unwrap();

        let due = due_events(&cfg).unwrap();
        assert_eq!(due.len(), 1);
        assert!(path.exists(), "期限到来時は保存する");
        // 一度きりの通知は無効化され、次のポーリングでは何も出ない（＝保存も起きない）。
        std::fs::remove_file(&path).unwrap();
        assert!(due_events(&cfg).unwrap().is_empty());
        assert!(!path.exists());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
