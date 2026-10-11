//! 本番の文脈・送信変換を共有する、合成教材と開発評価の出力入口。
//! このモジュールはテスト時だけ読み込み、利用者設定や保存履歴を参照しない。

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::*;

#[derive(Deserialize)]
struct Character {
    name: String,
    profile: String,
}

#[derive(Deserialize)]
struct CaseInput {
    character: Character,
    memories: Vec<String>,
    #[serde(default)]
    history: Vec<Message>,
    question: String,
    #[serde(default)]
    continuation: Vec<Message>,
    #[serde(default)]
    tools: Vec<Value>,
    #[serde(default)]
    tool_names: Vec<String>,
}

struct FixtureEnvironment {
    cfg: Config,
    tool_schemas: std::collections::HashMap<&'static str, &'static Value>,
    _directory: tempfile::TempDir,
}

impl FixtureEnvironment {
    fn new() -> Result<Self> {
        let scratch =
            crate::app_paths::test_workspace_dir(crate::app_paths::TestWorkspace::LoraFixtures)
                .map_err(anyhow::Error::msg)?;
        std::fs::create_dir_all(&scratch)?;
        let directory = tempfile::tempdir_in(scratch)?;
        let cfg = Config::new();
        // 空文字は環境変数へフォールバックするため、既定値を明示する。
        // 専用の空ディレクトリを使い、実際のAGENTS.mdも教材へ混ぜない。
        cfg.set(
            "PICOAGENT_ALLOWED_ROOT",
            directory.path().to_string_lossy().as_ref(),
        );
        cfg.set(
            "PICOAGENT_SYSTEM_PROMPT",
            super::super::prompt::DEFAULT_SYSTEM_PROMPT,
        );
        cfg.set(
            "PICOAGENT_RELATIONSHIP",
            crate::relationship_preset::DEFAULT_RELATIONSHIP,
        );
        cfg.set("PICOAGENT_RELATIONSHIP_OVERRIDES", "{}");
        // 本番の宣言を一度だけ索引化し、教材ごとの全件検索を繰り返さない。
        let tool_schemas = crate::tools::build_schemas()
            .iter()
            .map(|schema| {
                let name = schema["function"]["name"]
                    .as_str()
                    .context("登録済みツール定義の名前がありません")?;
                Ok((name, schema))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            cfg,
            tool_schemas,
            _directory: directory,
        })
    }

    fn body(&self, input: &CaseInput) -> Result<Value> {
        anyhow::ensure!(
            !input.character.name.trim().is_empty()
                && !input.character.profile.trim().is_empty()
                && !input.question.trim().is_empty(),
            "キャラの名前・人格・今回の質問は空にできません"
        );
        for message in input.history.iter().chain(&input.continuation) {
            anyhow::ensure!(
                matches!(message.role.as_str(), "user" | "assistant" | "tool"),
                "教材履歴の役割が不正です。人格と非公開資料は専用項目へ指定してください"
            );
            anyhow::ensure!(
                message.images.as_ref().is_none_or(Vec::is_empty),
                "日本語1.2Bの学習教材へ画像は指定できません"
            );
            if let Some(calls) = &message.tool_calls {
                anyhow::ensure!(
                    !calls.is_empty()
                        && calls
                            .iter()
                            .all(|call| !call.name.is_empty() && call.arguments.is_object()),
                    "正規形履歴のツール名または引数が不正です"
                );
            }
        }
        // questionの後ろは同じ依頼のツール往復だけ。次の質問を紛れ込ませない。
        anyhow::ensure!(
            input.continuation.iter().all(|message| {
                message.role == "tool"
                    || (message.role == "assistant" && message.tool_calls.is_some())
            }),
            "continuationにはツール呼出と対応する結果だけを指定してください"
        );
        let tools = self.resolve_tools(input)?;
        let mut public = vec![Message::new(
            "system",
            super::super::prompt::compose_content(
                &self.cfg,
                &input.character.profile,
                &input.character.name,
            ),
        )];
        public.extend_from_slice(&input.history);
        public.push(Message::new("user", &input.question));
        public.extend_from_slice(&input.continuation);
        let original = public.clone();
        let store = crate::memory::MemoryStore {
            items: input
                .memories
                .iter()
                .map(|text| crate::memory::MemoryItem {
                    category: "好み".into(),
                    text: text.clone(),
                    updated_at: "2026-10-11T00:00:00Z".into(),
                    hits: 1,
                })
                .collect(),
        };
        let mut context = TurnContext::default();
        context.push_owned(
            ContextSubject::UserMemory,
            "記憶",
            crate::memory::context_from_store(&store),
        );
        let session = TurnContextSession::new(
            ProtocolHistory::default(),
            &public,
            context,
            32768,
            Arc::new(Mutex::new(ToolSchemaSet::default())),
            1,
        );
        let raw = session.protocol_view(&public)?;
        anyhow::ensure!(equal(&public, &original), "公開履歴が書き換わりました");
        anyhow::ensure!(
            raw.iter().filter(|message| !is_private(message)).count() == public.len(),
            "文脈上限で教材の履歴が削られました。根拠を保てる長さへ直してください"
        );
        crate::providers::local::fixture_chat_body("lfm25_1_2b_jp", &raw, &tools)
    }

