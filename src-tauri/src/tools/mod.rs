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
    /// ユーザー発話からこのスキルを候補へ出す、副作用のない意図判定語彙。
    fn intent_keywords(&self) -> &'static [&'static str] {
        route_keywords(self.name())
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

pub mod http;
pub mod process;

pub struct ToolReg {
    pub make: fn() -> Box<dyn Tool>,
}
inventory::collect!(ToolReg);

// build.rs が生成した `pub mod time; pub mod read_file; ...` を取り込む。
include!(concat!(env!("OUT_DIR"), "/tool_mods.rs"));

/// 同梱 CLI ツール（`cargo xtask setup tools` → resources/bundled/tools/<ツール名>/）の探索先候補。
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

/// CLI ツールの実行パスを解決する。優先順: 明示設定（config_key）→ 同梱（setup tools）→ PATH。
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
        &[
            "天気", "天候", "気象", "気温", "雨", "降水", "予報", "weather",
        ],
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
            "書けた",
            "終わった",
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
        "desktop_control",
        &[
            "クリックして",
            "クリックし",
            "マウス",
            "キーボード",
            "キーを押",
            "入力して",
            "画面を操作",
            "pc操作",
            "デスクトップ操作",
            "ドラッグ",
        ],
    ),
    (
        "browser_control",
        &[
            "ブラウザ",
            "browser",
            "chrome",
            "edge",
            "ログインが必要",
            "ログインして",
            "開いて読",
            "画面を開いて",
        ],
    ),
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
        "glob_files",
        &[
            "ファイルを探",
            "ファイルはどこ",
            "どこにある",
            "ファイル名",
            "拡張子",
            "一覧して",
            "glob",
        ],
    ),
    (
        "grep_search",
        &[
            "使っている箇所",
            "使われている",
            "呼んでいる",
            "含むファイル",
            "中身を検索",
            "文字列を検索",
            "grep",
            "todo",
        ],
    ),
    (
        "edit_file",
        &["編集", "書き換え", "置換", "直して", "修正", "リファクタ"],
    ),
    (
        "todo_plan",
        &[
            "計画",
            "手順を整理",
            "やることを整理",
            "進捗",
            "どこまで終わ",
            "タスク整理",
        ],
    ),
    (
        "view_image",
        &[
            "画像を見",
            "スクリーンショット",
            "写真を見",
            "何が写",
            "この図",
            "画面の内容",
        ],
    ),
    (
        "ask_user",
        &[
            "どっち",
            "どれがいい",
            "選んで",
            "決めて",
            "おまかせ",
            "確認して",
        ],
    ),
    (
        "notebook_edit",
        &["ノートブック", "notebook", "ipynb", "jupyter", "セル"],
    ),
    (
        "command_job",
        &[
            "バックグラウンド",
            "ジョブ",
            "job",
            "実行中の",
            "起動したまま",
            "ログを見せ",
            "様子を見",
        ],
    ),
    (
        "project",
        &[
            "プロジェクト",
            "案件",
            "ワークスペース",
            "課題",
            "タスク",
            "やること",
            "残り",
            "締切",
            "期限",
            "決定事項",
            "決めたこと",
            "進捗",
            "経緯",
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

fn route_keywords(name: &str) -> &'static [&'static str] {
    TOOL_ROUTES
        .iter()
        .find_map(|(route_name, keywords)| (*route_name == name).then_some(*keywords))
        .unwrap_or(&[])
}

/// 登録済みスキルの意図メタデータだけで発話との一致を判定する。
pub fn intent_matches(name: &str, text: &str) -> bool {
    let normalized = text.to_lowercase();
    index()
        .get(name)
        .and_then(|position| all().get(*position))
        .is_some_and(|tool| {
            tool.intent_keywords()
                .iter()
                .any(|keyword| normalized.contains(&keyword.to_lowercase()))
        })
}

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
    for tool in all().iter().filter(|tool| tool.agent_callable()) {
        if selected.len() >= MAX_SELECTED_SCHEMAS {
            break;
        }
        if tool
            .intent_keywords()
            .iter()
            .any(|keyword| normalized.contains(&keyword.to_lowercase()))
        {
            push_schema_by_name(&mut selected, tool.name());
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

/// 自動計算時の下限。小さいコンテキストでも system・履歴・生成枠を残す。
const MIN_TOOL_OUTPUT_CHARS: usize = 1_000;
/// ツール出力の上限（設定でこれ以上は広げない。1回の出力で文脈を食い潰さないための天井）。
const MAX_TOOL_OUTPUT_CHARS: usize = 60_000;
/// クラウドプロバイダ利用時の既定（コンテキストが十分広いので固定値でよい）。
const CLOUD_TOOL_OUTPUT_CHARS: usize = 24_000;

/// ツール出力の最大文字数（コンテキストウィンドウ超過防止）。
///
/// 既定（`auto`）は**実際に使うモデルのコンテキスト長に合わせて**決める。同梱の
/// 既定モデル（Gemma 4 E2B / E4B）は `PICOAGENT_LOCAL_CTX=8192` なので、日本語では
/// 1 トークン ≒ 1 文字として使える幅そのものが狭く、ツール出力へ割けるのは全体の一部だけ。
/// 一方で `PICOAGENT_LOCAL_CTX` を広げた環境ではその分だけ多く読めた方がよいので、
/// 固定値ではなく ctx から算出して従来値を下回らないようにする。
pub fn tool_output_limit(cfg: &Config) -> usize {
    let configured = cfg.get("PICOAGENT_TOOL_OUTPUT_MAX_CHARS", "auto");
    let trimmed = configured.trim();
    if !trimmed.eq_ignore_ascii_case("auto") {
        if let Ok(value) = trimmed.parse::<usize>() {
            return value.clamp(1_000, MAX_TOOL_OUTPUT_CHARS);
        }
    }
    if cfg.get("PICOAGENT_PROVIDER", "local") != "local" {
        return CLOUD_TOOL_OUTPUT_CHARS;
    }
    let ctx = cfg
        .get("PICOAGENT_LOCAL_CTX", "8192")
        .parse::<usize>()
        .unwrap_or(8_192);
    // 人格 system・長期メモリ・会話履歴・ツール定義・生成分を残し、ツール出力へ 45% を配分する。
    // 旧実装は下限を8,000にしたため、既定 ctx=8,192 でも計算結果3,686を8,000へ
    // 戻してしまい、自動計算が実質機能していなかった。
    (ctx * 45 / 100).clamp(MIN_TOOL_OUTPUT_CHARS, MAX_TOOL_OUTPUT_CHARS)
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
    let limit = tool_output_limit(cfg);
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
    use super::schemas_for_text;

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
    fn 意図ルートは登録済みスキルだけを参照する() {
        let registered: std::collections::HashSet<&str> =
            super::all().iter().map(|tool| tool.name()).collect();
        for (name, keywords) in super::TOOL_ROUTES {
            assert!(
                registered.contains(name),
                "未登録スキルの意図ルートです: {name}"
            );
            assert!(!keywords.is_empty(), "意図語彙が空です: {name}");
        }
        for (text, name) in [
            ("今日の気象を教えて", "weather"),
            ("15分後にリマインダーして", "reminder"),
            ("Whisperで文字起こし", "whisper_transcribe"),
            ("競合比較を調査", "research_report"),
        ] {
            assert!(
                super::intent_matches(name, text),
                "{text} -> {name} が一致しません"
            );
            assert!(
                schema_names(text).contains(&name.to_string()),
                "{text} のschema候補に{name}がありません"
            );
        }
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
    fn ブラウザ操作スキルを候補に追加する() {
        let names = schema_names("このページをブラウザで開いて読んで");
        assert!(names.contains(&"browser_control".to_string()));
    }

    #[test]
    fn ツール出力の上限はコンテキスト長に追随する() {
        use crate::config::Config;
        let narrow = Config::new();
        narrow.set("PICOAGENT_LOCAL_CTX", "8192");
        assert_eq!(
            super::tool_output_limit(&narrow),
            3_686,
            "既定のコンテキストでも system・履歴・生成用に過半を残す"
        );

        let wide = Config::new();
        wide.set("PICOAGENT_LOCAL_CTX", "32768");
        assert!(
            super::tool_output_limit(&wide) > super::tool_output_limit(&narrow),
            "コンテキストを広げた分だけ多く読める"
        );

        let huge = Config::new();
        huge.set("PICOAGENT_LOCAL_CTX", "1000000");
        assert_eq!(
            super::tool_output_limit(&huge),
            super::MAX_TOOL_OUTPUT_CHARS,
            "天井は超えない"
        );

        let explicit = Config::new();
        explicit.set("PICOAGENT_TOOL_OUTPUT_MAX_CHARS", "12000");
        assert_eq!(super::tool_output_limit(&explicit), 12_000);
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
