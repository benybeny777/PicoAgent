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

pub mod process;

pub struct ToolReg {
    pub make: fn() -> Box<dyn Tool>,
}
inventory::collect!(ToolReg);

// build.rs が生成した `pub mod time; pub mod read_file; ...` を取り込む。
include!(concat!(env!("OUT_DIR"), "/tool_mods.rs"));

/// 同梱 CLI ツール（scripts/fetch-tools.mjs → resources/bundled/tools/<ツール名>/）の探索先候補。
/// 開発時（cwd 基準）とパッケージ後（実行ファイル基準）の両レイアウトに対応する。
fn bundled_tool_dirs(tool: &str) -> Vec<PathBuf> {
    let rel = format!("resources/bundled/tools/{tool}");
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("src-tauri").join(&rel));
        dirs.push(cwd.join(&rel));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join(&rel));
            dirs.push(parent.join("../Resources").join(&rel));
            dirs.push(parent.join("../..").join(&rel));
        }
    }
    dirs
}

/// ディレクトリ配下から実行ファイルを再帰探索する（アーカイブの展開レイアウト差異に強くする）。
fn find_bundled_exe(dir: &Path, exe_name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 7 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(exe_name) {
            return Some(path);
        }
        if path.is_dir() {
            subdirs.push(path);
        }
    }
    subdirs
        .into_iter()
        .find_map(|sub| find_bundled_exe(&sub, exe_name, depth + 1))
}

