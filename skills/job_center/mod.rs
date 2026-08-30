// skills/job_center/mod.rs - 再起動後も残るバックグラウンドジョブの確認・再試行・中止。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;

pub struct JobCenterTool;

fn state_label(state: &crate::durable_jobs::JobState) -> &'static str {
    use crate::durable_jobs::JobState;
    match state {
        JobState::Queued => "待機中",
        JobState::Running => "実行中",
        JobState::Completed => "完了",
        JobState::Failed => "失敗",
        JobState::WaitingAuth => "認証待ち",
        JobState::Cancelled => "キャンセル済み",
    }
}

#[async_trait]
impl Tool for JobCenterTool {
    fn name(&self) -> &'static str {
        "job_center"
    }
    fn label(&self) -> &'static str {
        "ジョブセンター"
    }
    fn description(&self) -> String {
        "AI Driveへの転送や成果物生成など、アプリ再起動後も再開するバックグラウンドジョブを一覧・再試行・中止する。".to_string()
    }
    fn intent_keywords(&self) -> &'static [&'static str] {
        &["ジョブ", "バックグラウンド", "進捗", "再試行"]
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "status", "retry", "cancel"] },
                "job_id": { "type": "string", "description": "status/retry/cancel の対象ジョブID。" }
            },
            "required": ["action"]
        })
    }
    async fn run(&self, _cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("list");
        let id = args
            .get("job_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if matches!(action, "status" | "retry" | "cancel") && id.is_empty() {
            return "エラー: job_id が指定されていません。".to_string();
        }
        match action {
            "list" | "status" => match crate::durable_jobs::list() {
                Ok(jobs) => {
                    let selected: Vec<_> = jobs
                        .into_iter()
                        .filter(|job| action == "list" || job.id == id)
                        .take(30)
                        .collect();
                    if selected.is_empty() {
                        return if action == "status" {
                            format!("エラー: ジョブが見つかりません: {id}")
                        } else {
                            "永続ジョブはありません。".to_string()
                        };
                    }
                    let mut out = String::from("永続ジョブ:\n");
                    for job in selected {
                        out.push_str(&format!(
                            "- {} | {} | {}% | {} | {}\n",
                            job.id,
                            state_label(&job.state),
                            job.progress,
                            job.label,
                            job.updated_at
                        ));
                        if !job.result.is_empty() {
                            out.push_str(&format!(
                                "  結果: {}\n",
                                job.result.lines().next().unwrap_or("")
                            ));
                        }
                        if !job.error.is_empty() {
                            out.push_str(&format!(
                                "  エラー: {}\n",
                                job.error.lines().next().unwrap_or("")
                            ));
                        }
                    }
                    out
                }
                Err(error) => format!("エラー: {error}"),
            },
            "retry" => crate::durable_jobs::retry(id)
                .map(|_| format!("再試行を開始しました: {id}"))
                .unwrap_or_else(|error| format!("エラー: {error}")),
            "cancel" => crate::durable_jobs::cancel(id)
                .map(|_| format!("キャンセルしました: {id}"))
                .unwrap_or_else(|error| format!("エラー: {error}")),
            _ => "エラー: action は list/status/retry/cancel のいずれかです。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(JobCenterTool) } }
