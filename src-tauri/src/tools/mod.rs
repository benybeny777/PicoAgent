// tools/mod.rs - エージェント機能（スキル）コネクタのトレイトとレジストリ。
//
// 各スキルは Tool を実装し、inventory::submit! で自身を登録する。
// 連結ファイルはプロジェクト直下の skills/<スキル名>/mod.rs に置き、
// build.rs が走査して `pub mod` 宣言を自動生成するため、ディレクトリを足すだけでスキルを追加できる
// （このファイルの編集は不要）。トレイト・レジストリ本体だけがここに残る。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::config::Config;
use crate::message::Field;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn label(&self) -> &'static str {
        self.name()
    }
    fn description(&self) -> String;
    fn parameters(&self) -> Value;
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

pub struct ToolReg {
    pub make: fn() -> Box<dyn Tool>,
}
inventory::collect!(ToolReg);

// build.rs が生成した `pub mod time; pub mod read_file; ...` を取り込む。
include!(concat!(env!("OUT_DIR"), "/tool_mods.rs"));

pub fn config_guide(tool_label: &str, keys: &[&str], note: &str) -> String {
    let mut out = format!(
        "エラー: {tool_label} を使うには設定が必要です。\n設定画面（右クリック →「設定…」→「スキル」）で次の項目を入力して保存してください:"
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

static TOOLS: OnceLock<Vec<Box<dyn Tool>>> = OnceLock::new();
static TOOL_INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
static TOOL_SCHEMAS: OnceLock<Vec<Value>> = OnceLock::new();
static TOOL_SCHEMA_INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();

const CORE_AGENT_TOOLS: [&str; 3] = ["get_time", "web_search", "web_fetch"];
const MAX_SELECTED_SCHEMAS: usize = 12;
const TOOL_ROUTES: &[(&str, &[&str])] = &[
    (
        "weather",
        &["天気", "気温", "雨", "降水", "予報", "weather"],
    ),
    (
        "currency_rates",
        &["為替", "換算", "円", "ドル", "ユーロ", "usd", "jpy", "eur"],
    ),
    (
        "rss_feed",
        &["rss", "atom", "フィード", "ニュース", "ブログ", "更新"],
    ),
    (
        "alarm",
        &[
            "アラーム",
            "目覚まし",
            "めざまし",
            "起こして",
            "起こし",
            "alarm",
            "wake",
            "何時に",
        ],
    ),
    ("read_file", &["読んで", "読み取", "ファイル", "read_file"]),
    (
        "workspace_files",
        &[
            "ファイル",
            "書いて",
            "保存",
            "追記",
            "フォルダ",
            "workspace",
        ],
    ),
    (
        "run_command",
        &[
            "コマンド",
            "実行",
            "テスト",
            "ビルド",
            "cargo",
            "npm",
            "git",
        ],
    ),
    ("slack_send_message", &["slack", "スラック"]),
    ("discord_send_message", &["discord", "ディスコード"]),
    ("gmail_message", &["gmail", "メール", "下書き", "送信"]),
    (
        "google_calendar",
        &["calendar", "カレンダー", "予定", "スケジュール"],
    ),
    (
        "google_docs",
        &["google docs", "docs", "ドキュメント", "文書"],
    ),
    ("google_drive", &["drive", "ドライブ"]),
    (
        "google_sheets",
        &["sheets", "スプレッドシート", "表計算", "セル"],
    ),
    ("google_tasks", &["tasks", "タスク", "todo", "やること"]),
    ("github_issue", &["github", "issue", "イシュー"]),
    ("notion", &["notion"]),
    (
        "microsoft_graph",
        &["outlook", "teams", "microsoft", "office365"],
    ),
    ("jira", &["jira"]),
    ("linear", &["linear"]),
    ("trello", &["trello"]),
    ("todoist", &["todoist"]),
    ("http_api", &["api", "webhook", "http"]),
    (
        "ms_office",
        &["word", "excel", "powerpoint", "office", "オフィス"],
    ),
    (
        "document_convert",
        &["変換", "pdf化", "libreoffice", "convert"],
    ),
    ("pdf_tools", &["pdf", "結合", "抽出", "回転"]),
    ("x_twitter", &["twitter", "xに", "ポスト", "ツイート"]),
    ("mastodon", &["mastodon", "マストドン"]),
    ("bluesky", &["bluesky", "bsky", "ブルースカイ"]),
    ("reddit", &["reddit"]),
    ("linkedin", &["linkedin"]),
    ("misskey", &["misskey", "ミスキー", "ノート"]),
];

/// 登録済みスキルを返す。各スキルはプロセス内で1回だけ生成する。
pub fn all() -> &'static [Box<dyn Tool>] {
    TOOLS
        .get_or_init(|| {
            inventory::iter::<ToolReg>
                .into_iter()
                .map(|r| (r.make)())
                .collect()
        })
        .as_slice()
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
pub fn schemas_for_text(text: &str) -> Vec<Value> {
    let normalized = text.to_lowercase();
    let mut selected = Vec::new();
    for name in CORE_AGENT_TOOLS {
        push_schema_by_name(&mut selected, name);
    }
    for (name, keywords) in TOOL_ROUTES {
        if selected.len() >= MAX_SELECTED_SCHEMAS {
            break;
        }
        if keywords
            .iter()
            .any(|keyword| normalized.contains(&keyword.to_lowercase()))
        {
            push_schema_by_name(&mut selected, name);
        }
    }
    selected
}

/// モデルが要求したツール呼び出しを実行する。失敗は例外にせず文字列で返す。
pub async fn dispatch(cfg: &Config, name: &str, args: Value) -> String {
    if let Some(idx) = index().get(name) {
        return all()[*idx].run(cfg, args).await;
    }
    format!("エラー: 未登録のツール \"{}\" が呼び出されました。", name)
}

#[cfg(test)]
mod tests {
    use super::schemas_for_text;

    fn schema_names(text: &str) -> Vec<String> {
        schemas_for_text(text)
            .iter()
            .filter_map(|schema| {
                schema
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(|name| name.as_str())
                    .map(ToString::to_string)
            })
            .collect()
    }

    #[test]
    fn 通常発話では基本ツールだけを中心に返す() {
        let names = schema_names("こんにちは");
        assert!(names.contains(&"get_time".to_string()));
        assert!(names.contains(&"web_search".to_string()));
        assert!(!names.contains(&"slack_send_message".to_string()));
    }

    #[test]
    fn 発話に合うスキルを追加する() {
        let names = schema_names("Slack にこの内容を送って");
        assert!(names.contains(&"slack_send_message".to_string()));
    }
}