/// CLI ツールの実行パスを解決する。優先順: 明示設定（config_key）→ 同梱（setup:tools）→ PATH。
/// `exe` は拡張子なしの実行ファイル名（Windows では自動で .exe を付ける）。
pub fn resolve_cli_path(cfg: &Config, config_key: &str, tool_dir: &str, exe: &str) -> String {
    let configured = cfg.get(config_key, "");
    let trimmed = configured.trim();
    if !trimmed.is_empty() && trimmed != exe {
        return trimmed.to_string();
    }
    let exe_name = if cfg!(windows) {
        format!("{exe}.exe")
    } else {
        exe.to_string()
    };
    for dir in bundled_tool_dirs(tool_dir) {
        if let Some(found) = find_bundled_exe(&dir, &exe_name, 0) {
            return found.to_string_lossy().to_string();
        }
    }
    exe.to_string()
}

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
        "app_control",
        &[
            "アプリ終了",
            "アプリを終了",
            "アプリ閉じ",
            "アプリを閉じ",
            "このアプリ",
            "このソフト",
            "picoagent",
            "ピコエージェント",
            "pico agent",
        ],
    ),
    (
        "weather",
        &["天気", "気温", "雨", "降水", "予報", "weather"],
    ),
    (
        "currency_rates",
        &["為替", "換算", "円", "ドル", "ユーロ", "usd", "jpy", "eur"],
    ),
    (
        "market_indices",
        &[
            "日経平均",
            "日系平均",
            "nikkei",
            "topix",
            "トピックス",
            "ダウ",
            "nasdaq",
            "ナスダック",
            "s&p",
            "株価指数",
        ],
    ),
    (
        "news_scrape",
        &[
            "ニュース",
            "ニューす",
            "にゅーす",
            "news",
            "出来事",
            "時事",
            "ヘッドライン",
        ],
    ),
    (
        "rss_feed",
        &["rss", "atom", "フィード", "ニュース", "ブログ", "更新"],
    ),
    (
        "research_report",
        &[
            "調査",
            "リサーチ",
            "レポート",
            "出典",
            "競合比較",
            "市場調査",
            "まとめて調べ",
            "ダッシュボード",
            "html",
            "research",
            "report",
        ],
    ),
    (
        "spark_workspace",
        &[
            "genspark",
            "資料一式",
            "ワークスペース",
            "スライドとサイト",
            "サイトとスライド",
            "ページと資料",
            "spark",
            "deck",
            "brief",
        ],
    ),
    (
        "genspark_suite",
        &[
            "genspark全部",
            "genspark風",
            "deep research",
            "ai slides",
            "ai sheets",
            "ai docs",
            "sparkpage",
            "autopilot",
            "meeting notes",
            "multi-agent",
            "ai browser",
            "全部やって",
        ],
    ),
    (
        "subagent",
        &[
            "サブエージェント",
            "分担",
            "手分け",
            "それぞれ調べ",
            "並行で調べ",
            "担当",
            "subagent",
        ],
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
    (
        "reminder",
        &[
            "リマインダー",
            "思い出させて",
            "通知して",
            "あとで教えて",
            "reminder",
        ],
    ),
    (
        "focus_timer",
        &[
            "集中",
            "ポモドーロ",
            "タイマー",
            "見守り",
            "作業を見て",
            "focus",
            "pomodoro",
        ],
    ),
    (
        "praise_log",
        &[
            "できた",
            "やったこと",
            "がんばった",
            "褒め",
            "ほめ",
            "praise",
        ],
    ),
    (
        "daily_briefing",
        &[
            "おでかけ",
            "出かける",
            "外出",
            "持っていく",
            "出る前",
            "briefing",
        ],
    ),
    (
        "whisper_transcribe",
        &["文字起こし", "whisper", "transcribe", "話者", "speaker"],
    ),
    (
        "meeting_recorder",
        &["録音", "議事録", "会議メモ", "meeting notes"],
    ),
    (
        "clipboard_text",
        &["クリップボード", "コピーした", "clipboard"],
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
        "project_workspace",
        &[
            "ワークスペース",
            "プロジェクト切り替え",
            "仕事用",
            "作業場所",
        ],
    ),
    (
        "workflow",
        &[
            "ワークフロー",
            "手順を覚えて",
            "この操作を覚えて",
            "自動化",
            "再実行",
        ],
    ),
    (
        "trash_file",
        &[
            "削除",
            "消して",
            "ゴミ箱",
            "捨てて",
            "ファイル削除",
            "フォルダ削除",
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
    (
        "code_runner",
        &[
            "スクリプト",
            "コードを書いて",
            "コード実行",
            "プログラムで",
            "自力で",
            "なんとかして",
            "script",
            "code",
        ],
    ),
    (
        "task_scheduler",
        &[
            "スケジュール",
            "スケジューラ",
            "cron",
            "定期実行",
            "指定時刻",
            "毎朝",
            "毎日",
            "毎週",
        ],
    ),
    (
        "slack_send_message",
        &["slack", "スラック", "slack履歴", "slack history"],
    ),
    ("discord_send_message", &["discord", "ディスコード"]),
    (
        "gmail_message",
        &[
            "gmail",
            "メール",
            "下書き",
            "送信",
            "未読",
            "受信",
            "メール一覧",
            "振り分け",
            "ラベル",
            "ゴミメール",
            "掃除",
            "アーカイブ",
            "既読",
            "ゴミ箱",
        ],
    ),
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
    (
        "productivity_tools",
        &[
            "文字数",
            "行数",
            "json",
            "base64",
            "urlエンコード",
            "urlデコード",
            "単位換算",
            "日付計算",
            "csv",
            "markdown表",
            "スラッグ",
            "slug",
            "チェックサム",
            "bmi",
            "rgb",
            "hex",
        ],
    ),
    ("pdf_tools", &["pdf", "結合", "抽出", "回転"]),
    (
        "image_tools",
        &[
            "psd",
            "xcf",
            "heic",
            "heif",
            "tiff",
            "webp",
            "avif",
            "画像変換",
            "pngに",
            "png化",
            "jpgに",
            "jpegに",
            "リサイズ",
            "縮小して",
            "画像情報",
            "レイヤー",
        ],
    ),
    (
        "comfyui_generate",
        &[
            "画像",
            "絵",
            "イラスト",
            "描いて",
            "生成して",
            "comfyui",
            "image",
            "写真",
            "動画",
        ],
    ),
    ("image_generate", &["dall-e", "dall\u{00b7}e", "openai画像"]),
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

/// エージェントが会話中に使えるスキル（ツール）のラベル一覧を「、」区切りで返す。
/// 「何ができる？」への回答をモデルが具体的に答えられるよう、system プロンプトに添える用途。
pub fn capability_summary() -> String {
    let mut labels: Vec<&str> = all()
        .iter()
        .filter(|t| t.agent_callable())
        .map(|t| t.label())
        .collect();
    labels.dedup();
    labels.join("、")
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
    // 外部 MCP サーバのツール（ユーザーが明示設定したもの）は常に候補へ加える。
    for schema in crate::mcp::client::schemas() {
        if selected.len() >= MAX_SELECTED_SCHEMAS + 8 {
            break;
        }
        selected.push(schema);
    }
    selected
}

/// ツール出力の最大文字数（コンテキストウィンドウ超過防止）。
const MAX_TOOL_OUTPUT_CHARS: usize = 8_000;

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
    let result = dispatch_interactive(cfg, name, args).await;
    match run_command::parse_approval_marker(&result) {
        Some(pending) => format!(
            "エラー: この操作には実行許可が必要です（この経路では承認を受け付けられません）。\n{}\n\nチャットで同じコマンドを直接依頼すると、その場で許可・永続許可・キャンセルを選べます。",
            run_command::approval_prompt(&pending)
        ),
        None => result,
    }
}

/// モデルが要求したツール呼び出しを実行する（対話ループ専用の生の入口）。
/// 承認マーカーをそのまま返すため、**呼び出し側が承認フローを実装している場合のみ**使う。
/// 失敗は例外にせず文字列で返す。出力が長すぎる場合は MAX_TOOL_OUTPUT_CHARS で打ち切る。
pub async fn dispatch_interactive(cfg: &Config, name: &str, args: Value) -> String {
    let result = if let Some(idx) = index().get(name) {
        all()[*idx].run(cfg, args).await
    } else if crate::mcp::client::is_mcp_tool(name) {
        // 外部 MCP サーバのツール（mcp__<サーバ>__<ツール>）へフォールバックする。
        crate::mcp::client::dispatch(name, args).await
    } else {
        format!("エラー: 未登録のツール \"{}\" が呼び出されました。", name)
    };
    if result.chars().count() > MAX_TOOL_OUTPUT_CHARS {
        let truncated: String = result.chars().take(MAX_TOOL_OUTPUT_CHARS).collect();
        format!(
            "{}\n\n[...出力が長いため{}文字で打ち切りました]",
            truncated, MAX_TOOL_OUTPUT_CHARS
        )
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::schemas_for_text;

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
        assert!(names.contains(&"spark_workspace".to_string()));
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
