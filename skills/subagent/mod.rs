// subagent/mod.rs - 単一サブタスクの委譲と、複数AI回答の比較統合。
//
// 従来の `task` だけを渡す呼び方は、現在選択中の叩くAIへ1つのサブタスクを委譲する。
// `agents` を2件以上渡した場合は、同じ質問を指定した複数の叩くAIへ送り、候補回答を
// 集めたあと、統合AIが良い部分をまとめて1つの回答を返す。
//
// 内蔵ローカルAIは1モデル常駐なので同時推論せず直列化する。クラウドAI、Ollama、
// CLIエージェントは全体の並列上限内で同時実行できる。無限再帰はタスクローカルな
// AGENT_DEPTHで防ぎ、並列エージェント同士をネストと誤認しない。

use std::sync::Arc;

use async_trait::async_trait;
use futures::{stream, StreamExt};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::agent;
use crate::config::Config;
use crate::message::{Field, Message};
use crate::providers;

pub struct SubagentTool;

const MAX_AGENTS: usize = 8;
const MAX_TEXT_CHARS: usize = 32_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AiSelection {
    name: String,
    provider: String,
    model: String,
    focus: String,
}

#[derive(Clone, Debug)]
struct MultiRequest {
    task: String,
    context: String,
    mode: String,
    steps: usize,
    max_concurrency: usize,
    agents: Vec<AiSelection>,
    judge: AiSelection,
}

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn bounded_text(value: String, label: &str) -> Result<String, String> {
    let count = value.chars().count();
    if count > MAX_TEXT_CHARS {
        return Err(format!(
            "エラー: {label} が長すぎます（上限 {MAX_TEXT_CHARS} 文字、現在 {count} 文字）。"
        ));
    }
    Ok(value)
}

fn selection_from_value(value: &Value, fallback_provider: &str, index: usize) -> AiSelection {
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or(fallback_provider)
        .trim()
        .to_string();
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let focus = value
        .get("focus")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    AiSelection {
        name: if name.is_empty() {
            format!("候補{}", index + 1)
        } else {
            name.to_string()
        },
        provider,
        model,
        focus,
    }
}

fn parse_multi_request(cfg: &Config, args: &Value) -> Result<Option<MultiRequest>, String> {
    let Some(agent_values) = args.get("agents").and_then(Value::as_array) else {
        return Ok(None);
    };
    if agent_values.len() < 2 {
        return Err("エラー: マルチAI合議には agents を2件以上指定してください。".to_string());
    }
    if agent_values.len() > MAX_AGENTS {
        return Err(format!(
            "エラー: 同時に指定できるAIは最大 {MAX_AGENTS} 件です。"
        ));
    }

    let task = bounded_text(str_arg(args, "task"), "task")?;
    if task.is_empty() {
        return Err("エラー: 複数AIへ送る共通の task が空です。".to_string());
    }
    let context = bounded_text(str_arg(args, "context"), "context")?;
    let mode = match str_arg(args, "mode").as_str() {
        "collaborate" => "collaborate".to_string(),
        "divide" => "divide".to_string(),
        _ => "consensus".to_string(),
    };
    let steps = args
        .get("steps")
        .and_then(Value::as_i64)
        .unwrap_or(6)
        .clamp(1, 12) as usize;
    let configured_concurrency = cfg
        .get("PICOAGENT_MULTI_AGENT_MAX_CONCURRENCY", "3")
        .parse::<usize>()
        .unwrap_or(3)
        .clamp(1, MAX_AGENTS);
    let max_concurrency = args
        .get("max_concurrency")
        .and_then(Value::as_i64)
        .map(|value| value.clamp(1, MAX_AGENTS as i64) as usize)
        .unwrap_or(configured_concurrency);

    let fallback_provider = cfg.get("PICOAGENT_PROVIDER", "local");
    let agents = agent_values
        .iter()
        .enumerate()
        .map(|(index, value)| selection_from_value(value, &fallback_provider, index))
        .collect::<Vec<_>>();
    if let Some(empty) = agents.iter().find(|selection| selection.provider.is_empty()) {
        return Err(format!("エラー: {} の provider が空です。", empty.name));
    }

    let judge_value = args.get("judge").unwrap_or(&Value::Null);
    let mut judge = selection_from_value(judge_value, &fallback_provider, 0);
    if judge_value.is_null() || judge.name == "候補1" {
        judge.name = "統合AI".to_string();
    }

    Ok(Some(MultiRequest {
        task,
        context,
        mode,
        steps,
        max_concurrency,
        agents,
        judge,
    }))
}

