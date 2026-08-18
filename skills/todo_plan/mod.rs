// skills/todo_plan/mod.rs - 多段タスクの手順を宣言し、進捗を1つずつ進める作業計画スキル。
//
// 小型のローカルモデルは、3手を超える依頼で「何をどこまでやったか」を落としやすい。
// 手順を明示的な状態として外に置き、毎回の結果で現在地を返すことで、途中でぶれても
// 次の一手が決まるようにする（Claude Code の TodoWrite / Codex の update_plan と同じ狙い）。
//
// 保存先は config の `PICOAGENT_TODO_PLAN`。並列ツール実行で消し合わないよう、
// 「読む → 直す → 書き戻す」は名前付きロックの内側で完結させる。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::text;

const KEY: &str = "PICOAGENT_TODO_PLAN";
const MAX_STEPS: usize = 20;
const MAX_STEP_CHARS: usize = 200;

#[derive(Serialize, Deserialize, Clone)]
pub struct Step {
    /// 表示・指定に使う 1 始まりの番号。
    pub number: usize,
    pub text: String,
    /// `pending` / `doing` / `done` のいずれか。
    pub status: String,
}

const PENDING: &str = "pending";
const DOING: &str = "doing";
const DONE: &str = "done";

fn mark(status: &str) -> &'static str {
    match status {
        DONE => "[x]",
        DOING => "[>]",
        _ => "[ ]",
    }
}

fn render(steps: &[Step]) -> String {
    if steps.is_empty() {
        return "作業計画はまだありません（set で手順を登録してください）。".to_string();
    }
    let done = steps.iter().filter(|step| step.status == DONE).count();
    let mut out = format!("作業計画（{done}/{} 完了）", steps.len());
    for step in steps {
        out.push_str(&format!(
            "\n{} {}. {}",
            mark(&step.status),
            step.number,
            step.text
        ));
    }
    if let Some(next) = steps.iter().find(|step| step.status != DONE) {
        out.push_str(&format!("\n次にやること: {}. {}", next.number, next.text));
    } else {
        out.push_str("\nすべて完了しました。");
    }
    out
}

fn parse_steps(args: &Value) -> Result<Vec<Step>, String> {
    let list = args
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "steps（手順の配列）が指定されていません。".to_string())?;
    if list.is_empty() {
        return Err("steps が空です。手順を1つ以上指定してください。".to_string());
    }
    if list.len() > MAX_STEPS {
        return Err(format!("steps は {MAX_STEPS} 件以内で指定してください。"));
    }
    let mut steps = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        // 文字列と { "text": ... } の両方を受け付ける（モデルの出力ぶれを吸収する）。
        let raw = item
            .as_str()
            .or_else(|| item.get("text").and_then(Value::as_str))
            .unwrap_or("")
            .trim();
        if raw.is_empty() {
            return Err(format!("steps[{index}] が空です。"));
        }
        steps.push(Step {
            number: index + 1,
            text: text::preview(raw, MAX_STEP_CHARS),
            // 最初の手順だけ着手中にしておくと、次の一手が常に決まる。
            status: if index == 0 { DOING } else { PENDING }.to_string(),
        });
    }
    Ok(steps)
}

fn save(cfg: &Config, steps: &[Step]) -> Result<(), String> {
    let encoded = serde_json::to_string(steps)
        .map_err(|error| format!("作業計画を保存用に変換できません: {error}"))?;
    cfg.set_string(KEY, encoded)
        .map_err(|error| format!("作業計画を保存できません: {error}"))
}

/// 指定番号を完了にし、次の未完了手順を着手中へ進める。
fn complete(steps: &mut [Step], number: Option<usize>) -> Result<usize, String> {
    let target = match number {
        Some(number) => number,
        None => steps
            .iter()
            .find(|step| step.status != DONE)
            .map(|step| step.number)
            .ok_or_else(|| "未完了の手順がありません。".to_string())?,
    };
    let Some(step) = steps.iter_mut().find(|step| step.number == target) else {
        return Err(format!("手順 {target} は登録されていません。"));
    };
    step.status = DONE.to_string();
    if let Some(next) = steps.iter_mut().find(|step| step.status == PENDING) {
        next.status = DOING.to_string();
    }
    Ok(target)
}

pub struct TodoPlanTool;

