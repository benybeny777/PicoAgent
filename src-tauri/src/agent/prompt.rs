//! エージェントへ渡す恒久ルールと会話別文脈を、一貫した順序で組み立てる。

use serde_json::{Map, Value};

use crate::config::Config;

const LEGACY_DEFAULT_SYSTEM_PROMPT: &str = "あなたは小さなデスクトップペットです。日本語で、親しみやすく簡潔に話します。選択中キャラクターの人格設定を守り、ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";
const PREVIOUS_DEFAULT_SYSTEM_PROMPT: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";
const PREVIOUS_DEFAULT_SYSTEM_PROMPT_2: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。「こんにちは」「こんにちわ」などの短い挨拶には表記の正誤を説明せず、自然に挨拶を返してください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";
const PREVIOUS_DEFAULT_SYSTEM_PROMPT_3: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。返答は必ず日本語で書いてください（ユーザーが他言語を明示的に求めたときだけ従ってください）。思考・推論の途中経過は出力せず、結論だけを話してください。いま選択されているキャラクター本人になりきり、他のキャラクターの名前・一人称・口調・性格を混ぜないでください。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。「こんにちは」「こんにちわ」などの短い挨拶には表記の正誤を説明せず、自然に挨拶を返してください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";
const PREVIOUS_DEFAULT_SYSTEM_PROMPT_4: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。返答は必ず日本語で書いてください（ユーザーが他言語を明示的に求めたときだけ従ってください）。思考・推論の途中経過は出力せず、結論だけを話してください。いま選択されている1キャラクター本人になりきり、他のキャラクターの名前・一人称・口調・性格を混ぜないでください。キャラ性は呼称で盛らず、反応の癖・言葉選び・得意な支え方で出してください。「お兄ちゃん」「お姉ちゃん」など、ユーザーを家族呼びして距離を詰める表現は禁止です。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。「こんにちは」「こんにちわ」などの短い挨拶には表記の正誤を説明せず、自然に挨拶を返してください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";

pub const DEFAULT_SYSTEM_PROMPT: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。返答は必ず日本語で書いてください（ユーザーが他言語を明示的に求めたときだけ従ってください）。思考・推論の途中経過は出力せず、結論だけを話してください。いま選択されている1キャラクター本人になりきり、他のキャラクターの名前・一人称・口調・性格を混ぜないでください。キャラ性は呼称で盛らず、反応の癖・言葉選び・得意な支え方で出してください。利用者が関係設定で明示した場合を除き、「お兄ちゃん」「お姉ちゃん」などの家族呼びで距離を詰めないでください。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。「こんにちは」「こんにちわ」などの短い挨拶には表記の正誤を説明せず、自然に挨拶を返してください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";

const PREVIOUS_DEFAULTS: &[&str] = &[
    LEGACY_DEFAULT_SYSTEM_PROMPT,
    PREVIOUS_DEFAULT_SYSTEM_PROMPT,
    PREVIOUS_DEFAULT_SYSTEM_PROMPT_2,
    PREVIOUS_DEFAULT_SYSTEM_PROMPT_3,
    PREVIOUS_DEFAULT_SYSTEM_PROMPT_4,
];

pub fn is_previous_default(value: &str) -> bool {
    PREVIOUS_DEFAULTS.contains(&value)
}

pub fn normalize_setting(values: &mut Map<String, Value>) {
    let should_update = values
        .get("PICOAGENT_SYSTEM_PROMPT")
        .and_then(Value::as_str)
        .is_some_and(is_previous_default);
    if should_update {
        values.insert(
            "PICOAGENT_SYSTEM_PROMPT".to_string(),
            Value::String(DEFAULT_SYSTEM_PROMPT.to_string()),
        );
    }
}

fn base_content(config: &Config) -> String {
    let configured = config.get("PICOAGENT_SYSTEM_PROMPT", DEFAULT_SYSTEM_PROMPT);
    let mut prompt = if is_previous_default(&configured) {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        configured
    };
    let capabilities = crate::tools::capability_summary();
    if !capabilities.is_empty() {
        prompt.push_str("\n\n使えるスキル（必要なときに自動で実行できます）: ");
        prompt.push_str(&capabilities);
        prompt.push_str("\n「何ができる？」「何が得意？」などと聞かれたら、上のスキルから役立つものをキャラクターの口調で具体的に挙げて教えてください。");
    }
    // 小型モデルは「今から調べるね、待ってて」と実行しない作業を約束しがちなため、毎回明示する。
    prompt.push_str("\n\nまだ実行していない作業について「今から調べる」「確認してみる」「ちょっと待ってて」のように後で行う約束をしないでください。調べる・作るなどが必要なら、その応答の中でスキルを実行して結果を伝え、実行できないときはできない理由を正直に伝えてください。");
    // 作業フォルダの AGENTS.md（この案件の作法）。許可ルートを切り替えると内容も入れ替わる。
    crate::project_rules::append_prompt_context(&mut prompt, config);
    prompt
}

fn finish(mut prompt: String) -> String {
    // 資料の帰属と質問との境界は固定規則。変動資料の包みへ毎回複写しない。
    // キャラプロフィールも含めた最後に、一つの入口から一度だけ付ける。
    prompt.push_str("\n\n");
    prompt.push_str(super::context::REFERENCE_RULES);
    prompt
}

/// 固定人格の本文を唯一の順序で組み立て、公開検査の共有状態は変更しない。
pub(crate) fn compose_content(
    config: &Config,
    character_profile: &str,
    character_id: &str,
) -> String {
    let mut prompt = base_content(config);
    if !character_profile.is_empty() {
        prompt.push_str("\n\n選択中キャラクターの人格設定:\n");
        prompt.push_str(character_profile);
        prompt.push_str(&crate::relationship_preset::profile_suffix(
            config,
            character_id,
        ));
    }
    finish(prompt)
}

