// tools/mod.rs - エージェント機能（スキル）コネクタのトレイトとレジストリ。
//
// 各スキルは Tool を実装し、inventory::submit! で自身を登録する。
// 連結ファイルはプロジェクト直下の skills/<スキル名>/mod.rs に置き、
// build.rs が走査して `pub mod` 宣言を自動生成するため、ディレクトリを足すだけでスキルを追加できる
// （このファイルの編集は不要）。トレイト・レジストリ本体だけがここに残る。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::Config;
use crate::message::Field;

/// 先回り実行したスキルの結果を、ユーザー発話へ差し込む文面へ変換する関数。
pub type IntentPromptRenderer = fn(&str, &str) -> String;

/// スキル側で判定した先回り実行の扱い。
#[derive(Clone, Copy)]
pub enum IntentHandling {
    /// 整形済みのスキル出力を、そのまま利用者へ返す。
    Direct,
    /// スキル出力またはローカル操作向けガイドを、モデル用の内部資料として差し込む。
    Material {
        renderer: IntentPromptRenderer,
        run_tool: bool,
        collect_sources: bool,
    },
}

/// 発話に一致したスキルが組み立てた引数と、実行後の扱い。
pub struct IntentInvocation {
    pub args: Value,
    pub handling: IntentHandling,
}

impl IntentInvocation {
    /// 発話の不正値を、外部通信や保存を始めず利用者へ返す。
    pub fn invalid(reason: String) -> Self {
        Self::direct(json!({"__picoagent_input_error": reason}))
    }
    pub fn direct(args: Value) -> Self {
        Self {
            args,
            handling: IntentHandling::Direct,
        }
    }

    pub fn material(args: Value, renderer: IntentPromptRenderer, collect_sources: bool) -> Self {
        Self {
            args,
            handling: IntentHandling::Material {
                renderer,
                run_tool: true,
                collect_sources,
            },
        }
    }

    pub fn guide(renderer: IntentPromptRenderer) -> Self {
        Self {
            args: Value::Null,
            handling: IntentHandling::Material {
                renderer,
                run_tool: false,
                collect_sources: false,
            },
        }
    }
}

/// 優先度順の走査で最初に一致した、登録済みスキルの先回り実行情報。
pub struct MatchedIntent {
    pub tool_name: &'static str,
    pub invocation: IntentInvocation,
}

/// スキル同梱のプロンプトをLFへ正規化し、テンプレートに元からある差し込み位置だけを置換する。
pub fn render_intent_prompt(template: &str, user_text: &str, tool_result: &str) -> String {
    const USER_TOKEN: &str = "{user_text}";
    const RESULT_TOKEN: &str = "{tool_result}";

    let normalized = template.replace("\r\n", "\n");
    let mut remaining = normalized.trim_end_matches('\n');
    let mut rendered = String::with_capacity(remaining.len() + user_text.len() + tool_result.len());
    loop {
        let user_at = remaining.find(USER_TOKEN);
        let result_at = remaining.find(RESULT_TOKEN);
        let (position, token, replacement) = match (user_at, result_at) {
            (Some(user), Some(result)) if user <= result => (user, USER_TOKEN, user_text),
            (Some(_), Some(result)) => (result, RESULT_TOKEN, tool_result),
            (Some(user), None) => (user, USER_TOKEN, user_text),
            (None, Some(result)) => (result, RESULT_TOKEN, tool_result),
            (None, None) => {
                rendered.push_str(remaining);
                break;
            }
        };
        rendered.push_str(&remaining[..position]);
        rendered.push_str(replacement);
        remaining = &remaining[position + token.len()..];
    }
    rendered
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn label(&self) -> &'static str {
        self.name()
    }
    fn description(&self) -> String;
    fn parameters(&self) -> Value;
    fn examples(&self) -> Vec<&'static str> {
        Vec::new()
    }
    /// ユーザー発話からこのスキルを候補へ出す、副作用のない意図判定語彙。
    fn intent_keywords(&self) -> &'static [&'static str] {
        &[]
    }
    /// 同じ発話へ複数スキルが一致したときの明示優先度。大きい方を先に提示する。
    fn intent_priority(&self) -> i32 {
        0
    }
    /// 発話から先回り実行の引数と扱いを組み立てる。通常のモデル呼び出しだけでよいスキルは `None` のままにする。
    fn intent_invocation(&self, _text: &str) -> Option<IntentInvocation> {
        None
    }
    /// 直前までの利用者発話（新しい順）も踏まえて先回り実行を組み立てる。
    /// 話題を含まない催促（「調べてくれないの？」）へ文脈を補うスキルだけが上書きする。
    fn intent_invocation_in_conversation(
        &self,
        text: &str,
        _earlier_user_texts: &[&str],
    ) -> Option<IntentInvocation> {
        self.intent_invocation(text)
    }
    /// 直前の回答も含めて先回り実行を組み立てる。
    /// 回答内の出典を「これ」「2番目の記事」のように参照するスキルだけが上書きする。
    fn intent_invocation_in_dialogue(
        &self,
        text: &str,
        earlier_user_texts: &[&str],
        _earlier_assistant_texts: &[&str],
    ) -> Option<IntentInvocation> {
        self.intent_invocation_in_conversation(text, earlier_user_texts)
    }
    /// このスキル自身が申告した語彙だけで発話との一致を判定する。
    fn matches_intent(&self, text: &str) -> bool {
        let normalized = text.to_lowercase();
        self.intent_keywords()
            .iter()
            .any(|keyword| normalized.contains(&keyword.to_lowercase()))
    }
    fn config_schema(&self) -> Vec<Field> {
        Vec::new()
    }
    /// エージェントループでモデルに function として提示するか。
    /// アプリ側が直接 `dispatch` するだけの内部スキル（例: 自発雑談）は false にして、
    /// モデルが会話中に誤って呼ばないようにする（`dispatch` 自体は false でも実行できる）。
    fn agent_callable(&self) -> bool {
        true
    }
    async fn run(&self, cfg: &Config, args: Value) -> String;
}

pub mod cli_detection;
pub mod datetime_words;
pub mod http;
pub mod process;
pub mod search_url;
pub mod social_post;
pub mod walk;
pub mod whisper;

pub struct ToolReg {
    pub make: fn() -> Box<dyn Tool>,
}
inventory::collect!(ToolReg);

