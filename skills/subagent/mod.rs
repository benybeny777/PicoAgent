// subagent/mod.rs - 集中タスクを別エージェントへ委譲するスキル。
//
// モデルが大きな依頼を分担したいとき、サブタスクを `task` として渡すと、
// 現在選択中の叩く AI とスキル一式を使ったネストしたエージェントループを回し、
// 結果テキストだけを呼び出し元へ返す。無限再帰は深さガード（AGENT_DEPTH）で防ぐ。

use std::sync::atomic::Ordering;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::agent;
use crate::config::Config;
use crate::message::{Field, Message};
use crate::providers;

pub struct SubagentTool;

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

#[async_trait]
impl Tool for SubagentTool {
    fn name(&self) -> &'static str {
        "subagent"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["サブエージェント", "分担", "手分け", "それぞれ調べ", "並行で調べ", "担当", "subagent"]
    }

    fn label(&self) -> &'static str {
        "サブエージェント"
    }

    fn description(&self) -> String {
        "大きな依頼を分担するためのサブエージェント。1つの集中したサブタスクを渡すと、別のエージェントループがツールを使って遂行し、結果だけを返す。複数観点の調査や、独立した小タスクの委譲に使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "サブエージェントに遂行させる単一の集中タスク（具体的に書く）"
                },
                "context": {
                    "type": "string",
                    "description": "サブタスクを進めるうえで前提となる背景情報（任意）"
                },
                "steps": {
                    "type": "integer",
                    "description": "サブエージェントの最大ループ数（既定6、1〜12）"
                }
            },
            "required": ["task"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "競合3社をそれぞれ調べて（各社をサブエージェントに分担）",
            "この資料の各章を担当ごとに要約して",
        ]
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            "PICOAGENT_SUBAGENT_MAX_DEPTH",
            "サブエージェントの最大ネスト深さ",
            "number",
            "2",
        )]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let task = str_arg(&args, "task");
        if task.is_empty() {
            return "エラー: サブエージェントに渡す task が空です。遂行させたいサブタスクを指定してください。".to_string();
        }

        // 無限再帰防止: 現在の深さが上限以上ならこれ以上ネストしない。
        // 呼び出し元のエージェントループ内で実行されるため、ここでの深さは 1 以上。
        let max_depth = cfg
            .get("PICOAGENT_SUBAGENT_MAX_DEPTH", "2")
            .parse::<usize>()
            .unwrap_or(2)
            .max(1);
        if agent::AGENT_DEPTH.load(Ordering::SeqCst) >= max_depth {
            return format!(
                "エラー: サブエージェントのネストが上限（{max_depth}）に達したため、これ以上は分担できません。このタスクは自分で続行してください。"
            );
        }

        // 現在選択中の叩く AI を解決する（呼び出し元と同じプロバイダ・モデルを使う）。
        let id = cfg.get("PICOAGENT_PROVIDER", "local");
        let provider = match providers::get(&id) {
            Some(p) => p,
            None => return format!("エラー: プロバイダ \"{id}\" が見つかりません。"),
        };
        let mut model = cfg.get("PICOAGENT_MODEL", "");
        if model.is_empty() {
            model = provider.default_model(cfg);
        }

        let steps = args
            .get("steps")
            .and_then(Value::as_i64)
            .unwrap_or(6)
            .clamp(1, 12) as usize;

        let context = str_arg(&args, "context");
        let mut system = String::from(
            "あなたは上位エージェントから1つのサブタスクを任された作業担当エージェントです。\
与えられたタスクだけに集中し、必要に応じてツールを使って遂行してください。\
最後は、上位エージェントがそのまま使える簡潔な結果（要点・結論・成果物の場所）だけを返してください。\
雑談や前置きは不要です。",
        );
        if !context.is_empty() {
            system.push_str("\n\n前提情報:\n");
            system.push_str(&context);
        }

        let mut messages = vec![Message::new("system", system), Message::new("user", task)];

        // ネストしたエージェントループ。AppHandle は渡さない（ストリーミング/使用量記録は上位のみ）。
        // 中断フラグも渡さない（停止ボタンは上位ループのステップ境界で効く）。
        match agent::run(cfg, provider, &model, &mut messages, steps, None, None).await {
            Ok(result) if !result.trim().is_empty() => {
                format!("【サブエージェントの結果】\n{}", result.trim())
            }
            Ok(_) => "サブエージェントは結果を返しませんでした。".to_string(),
            Err(e) => format!("エラー: サブエージェントの実行に失敗しました: {e}"),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(SubagentTool) } }
