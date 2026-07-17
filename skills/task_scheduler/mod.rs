// task_scheduler/mod.rs - cron 風の指定時刻タスクを登録・管理するスキル。
//
// 実行は中核の `crate::scheduler` が常駐して行う。ここではチャットからの追加・一覧・削除と、
// 設定画面に出す JSON schema を提供する。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::scheduler::{self, ScheduledTask};

pub struct TaskSchedulerTool;

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn string_array_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn normalized_kind(args: &Value) -> String {
    let kind = str_arg(args, "kind").to_lowercase();
    if kind == "tool" {
        "tool".to_string()
    } else {
        "command".to_string()
    }
}

fn format_task(task: &ScheduledTask) -> String {
    let schedule = if task.cron.trim().is_empty() {
        let days = scheduler::days_label(&task.days);
        if task.date.trim().is_empty() {
            format!("{} {}", days, task.time)
        } else {
            format!("{} {}", task.date, task.time)
        }
    } else {
        format!("cron {}", task.cron)
    };
    let target = if task.kind == "tool" {
        format!("スキル {} {}", task.tool_name, task.tool_args)
    } else {
        format!("コマンド {} {}", task.program, task.args.join(" "))
    };
    let repeat = if task.repeat { "繰り返し" } else { "一度だけ" };
    let state = if task.enabled { "" } else { "（無効）" };
    let label = if task.label.trim().is_empty() {
        String::new()
    } else {
        format!(" 〔{}〕", task.label)
    };
    format!("{schedule} / {target} / {repeat}{label}{state}（id:{}）", task.id)
}

fn list_text(tasks: &[ScheduledTask]) -> String {
    if tasks.is_empty() {
        return "登録中のスケジュールタスクはないよ。".to_string();
    }
    let mut out = String::from("登録中のスケジュールタスク:");
    for task in tasks {
        out.push_str("\n- ");
        out.push_str(&format_task(task));
    }
    out
}

fn build_task(cfg: &Config, args: &Value) -> Result<ScheduledTask, String> {
    let kind = normalized_kind(args);
    let cron = str_arg(args, "cron");
    let time = if cron.is_empty() {
        scheduler::normalize_time(&str_arg(args, "time"))
            .ok_or_else(|| "time は HH:MM（24時間表記）で指定してください。".to_string())?
    } else {
        String::new()
    };
    let repeat = args.get("repeat").and_then(Value::as_bool).unwrap_or(true);
    let tool_args = args
        .get("tool_args")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let task = ScheduledTask {
        id: scheduler::new_id(),
        kind,
        time,
        cron,
        days: scheduler::normalize_days(&string_array_arg(args, "days")),
        date: str_arg(args, "date"),
        repeat,
        label: str_arg(args, "label"),
        enabled: true,
        program: str_arg(args, "program"),
        args: string_array_arg(args, "args"),
        cwd: str_arg(args, "cwd"),
        timeout_secs: args.get("timeout_secs").and_then(Value::as_u64),
        tool_name: str_arg(args, "tool_name"),
        tool_args,
        last_run_minute: String::new(),
    };
    scheduler::validate_task(cfg, &task)?;
    Ok(task)
}