fn system_prompt(context: &str) -> String {
    let mut system = String::from(
        "あなたは上位エージェントから同じ質問への候補回答を求められた作業担当エージェントです。\
与えられた質問だけに集中し、必要に応じてツールを使って調査・作業してください。\
最後は、根拠と重要な注意点を含む、単独でも使える簡潔な回答を返してください。\
他の候補AIの回答は見えないため、比較や多数決について言及しないでください。",
    );
    if !context.is_empty() {
        system.push_str("\n\n前提情報:\n");
        system.push_str(context);
    }
    system
}

async fn run_candidate(
    cfg: &Config,
    task: String,
    context: &str,
    steps: usize,
    selection: AiSelection,
    local_gate: Arc<futures::lock::Mutex<()>>,
) -> (AiSelection, String, Result<String, String>) {
    let provider = match providers::get(&selection.provider) {
        Some(provider) => provider,
        None => {
            let id = selection.provider.clone();
            return (
                selection,
                task,
                Err(format!("プロバイダ \"{id}\" が見つかりません。")),
            );
        }
    };
    let model = if selection.model.is_empty() {
        provider.default_model(cfg)
    } else {
        selection.model.clone()
    };
    let mut resolved = selection;
    resolved.model = model.clone();
    let mut messages = vec![
        Message::new("system", system_prompt(context)),
        Message::new("user", task.clone()),
    ];

    // 内蔵ローカルAIは1モデル常駐。複数リクエストを同時に投げず、候補ごとに完了を待つ。
    let _local_guard = if resolved.provider == "local" {
        Some(local_gate.lock().await)
    } else {
        None
    };
    let result = agent::run(cfg, provider, &model, &mut messages, steps, None, None)
        .await
        .map_err(|error| error.to_string())
        .and_then(|answer| {
            let answer = answer.trim().to_string();
            if answer.is_empty() {
                Err("空の回答が返されました。".to_string())
            } else {
                Ok(answer)
            }
        });
    (resolved, task, result)
}

fn candidates_for_judge(
    task: &str,
    candidates: &[(AiSelection, String, Result<String, String>)],
) -> String {
    let mut prompt = format!(
        "次の質問に対する複数のAI回答を比較し、良い部分を統合して最終回答を1つ作ってください。\n\n質問:\n{task}\n\n"
    );
    for (index, (selection, assigned_task, result)) in candidates.iter().enumerate() {
        prompt.push_str(&format!(
            "--- 候補{}: {}（{} / {}） ---\n",
            index + 1,
            selection.name,
            selection.provider,
            selection.model
        ));
        if assigned_task != task {
            prompt.push_str(&format!("担当した部分:\n{assigned_task}\n\n"));
        }
        match result {
            Ok(answer) => prompt.push_str(answer),
            Err(error) => prompt.push_str(&format!("［取得失敗: {error}］")),
        }
        prompt.push_str("\n\n");
    }
    prompt
}