    fn resolve_tools(&self, input: &CaseInput) -> Result<Vec<Value>> {
        anyhow::ensure!(
            input.tool_names.is_empty() || input.tools.is_empty(),
            "tool_namesと非空のtoolsを同時には指定できません"
        );
        let tools = if input.tool_names.is_empty() {
            input.tools.clone()
        } else {
            input
                .tool_names
                .iter()
                .map(|name| {
                    self.tool_schemas
                        .get(name.as_str())
                        .map(|schema| (*schema).clone())
                        .ok_or_else(|| anyhow::anyhow!("登録されていないツールです: {name}"))
                })
                .collect::<Result<Vec<_>>>()?
        };
        let mut names = HashSet::new();
        for tool in &tools {
            let name = tool["function"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| anyhow::anyhow!("ツール定義の名前がありません"))?;
            anyhow::ensure!(
                tool["type"] == "function" && names.insert(name),
                "ツール定義の形式または名前の重複が不正です: {name}"
            );
        }
        Ok(tools)
    }
}

fn export_document(mut document: Value) -> Result<Value> {
    let environment = FixtureEnvironment::new()?;
    let cases = document["cases"]
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("cases配列がありません"))?;
    anyhow::ensure!(!cases.is_empty(), "cases配列が空です");
    let mut ids = HashSet::new();
    for case in cases {
        let id = case["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("ケースのidがありません"))?
            .to_owned();
        anyhow::ensure!(ids.insert(id.clone()), "ケースのidが重複しています: {id}");
        anyhow::ensure!(
            case["target"]["role"] == "assistant",
            "学習対象はassistant回答だけを指定してください: {id}"
        );
        let input: CaseInput =
            serde_json::from_value(case.clone()).with_context(|| format!("教材の形式: {id}"))?;
        case["body"] = environment
            .body(&input)
            .with_context(|| format!("本番入力へ変換できません: {id}"))?;
    }
    Ok(document)
}

#[test]
#[ignore = "LORA_FIXTURE_INPUTとLORA_FIXTURE_OUTPUTで合成教材の入出力を明示する"]
fn lora教材を本番入力で出力する() -> Result<()> {
    let input = std::path::PathBuf::from(
        std::env::var_os("LORA_FIXTURE_INPUT")
            .ok_or_else(|| anyhow::anyhow!("LORA_FIXTURE_INPUTを指定してください"))?,
    );
    let output = std::path::PathBuf::from(
        std::env::var_os("LORA_FIXTURE_OUTPUT")
            .ok_or_else(|| anyhow::anyhow!("LORA_FIXTURE_OUTPUTを指定してください"))?,
    );
    anyhow::ensure!(input != output, "入力教材と出力先は別にしてください");
    anyhow::ensure!(
        output.canonicalize().ok().as_ref() != Some(&input.canonicalize()?),
        "入力教材と出力先が同じファイルを指しています"
    );
    anyhow::ensure!(
        std::fs::metadata(&input)?.len() <= 32_000_000,
        "入力教材が上限32MBを超えています"
    );
    let document = serde_json::from_str(&std::fs::read_to_string(input)?)?;
    crate::store::write_json_atomic(&output, &export_document(document)?)
}

