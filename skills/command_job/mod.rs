// skills/command_job/mod.rs - バックグラウンド実行ジョブの様子見・停止スキル。
//
// `run_command` に `background=true` で起動したジョブ（開発サーバ・長いビルド・
// 監視付きテスト）の途中出力を取り、必要なら止める。Claude Code の BashOutput /
// KillShell に相当する。
//
// 出力は「前回返した続き」だけを返すので、同じジョブを何度も見てもコンテキストが
// 同じログで埋まらない。ジョブ本体の管理は `run_command` 側の `jobs.rs` にある
// （起動時の許可判定と同じ場所に置くため）。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::run_command::jobs;
use super::{Tool, ToolReg};
use crate::config::Config;

pub struct CommandJobTool;

#[async_trait]
impl Tool for CommandJobTool {
    fn name(&self) -> &'static str {
        "command_job"
    }

    fn label(&self) -> &'static str {
        "バックグラウンド実行の確認"
    }

    fn description(&self) -> String {
        "run_command を background=true で起動したジョブの一覧・途中出力の取得・停止を行う。output は前回からの続きだけを返す。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "output", "stop"],
                    "description": "list は実行中・終了済みの一覧、output は途中出力、stop は停止。"
                },
                "job_id": {
                    "type": "string",
                    "description": "対象のジョブID（run_command が返した job-N）。output / stop で必須。"
                },
                "from_start": {
                    "type": "boolean",
                    "description": "output で、前回の続きではなく保持している出力すべてを返す。"
                }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["さっきのビルドの様子を見せて", "動かしっぱなしのサーバを止めて"]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        // 起動元と同じスイッチで無効化できるようにする（無効なのに残骸を操作できると分かりにくい）。
        if !cfg.get_bool("PICOAGENT_COMMAND_ENABLED", false) {
            return "エラー: command_job は run_command と同じ設定で動きます。設定画面で PICOAGENT_COMMAND_ENABLED を true にしてください。"
                .to_string();
        }
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if action == "list" {
            return jobs::list();
        }
        let job_id = match args.get("job_id").and_then(Value::as_str).map(str::trim) {
            Some(job_id) if !job_id.is_empty() => job_id,
            _ => {
                return format!(
                    "エラー: job_id が指定されていません。\n{}",
                    jobs::list()
                )
            }
        };
        match action.as_str() {
            "output" => jobs::output(
                job_id,
                args.get("from_start")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            "stop" => jobs::stop(job_id),
            other => format!(
                "エラー: action は list/output/stop のいずれかを指定してください: {other}"
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(CommandJobTool) } }