// build.rs が生成した `pub mod time; pub mod read_file; ...` を取り込む。
include!(concat!(env!("OUT_DIR"), "/tool_mods.rs"));

/// 同梱 CLI ツール（`cargo xtask setup tools` → resources/bundled/tools/<ツール名>/）の探索先候補。
/// 追加導入した物、このビルドの同梱物の順。作業フォルダや実行ファイルの位置からは探さない。
fn bundled_tool_dirs(tool: &str) -> Vec<PathBuf> {
    crate::components::runtime_dirs(&format!("tools/{tool}"))
}

/// 明示設定か管理同梱物だけを使う。設定されたコマンド名だけはPATH検索を許可する。
pub fn resolve_cli_path(
    cfg: &Config,
    config_key: &str,
    tool_dir: &str,
    exe: &str,
) -> Result<PathBuf, String> {
    resolve_cli_in(
        &cfg.get(config_key, ""),
        &bundled_tool_dirs(tool_dir),
        config_key,
        exe,
    )
}

fn resolve_cli_in(
    configured: &str,
    dirs: &[PathBuf],
    key: &str,
    exe: &str,
) -> Result<PathBuf, String> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = Path::new(configured);
        if path.is_absolute() || path.components().count() > 1 {
            if !process::is_executable_file(path) {
                return Err(format!(
                    "{key} で指定した実行ファイルがありません: {}",
                    path.display()
                ));
            }
            return std::path::absolute(path)
                .map_err(|error| format!("実行パスを解決できません: {error}"));
        }
        // フォールバック許可: PR #472の利用者決定。明示したコマンド名だけPATHで解決する。（根拠: PR #472）
        return process::resolve_executable(configured)
            .ok_or_else(|| format!("{key} で指定したコマンドがPATHにありません"));
    }
    let exe_name = if cfg!(windows) {
        format!("{exe}.exe")
    } else {
        exe.to_string()
    };
    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        let found = find_bundled_exe(dir, &exe_name, 0)?;
        return found.ok_or_else(|| {
            format!("{exe} の同梱配置が不完全です。設定 → 追加機能・更新 から修復してください")
        });
    }
    Err(format!("{exe} が見つかりません。設定 → 追加機能・更新 から導入するか、{key} にパスを指定してください"))
}

fn find_bundled_exe(dir: &Path, exe_name: &str, depth: usize) -> Result<Option<PathBuf>, String> {
    // 実行ファイルの配置を深さ7まで検査する。対象外のFrameworks等は欠損候補にしない。
    if depth > 7 {
        return Ok(None);
    }
    let mut found = None;
    for entry in
        std::fs::read_dir(dir).map_err(|error| format!("同梱ツールを読めません: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("同梱ツールの項目を読めません: {error}"))?
            .path();
        let candidate =
            if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(exe_name) {
                if !process::is_executable_file(&path) {
                    return Err(format!(
                        "同梱実行ファイルに実行権限がありません: {}",
                        path.display()
                    ));
                }
                Some(path)
            } else if path.is_dir() {
                find_bundled_exe(&path, exe_name, depth + 1)?
            } else {
                None
            };
        if let Some(candidate) = candidate {
            if found.replace(candidate).is_some() {
                return Err("同梱の実行ファイルが重複しています。修復してください".into());
            }
        }
    }
    Ok(found)
}

pub fn config_guide(tool_label: &str, keys: &[&str], note: &str) -> String {
    let mut out = format!(
        "エラー: {tool_label} を使うには設定が必要です。\n設定画面（右クリック →「設定を開く…」→「スキル・連携」）で次の項目を入力して保存してください:"
    );
    for key in keys {
        out.push_str("\n- ");
        out.push_str(key);
    }
    if !note.trim().is_empty() {
        out.push_str("\n補足: ");
        out.push_str(note.trim());
    }
    out.push_str("\n保存後、もう一度同じ依頼をしてください。APIキーやトークンの値はチャット・ログ・ドキュメントへ貼らないでください。");
    out
}

#[cfg(test)]
mod cli_resolution_tests {
    #[test]
    fn 設定と同梱が欠損したら別候補へ進まない() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("installed");
        let packaged = dir.path().join("packaged");
        std::fs::create_dir(&packaged).unwrap();
        let exe = if cfg!(windows) { "tool.exe" } else { "tool" };
        let target = packaged.join(exe);
        std::fs::write(&target, []).unwrap();
        // 正しいCLIと併存する無関係な深い資産は、配置不完全にしない。
        std::fs::create_dir_all(packaged.join("a/b/c/d/e/f/g/h/i")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(
                super::resolve_cli_in(&target.to_string_lossy(), &[], "TEST_PATH", "tool").is_err()
            );
            assert!(super::resolve_cli_in("", &[packaged.clone()], "TEST_PATH", "tool").is_err());
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dirs = [installed.clone(), packaged];
        assert_eq!(
            super::resolve_cli_in("", &dirs, "TEST_PATH", "tool").unwrap(),
            target
        );
        let missing = dir.path().join("missing.exe");
        assert!(
            super::resolve_cli_in(&missing.to_string_lossy(), &dirs, "TEST_PATH", "tool").is_err()
        );
        std::fs::create_dir(&installed).unwrap();
        assert!(super::resolve_cli_in("", &dirs, "TEST_PATH", "tool")
            .unwrap_err()
            .contains("不完全"));
        assert!(super::resolve_cli_in("", &[], "TEST_PATH", "tool").is_err());
        let command = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(super::resolve_cli_in(command, &[], "TEST_PATH", "tool").is_ok());
    }
}

static TOOLS: OnceLock<Vec<Box<dyn Tool>>> = OnceLock::new();
static TOOL_INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
static TOOL_SCHEMAS: OnceLock<Vec<Value>> = OnceLock::new();
static TOOL_SCHEMA_INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
/// 定義1件ぶんの文字数。起動後は変わらないので1度だけ数える。
static TOOL_SCHEMA_CHARS: OnceLock<Vec<usize>> = OnceLock::new();
/// 各スキルの語彙（小文字化した形と、元の文字数）。`all()` と同じ並び。
static TOOL_KEYWORDS_LOWER: OnceLock<Vec<Vec<(String, usize)>>> = OnceLock::new();

// 利用者追加スキルは名前が任意で、発話だけから静的な語彙一致を作れない。
// `user_skill` の小さな管理schemaだけは常に提示し、毎ターン差し込む一覧から必要な
// SKILL.mdを読み出せるようにする。
const CORE_AGENT_TOOLS: [&str; 4] = ["get_time", "web_search", "web_fetch", "user_skill"];
const MAX_SELECTED_SCHEMAS: usize = 12;
const TOOL_ROUTES: &[(&str, &[&str])] = &[];

/// 登録済みスキルを優先度・一致語長・名前順で走査し、最初の先回り実行だけを返す（文脈なしの検査用）。
#[cfg(test)]
pub fn intent_for_text(text: &str) -> Option<MatchedIntent> {
    intent_for_conversation(text, &[])
}

/// 直前までの利用者発話（新しい順）も各スキルへ渡して、最初の先回り実行だけを返す。
#[cfg(test)]
pub fn intent_for_conversation(text: &str, earlier_user_texts: &[&str]) -> Option<MatchedIntent> {
    intent_for_dialogue(text, earlier_user_texts, &[])
}

/// 直前までの利用者発話と回答（いずれも新しい順）を各スキルへ渡す。
pub fn intent_for_dialogue(
    text: &str,
    earlier_user_texts: &[&str],
    earlier_assistant_texts: &[&str],
) -> Option<MatchedIntent> {
    debug_assert!(TOOL_ROUTES.is_empty());
    let normalized = text.to_lowercase();
    let mut matched: Vec<_> = all()
        .iter()
        .enumerate()
        .filter_map(|(index, tool)| {
            let invocation = tool.intent_invocation_in_dialogue(
                text,
                earlier_user_texts,
                earlier_assistant_texts,
            )?;
            let score = keyword_score(index, &normalized);
            Some((tool.intent_priority(), score, tool.name(), invocation))
        })
        .collect();
    matched.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.cmp(right.2))
    });
    matched
        .into_iter()
        .next()
        .map(|(_, _, tool_name, invocation)| MatchedIntent {
            tool_name,
            invocation,
        })
}
/// 登録済みスキルを返す。各スキルはプロセス内で1回だけ生成する。
pub fn all() -> &'static [Box<dyn Tool>] {
    TOOLS
        .get_or_init(|| {
            let mut tools: Vec<_> = inventory::iter::<ToolReg>
                .into_iter()
                .map(|r| (r.make)())
                .collect();
            tools.sort_unstable_by_key(|tool| tool.name());
            tools
        })
        .as_slice()
}

