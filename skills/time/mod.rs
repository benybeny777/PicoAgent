// tools/time.rs - エージェント機能コネクタ（例）: 現在時刻を返す。副作用なし。

use async_trait::async_trait;
use chrono::Local;
use chrono_tz::Tz;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;

pub struct TimeTool;

#[async_trait]
impl Tool for TimeTool {
    fn name(&self) -> &'static str {
        "get_time"
    }
    fn description(&self) -> String {
        "現在の日付と時刻を返す。ユーザーが今の時間・日付・曜日を尋ねたときに使う。".to_string()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "IANA タイムゾーン名（例: Asia/Tokyo）。省略時はローカル時刻。"
                }
            },
            "required": []
        })
    }

    async fn run(&self, _cfg: &Config, args: Value) -> String {
        let fmt = "%Y/%m/%d %H:%M:%S";
        if let Some(tz_name) = args.get("timezone").and_then(|v| v.as_str()) {
            if !tz_name.is_empty() {
                match tz_name.parse::<Tz>() {
                    Ok(tz) => {
                        let now = Local::now().with_timezone(&tz);
                        return format!("現在時刻: {}（{}）", now.format(fmt), tz_name);
                    }
                    Err(_) => return format!("エラー: 不明なタイムゾーンです: {}", tz_name),
                }
            }
        }
        let now = Local::now();
        format!("現在時刻: {}", now.format(fmt))
    }
}

inventory::submit! { ToolReg { make: || Box::new(TimeTool) } }
