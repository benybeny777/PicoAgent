// workflow/mod.rs - 成功した複数スキル操作を名前付き手順として保存・再実行する。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const KEY: &str = "PICOAGENT_WORKFLOWS";
pub struct WorkflowTool;

fn load(cfg: &Config) -> Result<Vec<Value>, String> {
    crate::store::load_list(cfg, KEY)
}
fn save(cfg: &Config, items: &[Value]) -> Result<(), String> {
    cfg.set_string(
        KEY,
        serde_json::to_string(items).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("ワークフローを保存できません: {e}"))
}

#[async_trait]
impl Tool for WorkflowTool {
    fn name(&self) -> &'static str {
        "workflow"
    }
    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "ワークフロー",
            "手順を覚えて",
            "この操作を覚えて",
            "自動化",
            "再実行",
        ]
    }
    fn label(&self) -> &'static str {
        "記憶したワークフロー"
    }
    fn description(&self) -> String {
        "複数スキルの手順を名前付きで保存し、一覧・再実行・登録解除する。".to_string()
    }
    fn examples(&self) -> Vec<&'static str> {
        vec![
            "この手順を週報作成として覚えて",
            "週報作成ワークフローを実行して",
        ]
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","add","run","remove"]},
            "name":{"type":"string"},
            "steps":{"type":"array","maxItems":8,"items":{"type":"object","properties":{"tool":{"type":"string"},"args":{"type":"object"}},"required":["tool","args"]}},
            "confirm":{"type":"boolean"}
        },"required":["action"]})
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(
            KEY,
            "保存済みワークフロー（JSON）",
            "textarea",
            "[]",
        )]
    }
    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        match action {
            "list" => {
                let items = match load(cfg) {
                    Ok(items) => items,
                    Err(error) => return format!("エラー: {error}"),
                };
                if items.is_empty() {
                    "保存済みワークフローはありません。".to_string()
                } else {
                    match serde_json::to_string_pretty(&items) {
                        Ok(json) => json,
                        Err(error) => format!("エラー: 一覧を整形できません: {error}"),
                    }
                }
            }
            "add" => {
                let Some(steps) = args.get("steps").and_then(Value::as_array) else {
                    return "エラー: steps を指定してください。".to_string();
                };
                if name.is_empty() || steps.is_empty() || steps.len() > 8 {
                    return "エラー: name と1〜8件の steps を指定してください。".to_string();
                }
                for (index, step) in steps.iter().enumerate() {
                    let tool = step.get("tool").and_then(Value::as_str).unwrap_or("");
                    if tool.is_empty() || tool == "workflow" || !super::index().contains_key(tool) {
                        return format!("エラー: {}番目のツール指定が不正です: {tool}", index + 1);
                    }
                    if !step.get("args").is_some_and(Value::is_object) {
                        return format!(
                            "エラー: {}番目の args はオブジェクトで指定してください。",
                            index + 1
                        );
                    }
                }
                let _guard = crate::store::lock_state(KEY);
                let mut items = match load(cfg) {
                    Ok(items) => items,
                    Err(error) => return format!("エラー: {error}"),
                };
                items.retain(|item| item.get("name").and_then(Value::as_str) != Some(name));
                items.push(json!({"name":name,"steps":steps}));
                match save(cfg, &items) {
                    Ok(()) => format!("ワークフローを保存しました: {name}"),
                    Err(e) => format!("エラー: {e}"),
                }
            }
            "run" => {
                let items = match load(cfg) {
                    Ok(items) => items,
                    Err(error) => return format!("エラー: {error}"),
                };
                let Some(item) = items
                    .iter()
                    .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
                else {
                    return format!("エラー: ワークフローが見つかりません: {name}");
                };
                let Some(steps) = item.get("steps").and_then(Value::as_array) else {
                    return "エラー: 保存済み手順が壊れています。".to_string();
                };
                let mut results = Vec::new();
                for (index, step) in steps.iter().take(8).enumerate() {
                    let tool = step.get("tool").and_then(Value::as_str).unwrap_or("");
                    let step_args = step.get("args").cloned().unwrap_or_else(|| json!({}));
                    if tool.is_empty() || tool == "workflow" {
                        return format!("エラー: {}番目のツール指定が不正です。", index + 1);
                    }
                    let result = crate::tools::dispatch(cfg, tool, step_args).await;
                    results.push(format!("## {}. {}\n{}", index + 1, tool, result));
                    if result.starts_with("エラー:") {
                        break;
                    }
                }
                format!(
                    "ワークフロー「{name}」の実行結果\n\n{}",
                    results.join("\n\n")
                )
            }
            "remove" => {
                if !args
                    .get("confirm")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return format!("プレビュー: ワークフロー「{name}」を解除します。実行には confirm=true が必要です。");
                }
                let _guard = crate::store::lock_state(KEY);
                let mut items = match load(cfg) {
                    Ok(items) => items,
                    Err(error) => return format!("エラー: {error}"),
                };
                let before = items.len();
                items.retain(|item| item.get("name").and_then(Value::as_str) != Some(name));
                if items.len() == before {
                    return format!("エラー: ワークフローが見つかりません: {name}");
                }
                match save(cfg, &items) {
                    Ok(()) => format!("ワークフローを解除しました: {name}"),
                    Err(e) => format!("エラー: {e}"),
                }
            }
            _ => "エラー: action は list/add/run/remove のいずれかを指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(WorkflowTool) } }