#[test]
fn 帰属対照四十ケースは原文と履歴を保持して本番送信変換を通る() -> Result<()> {
    let environment = FixtureEnvironment::new()?;
    let fixture: Value = serde_json::from_str(include_str!("fixtures/ownership-cases.json"))?;
    let mut exported = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let switched = case["carry"]["mode"] == "character-switch";
        let (name, drink) = if switched {
            ("はな", "ミルク")
        } else {
            ("むぎ", "コーヒー")
        };
        let profile = format!("名前: {name}。穏やかに短く相談へ答える。\nキャラクター「{name}」の好きな飲み物は{drink}です。好きな食べ物はプリン、趣味は読書です。");
        let history = if switched {
            Vec::new()
        } else {
            case["carry"]["turns"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|turn| {
                    [
                        Message::new("user", turn["user"].as_str().unwrap()),
                        Message::new("assistant", turn["assistant"].as_str().unwrap()),
                    ]
                })
                .collect()
        };
        let input: CaseInput = serde_json::from_value(json!({
            "character": {"name": name, "profile": profile},
            "memories": ["私は緑茶が好きです", "私はカレーが好きです", "私の趣味は写真撮影です", "私の名前はゆうです"],
            "history": history, "question": case["question"],
        }))?;
        let mut row = case.clone();
        row["id"] = json!(format!("tracked16/{}", case["name"].as_str().unwrap()));
        row["split"] = json!("dev");
        row["expected"] = json!({
            "expectedSubject": case["expectedSubject"],
            "expectedFacts": case["expectedFacts"],
            "reject": case["reject"],
        });
        row["body"] = environment.body(&input)?;
        let last = row["body"]["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(last.contains(&format!(
            "{QUESTION_BEGIN}\n{}\n{QUESTION_END}",
            input.question
        )));
        assert!(last.contains("各記録の主体を参照"));
        let binding = super::super::referents::bind(&input.question);
        if !binding.directive.is_empty() {
            assert!(last.contains(&binding.resolved));
        }
        exported.push(row);
    }
    assert_eq!(exported.len(), 16);
    let holdout: Value =
        serde_json::from_str(include_str!("fixtures/ownership-holdout-cases.json"))?;
    for suite in holdout["suites"].as_array().unwrap() {
        for case in suite["cases"].as_array().unwrap() {
            let input: CaseInput = serde_json::from_value(json!({
                "character": suite["character"],
                "memories": suite["memories"][case["memory"].as_str().unwrap()],
                "question": case["question"],
            }))?;
            let mut row = case.clone();
            row["id"] = json!(format!(
                "additional24/{}/{}",
                suite["name"].as_str().unwrap(),
                case["name"].as_str().unwrap()
            ));
            row["split"] = json!("dev");
            row["suite"] = suite["name"].clone();
            row["body"] = environment.body(&input)?;
            exported.push(row);
        }
    }
    assert_eq!(exported.len(), 40);
    let mut ids = HashSet::new();
    let mut counts = [0, 0];
    for row in &exported {
        let id = row["id"].as_str().unwrap();
        assert!(ids.insert(id), "開発評価のIDが重複しています: {id}");
        assert_eq!(row["split"], "dev");
        assert!(row["question"]
            .as_str()
            .is_some_and(|question| !question.is_empty()));
        assert!(row["body"]["messages"]
            .as_array()
            .is_some_and(|messages| !messages.is_empty()));
        if id.starts_with("tracked16/") {
            counts[0] += 1;
            for field in ["expectedSubject", "expectedFacts", "reject"] {
                assert_eq!(row["expected"][field], row[field]);
                assert!(row["expected"][field]
                    .as_array()
                    .is_some_and(|values| !values.is_empty()));
            }
        } else {
            assert!(id.starts_with("additional24/"));
            counts[1] += 1;
            assert!(row["expected"]
                .as_str()
                .is_some_and(|expected| !expected.is_empty()));
        }
    }
    assert_eq!(counts, [16, 24]);
    if let Some(path) = std::env::var_os("OWNERSHIP_FIXTURE_OUTPUT") {
        crate::store::write_json_atomic(
            std::path::Path::new(&path),
            &json!({
                "schema_version": 1,
                "metadata": fixture["metadata"],
                "additionalMetadata": holdout["metadata"],
                "cases": exported,
            }),
        )?;
    }
    Ok(())
}

fn sample_document() -> Value {
    json!({
        "metadata": {"split": "train", "source": "合成", "seed": 17},
        "cases": [{
            "id": "訂正", "group_id": "対の元", "category": "訂正",
            "character": {"name": "つむぎ", "profile": "名前: つむぎ。好きな飲み物は白湯。"},
            "memories": ["私の名前はなおです", "私はほうじ茶が好きです"],
            "history": [{"role":"user", "content":"私の好きな飲み物は？"},
                        {"role":"assistant", "content":"あなたは白湯が好きです。"}],
            "question": "私の好みを資料から確認して訂正して。",
            "tools": [],
            "target": {"role":"assistant", "content":"なおさんの好みはほうじ茶です。先ほどの白湯という回答を訂正します。"},
            "expected": {"owner":"なお", "value":"ほうじ茶"}
        }]
    })
}

