// user_skill - 利用者が追加するMarkdown指示スキルを会話から管理・適用する。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;

pub struct UserSkillTool;

fn text_arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("").trim()
}

fn tags_arg(args: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(value) = args.get("tags") else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| "tagsは文字列の配列で指定してください。".to_string())?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "tagsの各要素は文字列で指定してください。".to_string())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

#[async_trait]
impl Tool for UserSkillTool {
    fn name(&self) -> &'static str {
        "user_skill"
    }

    fn label(&self) -> &'static str {
        "Markdownで追加できるAIスキル"
    }

    fn description(&self) -> String {
        "利用者のSKILL.mdを一覧・表示・作成・更新・適用する。利用者が『スキルを作って』『この手順をAIスキルにして』と頼んだらcreate、既存スキルに合う依頼ではuseで本文を読んでから従う。SKILL.mdは指示だけを持ち、実操作は登録済みツールの許可と承認に従う。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "show", "create", "update", "use"],
                    "description": "list=一覧、show=本文表示、create=新規作成、update=既存更新、use=本文を読み今回の依頼へ適用"
                },
                "name": {
                    "type": "string",
                    "description": "英小文字で始まる英小文字・数字・ハイフンの識別名。例: weekly-report"
                },
                "description": {
                    "type": "string",
                    "description": "何をするスキルかを改行なしで説明。createでは必須"
                },
                "when_to_use": {
                    "type": "string",
                    "description": "自動利用する発話例や起動条件。例: ユーザーが週報、進捗報告、今週の成果と言ったとき"
                },
                "tags": {
                    "type": "array",
                    "maxItems": 12,
                    "items": {"type": "string"},
                    "description": "候補選択を助ける短い分類語。Agent Skills標準のmetadata.tagsへ保存"
                },
                "disable_model_invocation": {
                    "type": "boolean",
                    "description": "trueなら自動候補へ出さず、利用者が名前を指定したときだけ使う"
                },
                "instructions": {
                    "type": "string",
                    "description": "SKILL.md本文。具体的な工程、出力形式、停止条件を書く。createでは必須。updateでは変更時だけ指定"
                },
                "input": {
                    "type": "string",
                    "description": "use時に、このスキルで扱う今回の依頼や素材"
                }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "週報を三項目にまとめるスキルを作って",
            "利用者追加スキルの一覧を見せて",
            "weekly-reportスキルでこのメモを週報にして",
        ]
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "スキルを作",
            "スキル作成",
            "aiスキル",
            "skill.md",
            "追加スキル",
            "スキル一覧",
            "スキルを使",
            "スキルで",
        ]
    }

    fn intent_priority(&self) -> i32 {
        80
    }

    async fn run(&self, config: &Config, args: Value) -> String {
        let action = text_arg(&args, "action");
        let name = text_arg(&args, "name");
        match action {
            "list" => crate::user_skills::list_text(config)
                .unwrap_or_else(|error| format!("エラー: {error}")),
            "show" => {
                if name.is_empty() {
                    return "エラー: showにはnameが必要です。".to_string();
                }
                crate::user_skills::show_text(config, name)
                    .unwrap_or_else(|error| format!("エラー: {error}"))
            }
            "create" => {
                let description = text_arg(&args, "description");
                let instructions = text_arg(&args, "instructions");
                if name.is_empty() || description.is_empty() || instructions.is_empty() {
                    return "エラー: createにはname、description、instructionsが必要です。"
                        .to_string();
                }
                let tags = match tags_arg(&args) {
                    Ok(tags) => tags.unwrap_or_default(),
                    Err(error) => return format!("エラー: {error}"),
                };
                let disable_model_invocation = args
                    .get("disable_model_invocation")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                match crate::user_skills::create(
                    config,
                    name,
                    description,
                    instructions,
                    text_arg(&args, "when_to_use"),
                    &tags,
                    disable_model_invocation,
                ) {
                    Ok(path) => {
                        let availability = if disable_model_invocation {
                            "自動候補には出ません。利用するときは名前を指定してください。"
                        } else {
                            "次の依頼から、名前、説明、用途、tagsに合うとき自動で候補になります。"
                        };
                        format!(
                            "Markdownスキルを作成しました: `{name}`\n保存先: {}\n{availability}",
                            path.display()
                        )
                    }
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "update" => {
                if name.is_empty() {
                    return "エラー: updateにはnameが必要です。".to_string();
                }
                let description = args.get("description").and_then(Value::as_str);
                let when_to_use = args.get("when_to_use").and_then(Value::as_str);
                let tags = match tags_arg(&args) {
                    Ok(tags) => tags,
                    Err(error) => return format!("エラー: {error}"),
                };
                let disable_model_invocation = args
                    .get("disable_model_invocation")
                    .and_then(Value::as_bool);
                let instructions = args.get("instructions").and_then(Value::as_str);
                if description.is_none()
                    && when_to_use.is_none()
                    && tags.is_none()
                    && disable_model_invocation.is_none()
                    && instructions.is_none()
                {
                    return "エラー: updateにはdescription、when_to_use、tags、disable_model_invocation、instructionsのいずれかが必要です。"
                        .to_string();
                }
                let patch = crate::user_skills::SkillPatch {
                    description,
                    when_to_use,
                    tags,
                    disable_model_invocation,
                    instructions,
                };
                match crate::user_skills::update(config, name, patch) {
                    Ok(path) => format!(
                        "Markdownスキルを更新しました: `{name}`\n保存先: {}",
                        path.display()
                    ),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "use" => {
                if name.is_empty() {
                    return "エラー: useにはnameが必要です。listで利用可能な名前を確認してください。"
                        .to_string();
                }
                crate::user_skills::use_text(config, name, text_arg(&args, "input"))
                    .unwrap_or_else(|error| format!("エラー: {error}"))
            }
            _ => "エラー: actionはlist/show/create/update/useのいずれかを指定してください。"
                .to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(UserSkillTool) } }

#[cfg(test)]
mod tests {
    use super::{Tool, UserSkillTool};
    use crate::config::Config;
    use serde_json::json;

    fn test_config() -> (Config, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "picoagent-user-skill-tool-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = Config::new();
        config.load(root.join("config.json")).unwrap();
        (config, root)
    }

    #[tokio::test]
    async fn チャット用toolから作成して利用できる() {
        let (config, root) = test_config();
        let tool = UserSkillTool;
        let created = tool
            .run(
                &config,
                json!({
                    "action": "create",
                    "name": "weekly-report",
                    "description": "週報を整える",
                    "when_to_use": "週報や今週の成果を頼まれたとき",
                    "tags": ["週報", "進捗"],
                    "instructions": "成果と次の課題を箇条書きにする。"
                }),
            )
            .await;
        assert!(created.contains("作成しました"));
        let used = tool
            .run(
                &config,
                json!({"action":"use","name":"weekly-report","input":"今週のメモ"}),
            )
            .await;
        assert!(used.contains("成果と次の課題"));
        assert!(used.contains("今週のメモ"));

        let invalid = tool
            .run(
                &config,
                json!({"action":"update","name":"weekly-report","tags":["週報", 1]}),
            )
            .await;
        assert!(invalid.contains("各要素は文字列"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