async fn merge_candidates(
    cfg: &Config,
    request: &MultiRequest,
    candidates: &[(AiSelection, String, Result<String, String>)],
) -> Result<(AiSelection, String), String> {
    let provider = providers::get(&request.judge.provider).ok_or_else(|| {
        format!(
            "統合用プロバイダ \"{}\" が見つかりません。",
            request.judge.provider
        )
    })?;
    let model = if request.judge.model.is_empty() {
        provider.default_model(cfg)
    } else {
        request.judge.model.clone()
    };
    let mut judge = request.judge.clone();
    judge.model = model.clone();
    let messages = vec![
        Message::new(
            "system",
            "あなたは複数のAI回答を統合する編集者です。候補を盲目的に多数決せず、質問への適合性、事実の整合、具体性、抜け、注意点を比較してください。候補間で矛盾し確認できない点は断定せず明示してください。候補や統合手順の説明はせず、利用者へ返す完成した回答だけを書いてください。",
        ),
        Message::new("user", candidates_for_judge(&request.task, candidates)),
    ];
    let response = provider
        .chat(cfg, &model, &messages, &[])
        .await
        .map_err(|error| error.to_string())?;
    let answer = response.content.trim().to_string();
    if answer.is_empty() {
        return Err("統合AIが空の回答を返しました。".to_string());
    }
    Ok((judge, answer))
}

fn fallback_candidates(candidates: &[(AiSelection, String, Result<String, String>)]) -> String {
    let mut output = String::from("統合AIでまとめられなかったため、候補回答を個別に返します。");
    for (selection, assigned_task, result) in candidates {
        output.push_str(&format!(
            "\n\n【{}（{} / {}）】\n",
            selection.name, selection.provider, selection.model
        ));
        output.push_str(&format!("担当: {assigned_task}\n"));
        match result {
            Ok(answer) => output.push_str(answer),
            Err(error) => output.push_str(&format!("エラー: {error}")),
        }
    }
    output
}

fn parse_planned_tasks(text: &str, expected: usize) -> Option<Vec<String>> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let value: Value = serde_json::from_str(&text[start..=end]).ok()?;
    let tasks = value.get("tasks")?.as_array()?;
    if tasks.len() != expected {
        return None;
    }
    let tasks = tasks
        .iter()
        .map(|task| task.as_str().unwrap_or("").trim().to_string())
        .collect::<Vec<_>>();
    if tasks.iter().any(String::is_empty) {
        return None;
    }
    Some(tasks)
}

