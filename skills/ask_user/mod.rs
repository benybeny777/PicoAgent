// skills/ask_user/mod.rs - 選択肢つきでユーザーに判断を仰ぐスキル。
//
// 前提が足りないまま作業を進めて的外れな結果を出すより、1回聞いたほうが早い場面がある。
// Claude Code の AskUserQuestion に相当する。ツール結果は文字列しか返せないため、
// ここでは質問（マーカー）だけを返し、エージェントループを止めてユーザーへ提示する。
// 答えは次のユーザー発話として普通に会話へ入るので、モデルはそのまま続きを進められる。
//
// 「聞けば分かること」を何でも聞き返すと会話が煩雑になるだけなので、選択肢は 2〜4 個に
// 限り、既定値で進められる質問には使わない旨を説明文に書いている。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::text;

const MARKER: &str = "__PICOAGENT_USER_QUESTION__";
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 4;
const MAX_QUESTION_CHARS: usize = 300;
const MAX_LABEL_CHARS: usize = 40;
const MAX_DESCRIPTION_CHARS: usize = 120;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub question: String,
    pub options: Vec<QuestionOption>,
}

fn parse_option(item: &Value) -> Result<QuestionOption, String> {
    let label = item
        .as_str()
        .or_else(|| item.get("label").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    if label.is_empty() {
        return Err("選択肢の label が空です。".to_string());
    }
    Ok(QuestionOption {
        label: text::preview(label, MAX_LABEL_CHARS),
        description: text::preview(
            item.get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim(),
            MAX_DESCRIPTION_CHARS,
        ),
    })
}

fn parse_options(args: &Value) -> Result<Vec<QuestionOption>, String> {
    let list = args
        .get("options")
        .and_then(Value::as_array)
        .ok_or_else(|| "options（選択肢の配列）が指定されていません。".to_string())?;
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&list.len()) {
        return Err(format!(
            "options は {MIN_OPTIONS}〜{MAX_OPTIONS} 個で指定してください（今回は {} 個）。",
            list.len()
        ));
    }
    let mut options = Vec::with_capacity(list.len());
    for item in list {
        let option = parse_option(item)?;
        if options
            .iter()
            .any(|existing: &QuestionOption| existing.label == option.label)
        {
            return Err(format!("選択肢の label が重複しています: {}", option.label));
        }
        options.push(option);
    }
    Ok(options)
}

fn marker(pending: &PendingQuestion) -> String {
    match serde_json::to_string(pending) {
        Ok(encoded) => format!("{MARKER}{encoded}"),
        // 直列化に失敗したら質問文をそのまま返す（黙って質問を落とさない）。
        Err(error) => format!("エラー: 質問を組み立てられませんでした: {error}"),
    }
}

/// エージェントループが呼ぶ。マーカーなら保留中の質問を取り出す。
pub fn parse_marker(text: &str) -> Option<PendingQuestion> {
    let raw = text.strip_prefix(MARKER)?;
    serde_json::from_str(raw).ok()
}

/// ユーザーへ見せる質問文。
pub fn prompt(pending: &PendingQuestion) -> String {
    let mut out = pending.question.clone();
    for (index, option) in pending.options.iter().enumerate() {
        out.push_str(&format!("\n{}. {}", index + 1, option.label));
        if !option.description.is_empty() {
            out.push_str(&format!(" — {}", option.description));
        }
    }
    out.push_str("\n\n番号か選択肢の名前で答えてね。どれでもなければ自由に書いてもいいよ。");
    out
}

/// ユーザーの返答を選択肢へ寄せる。番号・ラベル一致なら label を、その他は None を返す
/// （None のときは自由記述として、書かれたままモデルへ渡す）。
pub fn resolve_answer(pending: &PendingQuestion, answer: &str) -> Option<String> {
    let answer = answer.trim();
    if answer.is_empty() {
        return None;
    }
    // 全角数字も受け付ける（フリック入力やIMEでそのまま送られることがある）。
    let normalized: String = answer
        .chars()
        .map(|c| match c {
            '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
            other => other,
        })
        .collect();
    let digits = normalized.trim_end_matches(['.', '。', '、', ')', '）']);
    if let Ok(number) = digits.parse::<usize>() {
        if let Some(option) = pending.options.get(number.wrapping_sub(1)) {
            return Some(option.label.clone());
        }
    }
    pending
        .options
        .iter()
        .find(|option| option.label.eq_ignore_ascii_case(answer))
        .map(|option| option.label.clone())
}

pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "どっち",
            "どれがいい",
            "選んで",
            "決めて",
            "おまかせ",
            "確認して",
        ]
    }

    fn label(&self) -> &'static str {
        "ユーザーに確認"
    }

    fn description(&self) -> String {
        "判断が分かれる点を、2〜4個の選択肢つきでユーザーに確認する。答えを聞かないと作業のやり直しになる場面だけに使う。妥当な既定値で進められること、調べれば分かることには使わない。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "確認したいこと。1文で具体的に。"
                },
                "options": {
                    "type": "array",
                    "description": "選ばせる候補（2〜4個）。おすすめがあれば先頭に置く。",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string", "description": "短い選択肢名。" },
                            "description": { "type": "string", "description": "選ぶとどうなるかの一言説明。" }
                        },
                        "required": ["label"]
                    }
                }
            },
            "required": ["question", "options"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec!["どっちがいいか聞いて決めて"]
    }

    async fn run(&self, _cfg: &Config, args: Value) -> String {
        let question = args
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if question.is_empty() {
            return "エラー: question が指定されていません。".to_string();
        }
        let options = match parse_options(&args) {
            Ok(options) => options,
            Err(error) => return format!("エラー: {error}"),
        };
        marker(&PendingQuestion {
            question: text::preview(question, MAX_QUESTION_CHARS),
            options,
        })
    }
}

inventory::submit! { ToolReg { make: || Box::new(AskUserTool) } }

#[cfg(test)]
mod tests {
    use super::{marker, parse_marker, parse_options, prompt, resolve_answer, PendingQuestion};
    use serde_json::json;

    fn sample() -> PendingQuestion {
        PendingQuestion {
            question: "どちらで進める？".to_string(),
            options: parse_options(&json!({
                "options": [
                    { "label": "そのまま", "description": "既存の形を保つ" },
                    { "label": "作り直す" }
                ]
            }))
            .unwrap(),
        }
    }

    #[test]
    fn 選択肢は2から4個に限る() {
        assert!(parse_options(&json!({ "options": ["A"] })).is_err());
        assert!(parse_options(&json!({ "options": ["A", "B", "C", "D", "E"] })).is_err());
        assert!(parse_options(&json!({ "options": ["A", "A"] })).is_err());
        assert!(parse_options(&json!({ "options": ["A", ""] })).is_err());
        // 文字列でもオブジェクトでも受け取れる。
        assert_eq!(
            parse_options(&json!({ "options": ["A", "B"] }))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn マーカーを往復できる() {
        let parsed = parse_marker(&marker(&sample())).expect("往復できない");
        assert_eq!(parsed.options.len(), 2);
        assert!(parse_marker("ただの文字列").is_none());
    }

    #[test]
    fn 質問文に番号つきの選択肢が並ぶ() {
        let text = prompt(&sample());
        assert!(
            text.contains("1. そのまま — 既存の形を保つ"),
            "text: {text}"
        );
        assert!(text.contains("2. 作り直す"), "text: {text}");
    }

    /// 番号・全角番号・ラベルのどれで答えても選択肢に寄せる。自由記述はそのまま通す。
    #[test]
    fn 返答を選択肢へ寄せる() {
        let pending = sample();
        assert_eq!(resolve_answer(&pending, "2").as_deref(), Some("作り直す"));
        assert_eq!(
            resolve_answer(&pending, "１。").as_deref(),
            Some("そのまま")
        );
        assert_eq!(
            resolve_answer(&pending, " 作り直す ").as_deref(),
            Some("作り直す")
        );
        assert_eq!(resolve_answer(&pending, "9"), None);
        assert_eq!(resolve_answer(&pending, "どっちでもいい"), None);
        assert_eq!(resolve_answer(&pending, ""), None);
    }
}