#[test]
fn lora出力は教材の根拠と誤答履歴とメタ情報を保持し正解を入力へ混ぜない() -> Result<()> {
    let input = sample_document();
    let output = export_document(input.clone())?;
    let mut restored = output.clone();
    restored["cases"][0].as_object_mut().unwrap().remove("body");
    assert_eq!(restored, input);
    let body = &output["cases"][0]["body"];
    assert_eq!(body["temperature"], json!(0.6));
    assert_eq!(body["model"], "lfm25_1_2b_jp");
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages[2]["content"],
        input["cases"][0]["history"][1]["content"]
    );
    let text = serde_json::to_string(body)?;
    assert!(text.contains("ほうじ茶"));
    assert!(!text.contains("先ほどの白湯という回答を訂正します"));
    assert!(!text.contains(crate::project_rules::CONTEXT_BEGIN));
    assert!(!text.contains("ユーザーとの関係:"));
    Ok(())
}

#[test]
fn lora出力は隔離先が違っても同じ本番入力になり一時パスを混ぜない() -> Result<()> {
    let first = FixtureEnvironment::new()?;
    let second = FixtureEnvironment::new()?;
    assert_ne!(first._directory.path(), second._directory.path());
    let input: CaseInput = serde_json::from_value(sample_document()["cases"][0].clone())?;
    let first_body = first.body(&input)?;
    let second_body = second.body(&input)?;
    assert_eq!(first_body, second_body);
    let prompt = first_body["messages"][0]["content"].as_str().unwrap();
    for environment in [&first, &second] {
        assert!(!prompt.contains(environment._directory.path().to_string_lossy().as_ref()));
    }
    assert!(!prompt.contains(crate::project_rules::CONTEXT_BEGIN));
    Ok(())
}

#[test]
fn lora出力は本番ツール定義と当ターンの呼出結果を同じ順序で渡す() -> Result<()> {
    let mut input = sample_document();
    let case = &mut input["cases"][0];
    case["history"] = json!([]);
    case["question"] = json!("東京の天気を確認して。");
    case["tool_names"] = json!(["weather"]);
    case["continuation"] = json!([
        {"role":"assistant", "content":"", "tool_calls":[
            {"id":"天気1", "name":"weather", "arguments":{"city":"東京"}}
        ]},
        {"role":"tool", "tool_call_id":"天気1", "content":"東京は晴れ。"}
    ]);
    case["target"] = json!({"role":"assistant", "content":"確認した東京の天気は晴れです。"});
    let output = export_document(input.clone())?;
    let body = &output["cases"][0]["body"];
    let schema = crate::tools::build_schemas()
        .iter()
        .find(|schema| schema["function"]["name"] == "weather")
        .unwrap();
    assert_eq!(body["tools"], json!([schema]));
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .contains("東京の天気を確認して。"));
    assert_eq!(
        messages[2]["tool_calls"][0]["function"]["arguments"],
        r#"{"city":"東京"}"#
    );
    assert_eq!(messages[3]["tool_call_id"], "天気1");
    assert_eq!(
        output["cases"][0]["continuation"],
        input["cases"][0]["continuation"]
    );
    Ok(())
}

#[test]
fn lora出力は壊れた教材と未知ツールを黙って補完しない() {
    for invalid in [
        json!({"tool_names":["存在しないツール"]}),
        json!({"tool_names":["weather", "weather"]}),
        json!({"question":""}),
        json!({"history":[{"role":"system", "content":"別の人格"}]}),
        json!({"continuation":[{"role":"user", "content":"別の質問"}]}),
        json!({"continuation":[{"role":"tool", "tool_call_id":"孤立", "content":"不明"}]}),
        json!({"continuation":[{"role":"assistant", "tool_calls":[{"id":"x", "name":"weather", "arguments":"{壊れ"}]}]}),
        json!({"target":{"role":"user", "content":"回答ではない"}}),
    ] {
        let mut input = sample_document();
        input["cases"][0]
            .as_object_mut()
            .unwrap()
            .extend(invalid.as_object().unwrap().clone());
        assert!(export_document(input).is_err(), "拒否すべき入力: {invalid}");
    }
    let mut duplicate = sample_document();
    let case = duplicate["cases"][0].clone();
    duplicate["cases"].as_array_mut().unwrap().push(case);
    assert!(export_document(duplicate).is_err());
}