/// 1件のツール定義がコンテキストで占める文字数。
///
/// 定義は起動後に変わらないので1度だけ数えて使い回す。以前は候補を積むたびに
/// `serde_json::to_string` をやり直しており、1ターンで全件ぶんを直列化していた。
fn schema_chars_at(index: usize) -> usize {
    TOOL_SCHEMA_CHARS
        .get_or_init(|| {
            build_schemas()
                .iter()
                .map(|schema| {
                    serde_json::to_string(schema)
                        .map(|text| text.chars().count())
                        .unwrap_or(0)
                })
                .collect()
        })
        .get(index)
        .copied()
        .unwrap_or(0)
}

/// 各スキルの語彙を小文字化して覚える。発話との照合は毎ターン走るため、
/// そのたびに全スキルの全語彙を小文字化し直さない。
/// 文字数は元の語彙のまま数える（並び順の判定基準を変えないため）。
fn lowered_keywords() -> &'static [Vec<(String, usize)>] {
    TOOL_KEYWORDS_LOWER.get_or_init(|| {
        all()
            .iter()
            .map(|tool| {
                tool.intent_keywords()
                    .iter()
                    .map(|keyword| (keyword.to_lowercase(), keyword.chars().count()))
                    .collect()
            })
            .collect()
    })
}

/// 発話に当たった語彙のうち、いちばん長いものの文字数。0は不一致。
/// `index` は `all()` での位置。
fn keyword_score(index: usize, normalized: &str) -> usize {
    lowered_keywords()
        .get(index)
        .and_then(|keywords| {
            keywords
                .iter()
                .filter(|(keyword, _)| normalized.contains(keyword.as_str()))
                .map(|(_, chars)| *chars)
                .max()
        })
        .unwrap_or(0)
}

/// ツール定義へ配分する文字数。全72件で約48,000文字あり、人格・履歴・生成の枠を
/// 食い潰さないようコンテキスト長の35%までに収める。定義そのものは削らない
/// （説明や引数の説明を削ると、モデルが引数を組み立てられずスキルの精度が落ちる）。
/// クラウドは十分広いので全件入る値を返す。
fn schema_budget_chars(cfg: &Config) -> Result<usize, String> {
    if cfg.get("PICOAGENT_PROVIDER", "local") != "local" {
        return Ok(usize::MAX);
    }
    let ctx = crate::providers::local::context_length(cfg)?;
    // 1トークンあたり約3文字として見積もる（日本語混じりのJSON定義での実測値）。
    Ok((ctx as u128 * 35 / 100 * 3)
        .max(4_000)
        .min(usize::MAX as u128) as usize)
}

/// スキルが返した文字列が失敗を表すか。
/// スキルは `run()` の失敗を `Result` ではなく「エラー: 〜」の文字列で返す慣習
/// （AGENTS.md のコネクタ規則）なので、その接頭辞で判定する。
pub fn is_failure_text(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with("エラー:") || head.starts_with("エラー：")
}

/// 登録済みスキル（ツール）の名前一覧。
/// モデルが本文へ書いてしまったツール呼び出しを見分けるために使う。
pub fn names() -> Vec<&'static str> {
    all().iter().map(|tool| tool.name()).collect()
}

/// エージェントが会話中に使えるスキル（ツール）のラベル一覧を「、」区切りで返す。
/// 「何ができる？」への回答をモデルが具体的に答えられるよう、system プロンプトに添える用途。
pub fn capability_summary() -> String {
    let mut labels: Vec<&str> = all()
        .iter()
        .filter(|t| t.agent_callable())
        .map(|t| t.label())
        .collect();
    labels.sort_unstable();
    labels.dedup();
    labels.join("、")
}

/// ツール名から画面表示用のラベルを返す。未登録（外部MCPなど）は `mcp__<サーバ>__<ツール>` のツール部分、
/// それも無ければ名前をそのまま返す。
pub fn label_of(name: &str) -> String {
    index()
        .get(name)
        .map(|&idx| all()[idx].label().to_string())
        .unwrap_or_else(|| name.rsplit("__").next().unwrap_or(name).to_string())
}