pub fn compose(config: &Config, character_profile: &str, character_id: &str) -> String {
    let prompt = compose_content(config, character_profile, character_id);
    // 人格を組み立てたこの場所が、公開検査の照合元を更新する唯一の入り口。
    // 呼び出し側（会話の作り直し・キャラ切替・設定変更）へ登録を任せると、
    // 増えた経路のどれか1つが忘れた時点で人格設定が画面へ出る。
    super::remember_private_context(&prompt);
    prompt
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};

    use super::{compose, normalize_setting, DEFAULT_SYSTEM_PROMPT, PREVIOUS_DEFAULTS};
    use crate::config::Config;

    #[test]
    fn 過去の既定プロンプトをすべて現行へ移行する() {
        for previous in PREVIOUS_DEFAULTS {
            let mut values = Map::from_iter([(
                "PICOAGENT_SYSTEM_PROMPT".to_string(),
                Value::String((*previous).to_string()),
            )]);
            normalize_setting(&mut values);
            assert_eq!(
                values["PICOAGENT_SYSTEM_PROMPT"],
                json!(DEFAULT_SYSTEM_PROMPT)
            );
        }
    }

    #[test]
    fn 固定人格と明示した関係設定だけを正本へ保持する() {
        // `compose` は公開検査の照合元を更新するので、同じ静的変数を触るテストと直列化する。
        let _lock = crate::agent::PRIVATE_CONTEXT_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let config = Config::new();
        config.set("PICOAGENT_RELATIONSHIP", "sister_younger");
        let prompt = compose(&config, "名前: むぎ", "mugi");

        assert!(prompt.contains("利用者が関係設定で明示した場合を除き"));
        assert!(prompt.contains("ユーザーの妹として接する"));
        assert!(!prompt.contains("長期メモリ"));
        assert!(!prompt.contains("関係性メモリ"));
        assert!(prompt.ends_with(super::super::context::REFERENCE_RULES));
        assert!(super::compose_content(&config, "", "")
            .ends_with(super::super::context::REFERENCE_RULES));
        assert!(
            prompt.find("名前: むぎ").unwrap()
                < prompt.find(super::super::context::REFERENCE_RULES).unwrap()
        );
        assert_eq!(
            prompt
                .matches(super::super::context::REFERENCE_RULES)
                .count(),
            1
        );
        assert_eq!(
            super::compose_content(&config, "", "")
                .matches(super::super::context::REFERENCE_RULES)
                .count(),
            1
        );
        // 組み立てた人格がそのまま公開検査の照合元になる（登録漏れの経路を作らない）。
        assert!(super::super::guard_public_answer("名前: むぎ".to_string(), "").is_err());
        super::super::remember_private_context("");
    }

    #[test]
    fn 純粋な人格組立は空と非空と関係設定を同順序にし明示設定と公開検査を変更しない() {
        let _lock = crate::agent::PRIVATE_CONTEXT_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let config = Config::new();
        let custom = "明示した独自プロンプト\n末尾も保持。";
        config.set("PICOAGENT_SYSTEM_PROMPT", custom);
        config.set("PICOAGENT_RELATIONSHIP", "sister_younger");
        let remembered = super::super::remembered_private_context();
        let empty = super::compose_content(&config, "", "mugi");
        assert!(empty.starts_with(custom));
        assert!(!empty.contains("選択中キャラクターの人格設定:"));
        assert!(!empty.contains("ユーザーの妹として接する"));
        let full = super::compose_content(&config, "名前: むぎ", "mugi");
        assert!(full.starts_with(custom));
        assert!(full.contains("ユーザーの妹として接する"));
        assert!(full.ends_with(super::super::context::REFERENCE_RULES));
        assert_eq!(
            full.matches(super::super::context::REFERENCE_RULES).count(),
            1
        );
        assert_eq!(config.get("PICOAGENT_SYSTEM_PROMPT", ""), custom);
        assert_eq!(super::super::remembered_private_context(), remembered);
        assert_eq!(compose(&config, "名前: むぎ", "mugi"), full);
        super::super::remember_private_context("");
    }

    // 関数名を変えた複製も、人格見出し・プロフィール追記・関係suffixの組立責務で拾う。
    fn duplicates_persona_assembly(body: &str) -> bool {
        body.contains("選択中キャラクターの人格設定:")
            && body.contains("profile_suffix")
            && (body.contains("push_str") || body.contains("format!"))
    }

    #[test]
    fn 人格本文の再組立を正本以外へ複製しない() {
        let duplicates: Vec<_> = crate::guard_scan::all_sources()
            .into_iter()
            .filter(|source| !source.path.ends_with("agent/prompt.rs"))
            .filter(|source| duplicates_persona_assembly(source.body()))
            .map(|source| source.path.display().to_string())
            .collect();
        assert!(
            duplicates.is_empty(),
            "人格組立が複製されています: {duplicates:?}"
        );
        let old_commands = r#"
    let mut persona = agent::prompt::base(&state.config);
    if !character_profile.is_empty() {
        persona.push_str("\n\n選択中キャラクターの人格設定:\n");
        persona.push_str(&character_profile);
        persona.push_str(&crate::relationship_preset::profile_suffix(&state.config, &character_id));
    }
"#;
        assert!(duplicates_persona_assembly(old_commands));
        assert!(!duplicates_persona_assembly(
            "agent::prompt::compose_content(&config, &profile, &id)"
        ));
    }
}
