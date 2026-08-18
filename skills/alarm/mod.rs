// alarm/mod.rs - 目覚まし時計スキル（チャットからアラームを設定・一覧・削除する）。
//
// 「平日の7時に起こして」のように頼むと、モデルがこのスキルを呼んで時刻・曜日・繰り返しを
// 登録する。保存と曜日/時刻の正規化は中核の `crate::alarm` が担い、発火（声でやさしく起こす）は
// フロントの時刻ポーリングが `pet_alarm_wake` 経由で行う。設定画面でも JSON を直接編集できる。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::alarm::{self, Alarm};
use crate::config::Config;
use crate::message::Field;

pub struct AlarmTool;

fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("al{nanos}")
}

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn days_arg(args: &Value) -> Vec<String> {
    args.get("days")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn format_alarm(a: &Alarm) -> String {
    let repeat = if a.repeat { "繰り返し" } else { "一度きり" };
    let state = if a.enabled { "" } else { "（無効）" };
    let snooze = a.snooze_minutes.clamp(1, 60);
    let label = if a.label.is_empty() {
        String::new()
    } else {
        format!(" 〔{}〕", a.label)
    };
    format!(
        "{} {}（{}・{}・スヌーズ{}分）{}{}",
        a.time,
        alarm::days_label(&a.days),
        repeat,
        "声で起こす",
        snooze,
        label,
        state
    )
}

fn list_text(alarms: &[Alarm]) -> String {
    if alarms.is_empty() {
        return "登録されている目覚ましはないよ。".to_string();
    }
    let mut out = String::from("登録中の目覚まし:");
    for a in alarms {
        out.push_str("\n- ");
        out.push_str(&format_alarm(a));
        out.push_str(&format!("（id:{}）", a.id));
    }
    out
}

#[async_trait]
impl Tool for AlarmTool {
    fn name(&self) -> &'static str {
        "alarm"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["アラーム", "目覚まし", "めざまし", "起こして", "起こし", "alarm", "wake", "何時に"]
    }

    fn label(&self) -> &'static str {
        "目覚まし時計"
    }

    fn description(&self) -> String {
        "目覚まし時計。声でやさしく起こすアラームを追加・一覧・削除・有効/無効にできる。\
時刻(HH:MM)・曜日(mon..sun、平日/週末/毎日)・繰り返し(repeat)を指定する。\
「平日の7時に起こして」「土日の朝8時、繰り返しで」のような依頼で使う。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove", "clear", "enable", "disable"],
                    "description": "操作。追加=add、一覧=list、削除=remove、全消去=clear、有効化=enable、無効化=disable"
                },
                "time": { "type": "string", "description": "時刻 HH:MM（24時間表記）。add で必須" },
                "days": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "曜日 mon..sun。平日/週末/毎日も可。空なら毎日"
                },
                "repeat": { "type": "boolean", "description": "繰り返す(true)か一度きり(false)。既定 true" },
                "snooze_minutes": { "type": "integer", "minimum": 1, "maximum": 60, "description": "起床確認がないときに再度鳴らすまでの分数。既定5分" },
                "label": { "type": "string", "description": "ラベル（任意）" },
                "id": { "type": "string", "description": "remove/enable/disable の対象ID（list で確認できる）" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            alarm::CONFIG_KEY,
            "目覚まし一覧（JSON。通常はチャットで設定。例:[{\"id\":\"a\",\"time\":\"07:00\",\"days\":[\"mon\"],\"repeat\":true,\"label\":\"朝\",\"enabled\":true,\"snooze_minutes\":5}]）",
            "textarea",
            "[]",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = str_arg(&args, "action").to_lowercase();
        // 読む→直す→書き戻すの間、同じ一覧を触る並列ツール実行と競合しないようにする。
        let _guard = crate::store::lock_state(alarm::CONFIG_KEY);
        let mut alarms = match alarm::load(cfg) {
            Ok(alarms) => alarms,
            // 空一覧から書き戻すと登録済みの目覚ましが消えるため、ここで止める。
            Err(error) => return format!("エラー: {error}"),
        };

        match action.as_str() {
            "list" => list_text(&alarms),
            "clear" => {
                alarms.clear();
                match alarm::save(cfg, &alarms) {
                    Ok(()) => "目覚ましを全部消したよ。".to_string(),
                    Err(error) => format!("エラー: 目覚ましの保存に失敗しました: {error}"),
                }
            }
            "add" => {
                let Some(time) = alarm::normalize_time(&str_arg(&args, "time")) else {
                    return "エラー: 時刻は HH:MM（24時間表記）で指定してください。".to_string();
                };
                let days = match alarm::normalize_days(&days_arg(&args)) {
                    Ok(days) => days,
                    Err(error) => return format!("エラー: {error}"),
                };
                let repeat = args.get("repeat").and_then(Value::as_bool).unwrap_or(true);
                let snooze_minutes = args
                    .get("snooze_minutes")
                    .and_then(Value::as_u64)
                    .map(|value| value.clamp(1, 60) as u32)
                    .unwrap_or(5);
                let label = str_arg(&args, "label");
                let alarm = Alarm {
                    id: new_id(),
                    time,
                    days,
                    repeat,
                    label,
                    enabled: true,
                    snooze_minutes,
                };
                let summary = format_alarm(&alarm);
                alarms.push(alarm);
                match alarm::save(cfg, &alarms) {
                    Ok(()) => format!("目覚ましを設定したよ: {summary}"),
                    Err(error) => format!("エラー: 目覚ましの保存に失敗しました: {error}"),
                }
            }
            "remove" => {
                let id = str_arg(&args, "id");
                if id.is_empty() {
                    return format!("エラー: 削除する目覚ましの id を指定してね。\n{}", list_text(&alarms));
                }
                let before = alarms.len();
                alarms.retain(|a| a.id != id);
                if alarms.len() == before {
                    return format!("その id の目覚ましは見つからなかったよ。\n{}", list_text(&alarms));
                }
                match alarm::save(cfg, &alarms) {
                    Ok(()) => "目覚ましを削除したよ。".to_string(),
                    Err(error) => format!("エラー: 目覚ましの保存に失敗しました: {error}"),
                }
            }
            "enable" | "disable" => {
                let id = str_arg(&args, "id");
                let want = action == "enable";
                let mut hit = false;
                for a in alarms.iter_mut() {
                    if a.id == id {
                        a.enabled = want;
                        hit = true;
                    }
                }
                if !hit {
                    return format!("その id の目覚ましは見つからなかったよ。\n{}", list_text(&alarms));
                }
                match alarm::save(cfg, &alarms) {
                    Ok(()) => {
                        if want {
                            "目覚ましを有効にしたよ。".to_string()
                        } else {
                            "目覚ましを無効にしたよ。".to_string()
                        }
                    }
                    Err(error) => format!("エラー: 目覚ましの保存に失敗しました: {error}"),
                }
            }
            other => format!(
                "エラー: 未知の action \"{other}\" です。add / list / remove / clear / enable / disable のいずれかを指定してください。"
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(AlarmTool) } }