fn index() -> &'static HashMap<&'static str, usize> {
    TOOL_INDEX.get_or_init(|| {
        all()
            .iter()
            .enumerate()
            .map(|(idx, tool)| (tool.name(), idx))
            .collect()
    })
}

/// プロバイダに渡す function スキーマ配列（OpenAI/Ollama 共通形式）を生成。
pub fn build_schemas() -> &'static [Value] {
    TOOL_SCHEMAS
        .get_or_init(|| {
            all()
                .iter()
                .filter(|t| t.agent_callable())
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name(),
                            "description": t.description(),
                            "parameters": t.parameters(),
                        }
                    })
                })
                .collect()
        })
        .as_slice()
}

fn schema_index() -> &'static HashMap<&'static str, usize> {
    TOOL_SCHEMA_INDEX.get_or_init(|| {
        let mut out = HashMap::new();
        for (idx, schema) in build_schemas().iter().enumerate() {
            if let Some(name) = schema
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            {
                out.insert(name, idx);
            }
        }
        out
    })
}

fn push_schema_by_name(out: &mut Vec<Value>, name: &str) {
    if out
        .iter()
        .any(|schema| schema["function"]["name"].as_str() == Some(name))
    {
        return;
    }
    if let Some(idx) = schema_index().get(name) {
        out.push(build_schemas()[*idx].clone());
    }
}

/// ユーザー発話に応じて、LLMへ渡す function schema を絞り込む。
/// 実行自体は dispatch で全スキルを呼べるため、これはモデルに見せる候補の圧縮だけを担う。
/// 直前までの利用者発話も見て、LLMへ渡す function schema を絞り込む。
///
/// 現在の発話だけで選ぶと、「今日の天気は？」→「神奈川県でよろー」のように地名だけを返す
/// 2ターン目で weather が候補から外れ、モデルは web_search しか呼べなくなる（実機で確認）。
/// 直前の発話にもキーワードを当てて、会話が続いている間は同じスキルを候補へ残す。
pub fn schemas_for_conversation(
    cfg: &Config,
    text: &str,
    earlier_user_texts: &[&str],
) -> Result<Vec<Value>, String> {
    // 見る過去発話の件数。0ですべての発話を見る。
    let turns = cfg.int(&crate::config::ints::TOOL_CONTEXT_TURNS)? as usize;
    let context = earlier_user_texts
        .iter()
        .take(if turns == 0 {
            earlier_user_texts.len()
        } else {
            turns
        })
        .map(|line| line.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = format!("{} {}", text.to_lowercase(), context);
    let mut selected = Vec::new();
    for name in CORE_AGENT_TOOLS {
        push_schema_by_name(&mut selected, name);
    }
    // どのスキルを使うかはモデルが決める。キーワードは「候補から外す」ためではなく
    // 「予算内で先に積む」順序付けだけに使う。以前はキーワードに一致しないスキルを
    // 候補から落としており、「今日の天気は？」→「神奈川県でよろー」の2ターン目で
    // weather が消えてモデルが web_search しか呼べなくなっていた。
    let mut ranked: Vec<_> = all()
        .iter()
        .enumerate()
        .filter(|(_, tool)| tool.agent_callable())
        .map(|(index, tool)| {
            let score = keyword_score(index, &normalized);
            (score > 0, tool.intent_priority(), score, tool.name())
        })
        .collect();
    // 一致あり → 申告した優先度 → 一致語の長さ → 名前、の順で安定させる。
    // 予算に収まる限り全件渡すので、この順序は「入り切らないとき何を先に積むか」を決める。
    ranked.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.3.cmp(right.3))
    });
    let budget = schema_budget_chars(cfg)?;
    let mut used: usize = CORE_AGENT_TOOLS
        .iter()
        .filter_map(|name| schema_index().get(name))
        .map(|index| schema_chars_at(*index))
        .sum();
    for (_, _, _, name) in ranked {
        let Some(index) = schema_index().get(name) else {
            continue;
        };
        let cost = schema_chars_at(*index);
        if used.saturating_add(cost) > budget {
            continue;
        }
        let before = selected.len();
        push_schema_by_name(&mut selected, name);
        if selected.len() > before {
            used = used.saturating_add(cost);
        }
    }
    // 外部 MCP サーバのツール（ユーザーが明示設定したもの）は常に候補へ加える。
    for schema in crate::mcp::client::schemas() {
        if selected.len() >= MAX_SELECTED_SCHEMAS + 8 {
            break;
        }
        selected.push(schema);
    }
    Ok(selected)
}

/// 自動計算時の下限。小さいコンテキストでも system・履歴・生成枠を残す。
const MIN_TOOL_OUTPUT_CHARS: usize = crate::config::ints::TOOL_OUTPUT_MAX_CHARS.min as usize;
/// ツール出力の上限（設定でこれ以上は広げない。1回の出力で文脈を食い潰さないための天井）。
const MAX_TOOL_OUTPUT_CHARS: usize = crate::config::ints::TOOL_OUTPUT_MAX_CHARS.max as usize;
/// クラウドプロバイダ利用時の既定（コンテキストが十分広いので固定値でよい）。
const CLOUD_TOOL_OUTPUT_CHARS: usize = 24_000;

/// ツール出力の最大文字数（コンテキストウィンドウ超過防止）。
///
/// 既定（`auto`）は使用中のコンテキスト長の45%から算出する。
/// ローカルの未設定時は32,768なので14,745文字となり、人格・履歴・回答生成の枠を残す。
/// auto計算を下限1,000文字・上限60,000文字に収め、明示値は共通設定宣言で検証する。
pub fn tool_output_limit(cfg: &Config) -> Result<usize, String> {
    if let Some(value) = cfg.int_or_auto(&crate::config::ints::TOOL_OUTPUT_MAX_CHARS)? {
        return Ok(value as usize);
    }
    if cfg.get("PICOAGENT_PROVIDER", "local") != "local" {
        return Ok(CLOUD_TOOL_OUTPUT_CHARS);
    }
    let ctx = crate::providers::local::context_length(cfg)?;
    // 明示整数を補正せず、autoの予算だけモデルの文脈長から計算する。
    Ok(
        (ctx as u128 * 45 / 100).clamp(MIN_TOOL_OUTPUT_CHARS as u128, MAX_TOOL_OUTPUT_CHARS as u128)
            as usize,
    )
}