#[async_trait]
impl Tool for TaskSchedulerTool {
    fn name(&self) -> &'static str {
        "task_scheduler"
    }

    fn label(&self) -> &'static str {
        "タスクスケジューラ"
    }

    fn description(&self) -> String {
        "cron 風のスケジュールタスク。指定時刻や cron 式で、許可済みコマンドまたは別スキルを実行する。追加・一覧・削除・有効/無効化ができる。"
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
                "kind": {
                    "type": "string",
                    "enum": ["command", "tool"],
                    "description": "実行対象。command=ローカルコマンド、tool=スキル実行。既定 command"
                },
                "time": { "type": "string", "description": "時刻 HH:MM（24時間表記）。cron を使わない add で必須" },
                "cron": { "type": "string", "description": "cron 風の5フィールド（分 時 日 月 曜日）。例: 0 9 * * 1" },
                "days": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "曜日 mon..sun。平日/週末/毎日も可。time 指定時だけ使用。空なら毎日"
                },
                "date": { "type": "string", "description": "YYYY-MM-DD。指定するとその日だけ対象" },
                "repeat": { "type": "boolean", "description": "繰り返すか。一度だけなら false" },
                "label": { "type": "string", "description": "ラベル（任意）" },
                "program": { "type": "string", "description": "kind=command の実行プログラム。shell は使われない" },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "kind=command の引数配列"
                },
                "cwd": { "type": "string", "description": "kind=command の作業ディレクトリ。共通許可ルート配下のみ" },
                "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 120 },
                "tool_name": { "type": "string", "description": "kind=tool のスキル名。例: weather / web_fetch / gmail_message" },
                "tool_args": { "type": "object", "description": "kind=tool に渡す JSON 引数" },
                "id": { "type": "string", "description": "remove/enable/disable の対象ID（list で確認）" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            scheduler::CONFIG_KEY,
            "スケジュールタスク一覧（JSON。通常はチャットで設定。kind=command/tool、time または cron を指定）",
            "textarea",
            "[]",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = str_arg(&args, "action").to_lowercase();
        let mut tasks = scheduler::load(cfg);
        match action.as_str() {
            "list" => list_text(&tasks),
            "clear" => {
                tasks.clear();
                match scheduler::save(cfg, &tasks) {
                    Ok(()) => "スケジュールタスクを全部消したよ。".to_string(),
                    Err(error) => format!("エラー: スケジュールタスクの保存に失敗しました: {error}"),
                }
            }
            "add" => {
                let task = match build_task(cfg, &args) {
                    Ok(task) => task,
                    Err(error) => return format!("エラー: {error}"),
                };
                let summary = format_task(&task);
                let command_disabled_note = task.kind != "tool"
                    && !cfg.get_bool("PICOAGENT_COMMAND_ENABLED", false);
                tasks.push(task);
                match scheduler::save(cfg, &tasks) {
                    Ok(()) => {
                        if command_disabled_note {
                            format!(
                                "スケジュールタスクを設定したよ: {summary}\n注意: コマンド実行には PICOAGENT_COMMAND_ENABLED=true が必要です。"
                            )
                        } else {
                            format!("スケジュールタスクを設定したよ: {summary}")
                        }
                    }
                    Err(error) => format!("エラー: スケジュールタスクの保存に失敗しました: {error}"),
                }
            }
            "remove" => {
                let id = str_arg(&args, "id");
                let before = tasks.len();
                tasks.retain(|task| task.id != id);
                if tasks.len() == before {
                    return format!("その id のタスクは見つからなかったよ。\n{}", list_text(&tasks));
                }
                match scheduler::save(cfg, &tasks) {
                    Ok(()) => "スケジュールタスクを削除したよ。".to_string(),
                    Err(error) => format!("エラー: スケジュールタスクの保存に失敗しました: {error}"),
                }
            }
            "enable" | "disable" => {
                let id = str_arg(&args, "id");
                let want = action == "enable";
                let mut hit = false;
                for task in tasks.iter_mut() {
                    if task.id == id {
                        task.enabled = want;
                        hit = true;
                    }
                }
                if !hit {
                    return format!("その id のタスクは見つからなかったよ。\n{}", list_text(&tasks));
                }
                match scheduler::save(cfg, &tasks) {
                    Ok(()) => {
                        if want {
                            "スケジュールタスクを有効にしたよ。".to_string()
                        } else {
                            "スケジュールタスクを無効にしたよ。".to_string()
                        }
                    }
                    Err(error) => format!("エラー: スケジュールタスクの保存に失敗しました: {error}"),
                }
            }
            other => format!(
                "エラー: 未知の action \"{other}\" です。add / list / remove / clear / enable / disable のいずれかを指定してください。"
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(TaskSchedulerTool) } }