async fn assigned_tasks(cfg: &Config, request: &MultiRequest) -> Vec<String> {
    if request.mode != "collaborate" && request.mode != "divide" {
        return vec![request.task.clone(); request.agents.len()];
    }
    let Some(provider) = providers::get(&request.judge.provider) else {
        return vec![request.task.clone(); request.agents.len()];
    };
    let model = if request.judge.model.is_empty() {
        provider.default_model(cfg)
    } else {
        request.judge.model.clone()
    };
    let members = request
        .agents
        .iter()
        .enumerate()
        .map(|(index, agent)| {
            format!(
                "{}: {}（{} / {}）{}",
                index + 1,
                agent.name,
                agent.provider,
                if agent.model.is_empty() { "既定" } else { &agent.model },
                if agent.focus.is_empty() {
                    String::new()
                } else {
                    format!("／担当の希望: {}", agent.focus)
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let messages = vec![
        Message::new(
            "system",
            "あなたは複数AIへ仕事を分ける進行役です。元の依頼全体を解決できるよう、重複を抑えつつ相互に補完する独立タスクへ分解してください。担当の希望が書かれているAIはその内容を優先し、空なら依頼に合う仕事を割り当ててください。各担当だけで実行可能な具体的指示にし、必ず指定件数と同じ順序のJSONだけを返してください。形式は {\"tasks\":[\"担当1\",\"担当2\"]} です。",
        ),
        Message::new(
            "user",
            format!(
                "元の依頼:\n{}\n\n担当AI:\n{}\n\n{}件へ分けてください。",
                request.task,
                members,
                request.agents.len()
            ),
        ),
    ];
    match provider.chat(cfg, &model, &messages, &[]).await {
        Ok(response) => parse_planned_tasks(&response.content, request.agents.len())
            .unwrap_or_else(|| vec![request.task.clone(); request.agents.len()]),
        Err(_) => vec![request.task.clone(); request.agents.len()],
    }
}

#[async_trait]
impl Tool for SubagentTool {
    fn name(&self) -> &'static str {
        "subagent"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "サブエージェント",
            "マルチエージェント",
            "マルチai",
            "複数のai",
            "複数ai",
            "複数の叩くai",
            "回答を比較",
            "回答を統合",
            "分担",
            "手分け",
            "subagent",
            "multi-agent",
        ]
    }

    fn label(&self) -> &'static str {
        "サブエージェント／マルチAI合議"
    }

    fn description(&self) -> String {
        "1つの集中タスクを別のエージェントへ委譲する。または、同じ質問を指定した複数の叩くAIへ送り、候補回答の正確さ・具体性・抜けを統合AIが比較して、良い部分をまとめた回答を1つ返す。agentsを指定するとマルチAI合議になる。候補数に加えて統合1回分の推論・API利用が発生する。内蔵ローカルAI同士は直列、その他は並列上限内で実行する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "単一サブエージェント、または複数AI全員へ送る共通の質問・依頼"
                },
                "context": {
                    "type": "string",
                    "description": "質問を進めるうえで共通の前提情報（任意）"
                },
                "mode": {
                    "type": "string",
                    "description": "consensus=全AIへ同じ質問をして統合、collaborate=仕事を分けて統合、divide=仕事を分けて個別結果を返す"
                },
                "steps": {
                    "type": "integer",
                    "description": "各候補AIの最大ループ数（既定6、1〜12）"
                },
                "agents": {
                    "type": "array",
                    "description": "同じtaskへ回答させる叩くAIを2〜8件指定。省略時は従来どおり単一サブエージェント",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "結果識別用の任意名" },
                            "provider": { "type": "string", "description": "local / ollama / openai / anthropic / gemini / deepseek / xai / mistral / agent_cli。省略時は現在選択中" },
                            "model": { "type": "string", "description": "モデル名。agent_cliではclaudeまたはcodexを指定可能。省略時はプロバイダ既定" }
                            ,"focus": { "type": "string", "description": "仕事分担時に優先する担当内容。空なら統合AIが自動で割り当てる" }
                        }
                    }
                },
                "judge": {
                    "type": "object",
                    "description": "候補回答を比較・統合するAI。省略時は現在選択中の叩くAI",
                    "properties": {
                        "name": { "type": "string", "description": "結果識別用の任意名" },
                        "provider": { "type": "string", "description": "統合に使うプロバイダid" },
                        "model": { "type": "string", "description": "統合に使うモデル名" }
                    }
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "候補AIの最大並列数（既定は設定値3、1〜8）。内蔵localは常に1"
                }
            },
            "required": ["task"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "この質問をローカルAI、Gemini、Claudeへ聞いて、良い回答をまとめて",
            "GeminiとCodexへ同じ質問をして、最終回答を1つにして",
            "この資料をサブエージェントに要約させて",
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
        let task = match bounded_text(str_arg(&args, "task"), "task") {
            Ok(task) if !task.is_empty() => task,
            Ok(_) => return "エラー: サブエージェントへ渡す task が空です。".to_string(),
            Err(error) => return error,
        };

        let max_depth = cfg
            .get("PICOAGENT_SUBAGENT_MAX_DEPTH", "2")
            .parse::<usize>()
            .unwrap_or(2)
            .max(1);
        if agent::agent_depth() >= max_depth {
            return format!(
                "エラー: サブエージェントのネストが上限（{max_depth}）に達したため、これ以上は委譲できません。このタスクは自分で続行してください。"
            );
        }

        let request = match parse_multi_request(cfg, &args) {
            Ok(Some(request)) => request,
            Ok(None) => {
                let id = cfg.get("PICOAGENT_PROVIDER", "local");
                let provider = match providers::get(&id) {
                    Some(provider) => provider,
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
                let context = match bounded_text(str_arg(&args, "context"), "context") {
                    Ok(context) => context,
                    Err(error) => return error,
                };
                let mut messages = vec![
                    Message::new("system", system_prompt(&context)),
                    Message::new("user", task),
                ];
                return match agent::run(cfg, provider, &model, &mut messages, steps, None, None).await {
                    Ok(result) if !result.trim().is_empty() => {
                        format!("【サブエージェントの結果】\n{}", result.trim())
                    }
                    Ok(_) => "サブエージェントは結果を返しませんでした。".to_string(),
                    Err(error) => format!("エラー: サブエージェントの実行に失敗しました: {error}"),
                };
            }
            Err(error) => return error,
        };

        let tasks = assigned_tasks(cfg, &request).await;
        let local_gate = Arc::new(futures::lock::Mutex::new(()));
        let mut candidates = stream::iter(request.agents.clone().into_iter().zip(tasks).map(|(selection, assigned_task)| {
            run_candidate(
                cfg,
                assigned_task,
                &request.context,
                request.steps,
                selection,
                Arc::clone(&local_gate),
            )
        }))
        .buffer_unordered(request.max_concurrency)
        .collect::<Vec<_>>()
        .await;

        candidates.sort_by_key(|(selection, _, _)| {
            request
                .agents
                .iter()
                .position(|candidate| candidate == selection)
                .unwrap_or(usize::MAX)
        });

        let successful = candidates
            .iter()
            .filter(|(_, _, result)| result.is_ok())
            .count();
        if successful == 0 {
            return fallback_candidates(&candidates);
        }
        if request.mode == "divide" {
            return format!(
                "【マルチAIの分担結果】\n{}\n\n［担当{}件中{}件成功／最終統合なし］",
                fallback_candidates(&candidates)
                    .trim_start_matches("統合AIでまとめられなかったため、候補回答を個別に返します。")
                    .trim(),
                candidates.len(),
                successful
            );
        }

        match merge_candidates(cfg, &request, &candidates).await {
            Ok((judge, answer)) => format!(
                "【マルチAI合議の統合結果】\n{}\n\n［候補{}件中{}件成功／統合: {}（{} / {}）］",
                answer.trim(),
                candidates.len(),
                successful,
                judge.name,
                judge.provider,
                judge.model
            ),
            Err(error) => format!("{}\n\n統合エラー: {error}", fallback_candidates(&candidates)),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(SubagentTool) } }

#[cfg(test)]
mod tests {
    use super::{parse_multi_request, AiSelection};
    use crate::config::Config;
    use serde_json::json;

    #[test]
    fn 複数aiへ同じ質問と個別モデルを指定できる() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_PROVIDER", "local");
        let request = parse_multi_request(
            &cfg,
            &json!({
                "task": "同じ質問",
                "agents": [
                    {"provider": "local", "name": "手元"},
                    {"provider": "gemini", "model": "gemini-test", "focus": "調査"},
                    {"provider": "agent_cli", "model": "codex"}
                ],
                "judge": {"provider": "anthropic", "model": "claude-test"},
                "max_concurrency": 8
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(request.task, "同じ質問");
        assert_eq!(request.max_concurrency, 8);
        assert_eq!(request.agents[0].name, "手元");
        assert_eq!(request.agents[1].model, "gemini-test");
        assert_eq!(request.agents[1].focus, "調査");
        assert_eq!(request.agents[2].model, "codex");
        assert_eq!(
            request.judge,
            AiSelection {
                name: "統合AI".to_string(),
                provider: "anthropic".to_string(),
                model: "claude-test".to_string(),
                focus: String::new(),
            }
        );
    }

    #[test]
    fn マルチai合議は二件未満と上限超過を拒否する() {
        let cfg = Config::new();
        let one = parse_multi_request(
            &cfg,
            &json!({"task": "質問", "agents": [{"provider": "local"}]}),
        )
        .unwrap_err();
        assert!(one.contains("2件以上"));

        let too_many = (0..9)
            .map(|_| json!({"provider": "local"}))
            .collect::<Vec<_>>();
        let error = parse_multi_request(
            &cfg,
            &json!({"task": "質問", "agents": too_many}),
        )
        .unwrap_err();
        assert!(error.contains("最大 8 件"));
    }
}