/// ツール出力にプロンプトインジェクション試行パターンが含まれるか検査する。
pub fn check_injection(content: &str) -> bool {
    let lower = content.to_lowercase();
    [
        "ignore previous instructions",
        "ignore all previous",
        "disregard previous",
        "new instructions:",
        "forget your instructions",
        "you are now",
        "前の指示を無視",
        "指示を無視して",
        "システムプロンプトを",
        "あなたは今から",
        "<system>",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

/// モデルが要求したツール呼び出しを実行する。失敗は例外にせず文字列で返す。
/// 出力が長すぎる場合は MAX_TOOL_OUTPUT_CHARS で打ち切ってコンテキスト超過を防ぐ。
/// ツールを実行する（**既定の安全な入口**）。
///
/// `run_command` が未許可コマンドに対して返す承認マーカーは、対話ループだけが解釈できる
/// 内部プロトコル。ここでは人が読める案内文へ変換して返すため、workflow / subagent /
/// スケジューラ / 接続テストなど対話できない経路から呼んでも内部トークンが露出しない。
/// 承認フローを自分で扱えるのは `agent.rs` だけで、そこは `dispatch_interactive` を使う。
pub async fn dispatch(cfg: &Config, name: &str, args: Value) -> String {
    readable_approval_notice(dispatch_interactive(cfg, name, args).await)
}

/// 承認マーカーは内部プロトコル。承認を受け付けられない経路では人が読める案内文へ変換する。
fn readable_approval_notice(result: String) -> String {
    match run_command::parse_approval_marker(&result) {
        Some(pending) => format!(
            "エラー: この操作には実行許可が必要です（この経路では承認を受け付けられません）。\n{}\n\nチャットで同じコマンドを直接依頼すると、その場で許可・永続許可・キャンセルを選べます。",
            run_command::approval_prompt(&pending)
        ),
        None => result,
    }
}

/// ライセンス判定とツール本体の実行だけを行う共通部分。打ち切りは呼び出し側の判断。
async fn run_tool(cfg: &Config, name: &str, args: Value) -> String {
    if let Some(reason) = args.get("__picoagent_input_error") {
        return format!(
            "エラー: {}",
            reason.as_str().unwrap_or("入力の検査結果が不正です。")
        );
    }
    if crate::license::tool_requires_pro(name) && !crate::license::is_pro(cfg) {
        return "エラー: このスキルは PicoAgent Pro のライセンスが必要です。設定の「製品情報・ライセンス」でライセンスキーを登録してください。".to_string();
    }
    if let Some(idx) = index().get(name) {
        all()[*idx].run(cfg, args).await
    } else if crate::mcp::client::is_mcp_tool(name) {
        // 外部 MCP サーバのツール（mcp__<サーバ>__<ツール>）へフォールバックする。
        crate::mcp::client::dispatch(name, args).await
    } else {
        format!("エラー: 未登録のツール \"{}\" が呼び出されました。", name)
    }
}

/// 戻り値をモデルへ渡すツール出力ではなく、**利用者への最終返信としてそのまま使う**呼び出し。
/// マルチAIの統合結果のように、途中で打ち切ると回答自体が欠けるものへ使う。
/// ツール出力上限を当てない代わりに、呼び出し側が長さと表示先に責任を持つ。
pub async fn dispatch_final(cfg: &Config, name: &str, args: Value) -> String {
    readable_approval_notice(run_tool(cfg, name, args).await)
}

/// モデルが要求したツール呼び出しを実行する（対話ループ専用の生の入口）。
/// 承認マーカーをそのまま返すため、**呼び出し側が承認フローを実装している場合のみ**使う。
/// 失敗は例外にせず文字列で返す。出力が長すぎる場合は MAX_TOOL_OUTPUT_CHARS で打ち切る。
pub async fn dispatch_interactive(cfg: &Config, name: &str, args: Value) -> String {
    let limit = match tool_output_limit(cfg) {
        Ok(limit) => limit,
        Err(error) => return format!("エラー: {error}"),
    };
    let result = run_tool(cfg, name, args).await;
    if result.chars().count() > limit {
        let truncated: String = result.chars().take(limit).collect();
        format!(
            "{}\n\n[...出力が長いため{}文字で打ち切りました]",
            truncated, limit
        )
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::is_failure_text;

    #[tokio::test]
    async fn 発話の不正値はスキル実行前に理由を返す() {
        let cfg = crate::config::Config::new();
        let before = cfg.get_all();
        for name in [
            "focus_timer",
            "reminder",
            "currency_rates",
            "market_indices",
        ] {
            let invocation = super::IntentInvocation::invalid("数値が大きすぎます".into());
            assert_eq!(
                super::dispatch(&cfg, name, invocation.args).await,
                "エラー: 数値が大きすぎます"
            );
            assert_eq!(cfg.get_all(), before);
        }
    }

    #[test]
    fn スキルの失敗文を見分ける() {
        // スキルは run() の失敗を「エラー: 〜」の文字列で返す（AGENTS.md のコネクタ規則）。
        assert!(is_failure_text(
            "エラー: 天気の地点が未設定です。設定で市区町村または都道府県を入力してください。"
        ));
        assert!(is_failure_text("  エラー：全角コロンの失敗文"));
        assert!(!is_failure_text("今日の東京は晴れ、最高気温は28度だよ。"));
        assert!(!is_failure_text("エラーの直し方を説明するね"));
    }

    fn validate_schema(value: &serde_json::Value, location: &str) {
        let object = value
            .as_object()
            .unwrap_or_else(|| panic!("{location} はJSON objectである必要があります"));
        if let Some(kind) = object.get("type") {
            assert!(
                kind.is_string() || kind.is_array(),
                "{location}.type は文字列または配列にしてください"
            );
        }
        if let Some(properties) = object.get("properties") {
            for (name, property) in properties
                .as_object()
                .unwrap_or_else(|| panic!("{location}.properties はobjectにしてください"))
            {
                validate_schema(property, &format!("{location}.properties.{name}"));
            }
        }
        if let Some(required) = object.get("required") {
            let properties = object
                .get("properties")
                .and_then(serde_json::Value::as_object);
            for name in required
                .as_array()
                .unwrap_or_else(|| panic!("{location}.required は配列にしてください"))
            {
                let name = name
                    .as_str()
                    .unwrap_or_else(|| panic!("{location}.required の要素は文字列にしてください"));
                assert!(
                    properties.is_some_and(|values| values.contains_key(name)),
                    "{location}.required の {name} がpropertiesにありません"
                );
            }
        }
        if let Some(items) = object.get("items") {
            validate_schema(items, &format!("{location}.items"));
        }
    }

    #[test]
    fn 全スキルのメタデータとschemaは一意で妥当() {
        let mut names = std::collections::HashSet::new();
        let mut labels = std::collections::HashSet::new();
        let allowed_field_types = [
            "text", "password", "textarea", "checkbox", "number", "select",
        ];
        for tool in super::all() {
            assert!(!tool.name().trim().is_empty(), "スキル名が空です");
            assert!(
                names.insert(tool.name()),
                "スキル名が重複しています: {}",
                tool.name()
            );
            assert!(
                !tool.label().trim().is_empty(),
                "{} の表示名が空です",
                tool.name()
            );
            assert!(
                labels.insert(tool.label()),
                "スキル表示名が重複しています: {}",
                tool.label()
            );
            assert!(
                !tool.description().trim().is_empty(),
                "{} の説明が空です",
                tool.name()
            );
            validate_schema(&tool.parameters(), tool.name());
            let mut config_keys = std::collections::HashSet::new();
            for field in tool.config_schema() {
                assert!(
                    !field.key.trim().is_empty(),
                    "{} の設定キーが空です",
                    tool.name()
                );
                assert!(
                    config_keys.insert(field.key.clone()),
                    "{} の設定キーが重複しています: {}",
                    tool.name(),
                    field.key
                );
                assert!(
                    allowed_field_types.contains(&field.r#type.as_str()),
                    "{} の設定型が不明です: {}",
                    tool.name(),
                    field.r#type
                );
                assert!(
                    !field.label.trim().is_empty(),
                    "{} / {} のラベルが空です",
                    tool.name(),
                    field.key
                );
            }
        }
    }

    #[test]
    fn 意図ルートは各スキルの登録情報だけを参照する() {
        assert!(
            super::TOOL_ROUTES.is_empty(),
            "中央の意図ルート表へスキルを戻さないでください"
        );
        for (text, name) in [
            ("今日の気象を教えて", "weather"),
            ("15分後にリマインダーして", "reminder"),
            ("Whisperで文字起こし", "whisper_transcribe"),
            ("競合比較を調査", "research_report"),
        ] {
            let matched = super::intent_for_text(text)
                .unwrap_or_else(|| panic!("{text} の先回り実行が見つかりません"));
            assert_eq!(matched.tool_name, name, "{text} の割当先が違います");
            assert!(matches!(
                matched.invocation.handling,
                super::IntentHandling::Direct
            ));
            assert!(
                schema_names(text).contains(&name.to_string()),
                "{text} のschema候補に{name}がありません"
            );
        }

        let weather = super::intent_for_text("大阪の気温").unwrap();
        assert_eq!(weather.tool_name, "weather");
        assert_eq!(weather.invocation.args["location"], "大阪");

        let reminder = super::intent_for_text("15分後に洗濯物をリマインダーして").unwrap();
        assert_eq!(reminder.tool_name, "reminder");
        assert_eq!(reminder.invocation.args["minutes_from_now"], 15);

        let transcription = super::intent_for_text("話者分類つきでWhisper文字起こしして").unwrap();
        assert_eq!(transcription.tool_name, "whisper_transcribe");
        assert_eq!(transcription.invocation.args["diarize"], true);

        let research =
            super::intent_for_text("AIエージェント領域を出典付きで調査してレポートにして").unwrap();
        assert_eq!(research.tool_name, "research_report");
        assert_eq!(research.invocation.args["save"], true);
        assert_eq!(research.invocation.args["artifact_bundle"], true);
    }

    #[test]
    fn 先回り実行は旧分岐と同じ引数と扱いを返す() {
        let currency = super::intent_for_text("ドル円のチャート見せて").unwrap();
        assert_eq!(currency.tool_name, "currency_rates");
        assert_eq!(currency.invocation.args["base"], "USD");
        assert_eq!(currency.invocation.args["symbols"], "JPY");
        assert_eq!(currency.invocation.args["chart"], true);

        let market = super::intent_for_text("日系平均のチャートを見せて").unwrap();
        assert_eq!(market.tool_name, "market_indices");
        assert_eq!(market.invocation.args["symbol"], "nikkei225");
        assert_eq!(market.invocation.args["chart"], true);

        let focus = super::intent_for_text("25分集中タイマー").unwrap();
        assert_eq!(focus.tool_name, "focus_timer");
        assert_eq!(focus.invocation.args["action"], "start");
        assert_eq!(focus.invocation.args["minutes"], 25);

        let praise = super::intent_for_text("企画書を書けた").unwrap();
        assert_eq!(praise.tool_name, "praise_log");
        assert_eq!(praise.invocation.args["action"], "add");

        let meeting = super::intent_for_text("会議の議事録を作って").unwrap();
        assert_eq!(meeting.tool_name, "meeting_recorder");
        assert_eq!(meeting.invocation.args["action"], "minutes");

        let suite = super::intent_for_text("Genspark風の機能を全部やって").unwrap();
        assert_eq!(suite.tool_name, "genspark_suite");
        assert_eq!(suite.invocation.args["audience"], "一般読者");

        let workspace = super::intent_for_text("Gensparkみたいに資料一式を作って").unwrap();
        assert_eq!(workspace.tool_name, "genspark_suite");
        assert_eq!(workspace.invocation.args["mode"], "light");
        assert_eq!(
            workspace.invocation.args["topic"],
            "Gensparkみたいに資料一式を作って"
        );
    }

    #[test]
    fn 資料差し込みと直接返却の区別を維持する() {
        let news = super::intent_for_text("今日のニュースをまとめて").unwrap();
        assert_eq!(news.tool_name, "news_scrape");
        assert!(matches!(
            news.invocation.handling,
            super::IntentHandling::Material {
                run_tool: true,
                collect_sources: true,
                ..
            }
        ));

        let clipboard = super::intent_for_text("クリップボードを要約して").unwrap();
        assert_eq!(clipboard.tool_name, "clipboard_text");
        assert!(matches!(
            clipboard.invocation.handling,
            super::IntentHandling::Material {
                run_tool: true,
                collect_sources: false,
                ..
            }
        ));

        let reminder = super::intent_for_text("リマインダー一覧を見せて").unwrap();
        assert_eq!(reminder.tool_name, "reminder");
        assert!(matches!(
            reminder.invocation.handling,
            super::IntentHandling::Material {
                run_tool: false,
                collect_sources: false,
                ..
            }
        ));

        let search = super::intent_for_text("Rustとは").unwrap();
        assert_eq!(search.tool_name, "web_search");
        assert!(matches!(
            search.invocation.handling,
            super::IntentHandling::Material {
                run_tool: true,
                collect_sources: true,
                ..
            }
        ));
    }

    #[test]
    fn 直前回答の見出しを指定した深掘りは記事本文を取得する() {
        let previous = "主なニュースです。\n- [宇宙開発の新しい発表](https://example.com/space)";
        let matched =
            super::intent_for_dialogue("宇宙開発のニュースを深掘りして", &[], &[previous]).unwrap();
        assert_eq!(matched.tool_name, "web_fetch");
        assert_eq!(matched.invocation.args["url"], "https://example.com/space");
        assert_eq!(
            matched.invocation.args["source_title"],
            "宇宙開発の新しい発表"
        );
    }

    #[test]
    fn 実行中タスクには画面用のラベルを渡す() {
        assert_eq!(super::label_of("web_search"), "Web検索");
        assert_eq!(
            super::label_of("comfyui_generate"),
            "ローカル画像・動画生成"
        );
        assert_eq!(super::label_of("mcp__files__read_text"), "read_text");
        assert_eq!(super::label_of("unknown_tool"), "unknown_tool");
    }

    #[test]
    fn プロンプト差し込みは入力内のプレースホルダーを再置換しない() {
        let rendered = super::render_intent_prompt(
            "{user_text}\n---\n{tool_result}",
            "入力{tool_result}",
            "結果{user_text}",
        );
        assert_eq!(rendered, "入力{tool_result}\n---\n結果{user_text}");
    }

    #[test]
    fn エージェントループへスキル名と長文資料プロンプトを戻さない() {
        let agent = crate::guard_scan::all_sources()
            .into_iter()
            .find(|source| source.path.ends_with("src-tauri/src/agent.rs"))
            .expect("agent.rs");
        let allowed = ["run_command", "ask_user", "view_image"];
        let leaked_names: Vec<_> = super::all()
            .iter()
            .map(|tool| tool.name())
            .filter(|name| !allowed.contains(name))
            .filter(|name| agent.body().contains(&format!("\"{name}\"")))
            .collect();
        assert!(
            leaked_names.is_empty(),
            "agent.rsへスキル名リテラルを戻さないでください: {leaked_names:?}"
        );
        for marker in [
            "以下はGoogle News / Yahoo!ニュースから取得した内部資料です",
            "以下はローカルのクリップボード取得結果です",
            "以下は回答前に必ず実行したWeb検索の結果です",
        ] {
            assert!(
                !agent.body().contains(marker),
                "長文資料プロンプトは対応するskills/<名前>/promptsへ置いてください: {marker}"
            );
        }
    }

    #[test]
    fn スキル候補と能力一覧は登録順に依存しない() {
        let registered: Vec<_> = super::all().iter().map(|tool| tool.name()).collect();
        let mut sorted_registered = registered.clone();
        sorted_registered.sort_unstable();
        assert_eq!(registered, sorted_registered);

        // 同じ優先度どうしは、長い一致語のスキルを先にする。
        let first = schema_names("PDFを画像変換して");
        assert_eq!(first, schema_names("PDFを画像変換して"));
        let image_tools = first.iter().position(|name| name == "image_tools").unwrap();
        let pdf_tools = first.iter().position(|name| name == "pdf_tools").unwrap();
        assert!(
            image_tools < pdf_tools,
            "長い一致語を先にしてください: {first:?}"
        );

        // 明示した優先度は一致語の長さより優先する（画像生成110は画像変換0より先）。
        let prioritized = schema_names("画像変換して画像を作って");
        let image_tools = prioritized
            .iter()
            .position(|name| name == "image_tools")
            .unwrap();
        let generator = prioritized
            .iter()
            .position(|name| name == "comfyui_generate")
            .unwrap();
        assert!(
            generator < image_tools,
            "明示した優先度を先にしてください: {prioritized:?}"
        );

        let summary = super::capability_summary();
        let labels: Vec<_> = summary.split('、').collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(labels, sorted);
    }

    /// 承認マーカーは対話ループ専用の内部プロトコル。安全側の `dispatch` からは
    /// 人が読める案内文へ変換され、内部トークン（token / cwd 等）が露出しないこと。
    #[test]
    fn 対話できない経路には承認マーカーを漏らさない() {
        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_COMMAND_ENABLED", "true");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        // 既定 allowlist に無いコマンド＝承認が必要な経路。
        let args = serde_json::json!({ "program": "picoagent-unlisted", "args": ["x"] });

        // 対話ループ専用の入口は承認フローのためマーカーをそのまま返す（前提の確認）。
        let raw = runtime.block_on(super::dispatch_interactive(
            &cfg,
            "run_command",
            args.clone(),
        ));
        assert!(
            raw.contains("__PICOAGENT_COMMAND_APPROVAL__"),
            "承認マーカーの前提が崩れています: {raw}"
        );

        // 安全側は内部トークンを出さず、人が読める案内文にする。
        let safe = runtime.block_on(super::dispatch(&cfg, "run_command", args));
        assert!(
            !safe.contains("__PICOAGENT_COMMAND_APPROVAL__"),
            "内部マーカーが漏れています: {safe}"
        );
        assert!(
            !safe.contains("\"token\""),
            "内部トークンが漏れています: {safe}"
        );
        assert!(
            safe.contains("実行許可が必要"),
            "案内文になっていません: {safe}"
        );
    }

    fn schema_names(text: &str) -> Vec<String> {
        super::schemas_for_conversation(&crate::config::Config::new(), text, &[])
            .unwrap()
            .iter()
            .filter_map(|schema| {
                schema
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(|name| name.as_str())
                    .map(ToString::to_string)
            })
            // 外部MCPは別テストが接続中でも変化するため、組み込み候補の検査から分離する。
            .filter(|name| !name.starts_with("mcp__"))
            .collect()
    }

    /// 設計ガード: 対話ループ専用の生入口を、承認フローを持たない場所から使わないこと。
    /// `dispatch_interactive` は承認マーカーをそのまま返すため、agent.rs 以外が使うと
    /// 内部トークンがユーザーへ露出する。
    #[test]
    fn 生の実行入口はエージェントループだけが使う() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri の親");
        let mut violations = Vec::new();
        for dir in [root.join("skills"), root.join("src-tauri/src")] {
            let mut stack = vec![dir];
            while let Some(current) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&current) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                        continue;
                    }
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    // 定義元（tools/mod.rs）と承認フローの実装（agent.rs）だけが例外。
                    if name == "agent.rs" || path.ends_with("tools/mod.rs") {
                        continue;
                    }
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    if text.contains("dispatch_interactive") {
                        violations.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
        assert!(
            violations.is_empty(),
            "承認フローを持たない場所が `dispatch_interactive` を使っています。`tools::dispatch` を使ってください:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn 発話に関係なくコンテキストに収まるスキルはすべて渡す() {
        // どれを使うかはモデルが決める。キーワードに一致しないという理由で候補から
        // 外すと、言い回しが変わっただけで機能が届かなくなる。
        let names = schema_names("こんにちは");
        assert!(names.contains(&"get_time".to_string()));
        assert!(names.contains(&"web_search".to_string()));
        // キーワードに一致しないスキルも、コンテキストに余裕がある限り候補へ残す。
        // 入り切らない分は優先度順で落ちるが、それは「一致しないから外す」のとは別の理由。
        assert!(
            names.len() >= 30,
            "既定のコンテキストでは多くのスキルが入る: {}",
            names.len()
        );
        assert!(
            names.len() > super::CORE_AGENT_TOOLS.len() + 5,
            "基本ツールだけに絞り込まない: {names:?}"
        );
    }

    #[test]
    fn 発話に合うスキルを追加する() {
        let names = schema_names("Slack にこの内容を送って");
        assert!(names.contains(&"slack_send_message".to_string()));
    }

    #[test]
    fn ブラウザ操作スキルを候補に追加する() {
        let names = schema_names("このページをブラウザで開いて読んで");
        assert!(names.contains(&"browser_control".to_string()));
    }

    #[tokio::test]
    async fn 不正な出力予算ではツールを実行しない() {
        let cfg = crate::config::Config::new();
        for value in ["999", "60001", "bad", "1.5", "-1"] {
            cfg.set("PICOAGENT_TOOL_OUTPUT_MAX_CHARS", value);
            // 未登録の呼出も設定エラーより先へ進めず、実行経路を共通で止める。
            let reply =
                super::dispatch_interactive(&cfg, "未登録テスト", serde_json::json!({})).await;
            assert!(reply.contains("PICOAGENT_TOOL_OUTPUT_MAX_CHARS"));
            assert!(!reply.contains("未登録のツール"));
        }
        cfg.set("PICOAGENT_TOOL_CONTEXT_TURNS", "-1");
        assert!(super::schemas_for_conversation(&cfg, "天気", &[])
            .unwrap_err()
            .contains("PICOAGENT_TOOL_CONTEXT_TURNS"));
    }

    #[test]
    fn ツール出力の上限はコンテキスト長に追随する() {
        use crate::config::Config;
        let narrow = Config::new();
        narrow.set("PICOAGENT_LOCAL_CTX", "32768");
        assert_eq!(super::tool_output_limit(&narrow).unwrap(), 14_745);
        let invalid = Config::new();
        invalid.set("PICOAGENT_LOCAL_CTX", "8192");
        assert!(super::tool_output_limit(&invalid)
            .unwrap_err()
            .contains("PICOAGENT_LOCAL_CTX"));

        let wide = Config::new();
        wide.set("PICOAGENT_LOCAL_CTX", "65536");
        assert!(
            super::tool_output_limit(&wide).unwrap() > super::tool_output_limit(&narrow).unwrap(),
            "コンテキストを広げた分だけ多く読める"
        );

        let huge = Config::new();
        huge.set("PICOAGENT_LOCAL_CTX", "1000000");
        assert_eq!(
            super::tool_output_limit(&huge).unwrap(),
            super::MAX_TOOL_OUTPUT_CHARS,
            "天井は超えない"
        );

        huge.set("PICOAGENT_LOCAL_CTX", &i64::MAX.to_string());
        assert_eq!(
            super::tool_output_limit(&huge).unwrap(),
            super::MAX_TOOL_OUTPUT_CHARS
        );
        assert!(super::schema_budget_chars(&huge).unwrap() > 0);

        let explicit = Config::new();
        explicit.set("PICOAGENT_TOOL_OUTPUT_MAX_CHARS", "12000");
        assert_eq!(super::tool_output_limit(&explicit).unwrap(), 12_000);
    }

    #[test]
    fn 日常補助スキルを候補に追加する() {
        let reminder = schema_names("15分後にリマインダーして");
        assert!(reminder.contains(&"reminder".to_string()));
        let focus = schema_names("25分集中タイマーを始めて");
        assert!(focus.contains(&"focus_timer".to_string()));
        let clipboard = schema_names("クリップボードを要約して");
        assert!(clipboard.contains(&"clipboard_text".to_string()));
    }

    #[test]
    fn 実用ツール集を候補に追加する() {
        let json = schema_names("このJSONを整形して");
        assert!(json.contains(&"productivity_tools".to_string()));
        let unit = schema_names("10kmをmileに単位換算して");
        assert!(unit.contains(&"productivity_tools".to_string()));
    }

    #[test]
    fn 資料ワークスペースを候補に追加する() {
        let names = schema_names("Gensparkみたいに資料一式を作って");
        assert!(names.contains(&"genspark_suite".to_string()));
        assert!(!super::all()
            .iter()
            .any(|tool| tool.name() == "spark_workspace"));
    }

    #[test]
    fn 画像変換スキルを候補に追加する() {
        let names = schema_names("このPSDをPNGに変換して");
        assert!(names.contains(&"image_tools".to_string()));
        let heic = schema_names("HEICの写真をJPGにして");
        assert!(heic.contains(&"image_tools".to_string()));
    }

    #[test]
    fn 会議録音スキルを候補に追加する() {
        let names = schema_names("Whisperで会議を文字起こしして議事録にして");
        assert!(names.contains(&"meeting_recorder".to_string()));
        assert!(names.contains(&"whisper_transcribe".to_string()));
    }
}

/// ToolのString戻り値へ、実行前の失敗を伝える。
#[macro_export]
macro_rules! tool_try {
    ($value:expr) => {
        match $value {
            Ok(value) => value,
            Err(error) => return format!("エラー: {error}"),
        }
    };
}