#[async_trait]
impl Tool for TodoPlanTool {
    fn name(&self) -> &'static str {
        "todo_plan"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["計画", "手順を整理", "やることを整理", "進捗", "どこまで終わ", "タスク整理"]
    }

    fn label(&self) -> &'static str {
        "作業計画"
    }

    fn description(&self) -> String {
        "複数手順の作業計画を登録し、1つずつ進捗を進める。3手以上かかる依頼を受けたら最初に set で手順を並べ、1手終えるたびに complete を呼ぶ。現在地は list で確認する。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "list", "complete", "add", "clear"],
                    "description": "set は計画の作り直し、complete は手順を1つ完了、add は手順の追加、clear は全消去。"
                },
                "steps": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "set / add で登録する手順（短い日本語で1行ずつ）。"
                },
                "number": {
                    "type": "integer",
                    "description": "complete で完了にする手順番号。省略時は先頭の未完了手順を完了にする。"
                }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["やることを整理して進めて", "いまどこまで終わってる？"]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_lowercase();
        // 読む→直す→書き戻すの間、同じ一覧を触る並列ツール実行と競合しないようにする。
        let _guard = crate::store::lock_state(KEY);
        // 壊れていても空一覧で始めない（次の保存で計画が丸ごと消えるため）。
        let mut steps: Vec<Step> = match crate::store::load_list(cfg, KEY) {
            Ok(steps) => steps,
            Err(error) => return format!("エラー: {error}"),
        };

        match action.as_str() {
            "list" => render(&steps),
            "set" => {
                let steps = match parse_steps(&args) {
                    Ok(steps) => steps,
                    Err(error) => return format!("エラー: {error}"),
                };
                match save(cfg, &steps) {
                    Ok(()) => format!("OK: 作業計画を登録しました。\n{}", render(&steps)),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "add" => {
                let added = match parse_steps(&args) {
                    Ok(added) => added,
                    Err(error) => return format!("エラー: {error}"),
                };
                if steps.len() + added.len() > MAX_STEPS {
                    return format!("エラー: 手順は合計 {MAX_STEPS} 件までです。");
                }
                for step in added {
                    let number = steps.len() + 1;
                    steps.push(Step {
                        number,
                        text: step.text,
                        status: PENDING.to_string(),
                    });
                }
                // 着手中が無ければ先頭の未完了を着手中にする。
                if !steps.iter().any(|step| step.status == DOING) {
                    if let Some(next) = steps.iter_mut().find(|step| step.status == PENDING) {
                        next.status = DOING.to_string();
                    }
                }
                match save(cfg, &steps) {
                    Ok(()) => format!("OK: 手順を追加しました。\n{}", render(&steps)),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "complete" => {
                if steps.is_empty() {
                    return "作業計画がありません。先に set で手順を登録してください。".to_string();
                }
                let number = args
                    .get("number")
                    .and_then(Value::as_u64)
                    .map(|number| number as usize);
                match complete(&mut steps, number) {
                    Ok(done) => match save(cfg, &steps) {
                        Ok(()) => format!("OK: 手順 {done} を完了にしました。\n{}", render(&steps)),
                        Err(error) => format!("エラー: {error}"),
                    },
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "clear" => match cfg.set_string(KEY, String::new()) {
                Ok(()) => "OK: 作業計画を消しました。".to_string(),
                Err(error) => format!("エラー: 作業計画を保存できません: {error}"),
            },
            other => format!(
                "エラー: action は set/list/complete/add/clear のいずれかを指定してください: {other}"
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(TodoPlanTool) } }

#[cfg(test)]
mod tests {
    use super::{complete, parse_steps, render, DOING, DONE, PENDING};
    use serde_json::json;

    #[test]
    fn 手順の配列を文字列でもオブジェクトでも受け取る() {
        let steps = parse_steps(&json!({ "steps": ["調べる", { "text": "直す" }] })).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].status, DOING);
        assert_eq!(steps[1].status, PENDING);
        assert_eq!(steps[1].text, "直す");
        assert!(parse_steps(&json!({ "steps": [] })).is_err());
    }

    /// 完了にしたら、次の未完了手順が自動で着手中になる（次の一手が常に決まる）。
    #[test]
    fn 完了すると次の手順が着手中になる() {
        let mut steps = parse_steps(&json!({ "steps": ["A", "B", "C"] })).unwrap();
        assert_eq!(complete(&mut steps, None).unwrap(), 1);
        assert_eq!(steps[0].status, DONE);
        assert_eq!(steps[1].status, DOING);
        assert_eq!(complete(&mut steps, Some(3)).unwrap(), 3);
        assert_eq!(steps[2].status, DONE);
        assert!(complete(&mut steps, Some(9)).is_err());
    }

    #[test]
    fn 現在地と次の一手を表示する() {
        let mut steps = parse_steps(&json!({ "steps": ["A", "B"] })).unwrap();
        assert!(render(&steps).contains("次にやること: 1. A"));
        complete(&mut steps, None).unwrap();
        complete(&mut steps, None).unwrap();
        assert!(render(&steps).contains("すべて完了"));
    }
}
