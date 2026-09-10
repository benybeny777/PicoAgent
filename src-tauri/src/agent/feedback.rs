//! 計画の事前生成と、最終回答が明らかに不十分な場合の再生成を担当する。

use crate::config::Config;
use crate::message::Message;
use crate::providers::Provider;

use super::{agent_depth, super_agent_enabled};

fn extract_json_object(content: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) {
        if value.is_object() {
            return Some(value);
        }
    }
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if start <= end {
        serde_json::from_str::<serde_json::Value>(&content[start..=end]).ok()
    } else {
        None
    }
}

fn is_complex_query(text: &str) -> bool {
    let count = text.chars().count();
    if count < 60 {
        return false;
    }
    let questions = text.matches('?').count() + text.matches('？').count();
    count > 120
        || questions >= 2
        || text.contains("手順")
        || text.contains("ステップ")
        || text.contains("step")
}

pub(super) async fn planning_step(
    provider: &dyn Provider,
    cfg: &Config,
    model: &str,
    user_text: &str,
) -> Option<String> {
    if !cfg.get_bool("PICOAGENT_PLANNING_ENABLED", false) && !super_agent_enabled(cfg) {
        return None;
    }
    if agent_depth() > 1 || !is_complex_query(user_text) {
        return None;
    }
    let plan_messages = vec![
        Message::new(
            "system",
            "あなたはタスク分解AIです。ユーザーの依頼を最大4つのサブタスクに分解して、必ず次のJSON形式だけで返してください。{\"steps\":[\"手順1\",\"手順2\",...]}",
        ),
        Message::new("user", user_text),
    ];
    let result = provider
        .chat_with_options(cfg, model, &plan_messages, &[], true)
        .await
        .ok()?;
    let plan = extract_json_object(&result.content)?;
    let steps = plan.get("steps")?.as_array()?;
    if steps.len() <= 1 {
        return None;
    }
    let plan_lines: Vec<String> = steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| step.as_str().map(|text| format!("{}. {}", index + 1, text)))
        .collect();
    Some(format!("【実行計画】\n{}", plan_lines.join("\n")))
}

pub(super) async fn self_improve(
    provider: &dyn Provider,
    cfg: &Config,
    model: &str,
    messages: &[Message],
    answer: String,
) -> String {
    if !cfg.get_bool("PICOAGENT_SELF_EVAL_ENABLED", false) && !super_agent_enabled(cfg) {
        return answer;
    }
    if agent_depth() > 1 {
        return answer;
    }
    let too_short = answer.trim().chars().count() < 8;
    let refusal = (answer.contains("申し訳") && answer.contains("できません"))
        || answer.contains("お答えできません");
    if !too_short && !refusal {
        return answer;
    }
    let clean: Vec<Message> = messages
        .iter()
        .filter(|message| {
            message.role == "system"
                || message.role == "user"
                || (message.role == "assistant"
                    && message.tool_calls.is_none()
                    && !message.content.is_empty())
        })
        .cloned()
        .collect();
    // フォールバック許可: 再生成失敗時は元の回答を返す（会話を止めない）。
    match provider.chat(cfg, model, &clean, &[]).await {
        Ok(message) if !message.content.is_empty() => message.content,
        _ => answer,
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_json_object, is_complex_query};

    #[test]
    fn 文中の計画jsonを抽出する() {
        let value =
            extract_json_object("計画: ```json\n{\"steps\":[\"調査\",\"実装\"]}\n```").unwrap();
        assert_eq!(value["steps"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn 短い単一質問は追加の計画推論を起動しない() {
        assert!(!is_complex_query("今日の天気は？"));
        assert!(is_complex_query(&format!(
            "{}？{}？",
            "手順を説明して".repeat(10),
            "検証方法も教えて"
        )));
    }
}
