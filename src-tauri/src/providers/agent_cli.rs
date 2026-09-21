// providers/agent_cli.rs - 手元の CLI エージェント（Claude Code / Codex）を叩く AI として使う。
//
// ねらいは**開発・個人利用**。同梱の小型ローカルモデルでは判断が怪しい場面で、
// 手元に入っている高性能な CLI をそのまま応答生成へ回せると、挙動の切り分けが速い。
//
// ツール呼び出しについて:
// CLI は OpenAI 形式の function calling を受け付けないため、**プロンプト側の約束事**で
// 代替する。使えるスキルの一覧を渡し、「ツールを使うときは決められた JSON だけを返す」と
// 指示して、その JSON を `tool_calls` へ変換する。JSON でなければ通常の返答として扱うので、
// 約束を守らなかった場合でも壊れず、単にツールなしの応答になる（安全側に倒れる）。
// 知らないツール名を返してきた場合も、実行せず通常の返答として扱う。
//
// 制約（重要）:
// - CLI のプロセス起動は応答ごとに行う。同じ論理会話では明示セッションIDを再開するが、
//   常駐プロセスにはせず、停止・失敗時に毎回回収する。
// - 配布物の既定として使うと、利用者側に CLI の導入とログインを要求することになり、
//   「Ollama も要らない」という前提と逆行する。**販売する構成での既定にはしない。**
//
// プロンプトは引数として渡す（シェルを経由しない）ので、文字列の解釈による事故は起きない。
// Claude / Codex 自身のツールは既定で封じ、PicoAgent が提示したスキルだけを JSON 経由で
// 呼ばせる。ここを開けると PicoAgent の権限境界・監査・物理削除拒否を迂回できてしまう。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, ToolCall};

#[path = "agent_cli/models.rs"]
mod models;

pub struct AgentCli;

/// CLI へ渡すプロンプトの上限。これを超える分は古い履歴から落とす。
const MAX_PROMPT_CHARS: usize = 24_000;
/// 長い会話でも人格・安全指示を残すため system に確保する最大枠。
const MAX_SYSTEM_CHARS: usize = 12_000;
/// ツール一覧だけで Windows のコマンドライン上限を使い切らないための天井。
const MAX_TOOL_INSTRUCTION_CHARS: usize = 10_000;

/// 対応している CLI の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Claude,
    Codex,
    Custom,
}

/// 1つの論理会話だけで共有するCLIセッション。CLIが作る永続セッションIDは設定ファイルへ
/// 保存せず、このアプリ起動中だけ保持する。別会話・別候補へ同じ値を渡さない。
#[derive(Clone, Default)]
pub struct Session {
    inner: Arc<Mutex<SessionState>>,
}

#[derive(Default)]
struct SessionState {
    kind: Option<Kind>,
    id: Option<String>,
    in_use: bool,
    /// このCLIセッションへ最後に渡したsystem文脈。再開ターンは履歴を送り直さないため、
    /// これと違う内容になったときだけ更新分を明示し、古い前提を置き換えさせる。
    system: Option<String>,
}

tokio::task_local! {
    static ACTIVE_SESSION: Session;
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }
}

/// 指定した論理エージェントの実行中だけCLIセッションを有効にする。
pub async fn with_session<F: std::future::Future>(session: Session, task: F) -> F::Output {
    ACTIVE_SESSION.scope(session, task).await
}

/// 通常会話で設定変更・CLI切替を検出するための識別子。
pub fn session_identity(cfg: &Config, model: &str) -> String {
    let selected = kind_for_model(cfg, model);
    format!(
        "{:?}|{}|{}|{}|{:?}",
        selected,
        program_for_kind(cfg, selected),
        cfg.get("PICOAGENT_AGENT_CLI_ARGS", ""),
        model,
        models::default_model(selected).ok().flatten()
    )
}

struct SessionUse {
    session: Session,
    resume_id: Option<String>,
    /// 再開先のCLIがすでに持っているsystem文脈。新規セッションでは `None`。
    sent_system: Option<String>,
    committed: bool,
}

impl SessionUse {
    fn begin(session: Session, selected: Kind) -> anyhow::Result<Self> {
        let mut state = session.inner.lock().unwrap();
        if state.in_use {
            anyhow::bail!(
                "同じCLIセッションが並列に呼ばれました。処理を分けて再実行してください。"
            );
        }
        if state.kind != Some(selected) {
            state.kind = Some(selected);
            state.id = None;
            state.system = None;
        }
        state.in_use = true;
        let resume_id = state.id.clone();
        let sent_system = state.system.clone();
        drop(state);
        Ok(Self {
            session,
            resume_id,
            sent_system,
            committed: false,
        })
    }

    /// 再開先のCLIが持っている前提と、今回のリクエストの前提が食い違うか。
    fn system_changed(&self, system: &str) -> bool {
        self.sent_system.as_deref() != Some(system)
    }

    fn commit(mut self, id: String, system: String) {
        let mut state = self.session.inner.lock().unwrap();
        state.id = Some(id);
        state.system = Some(system);
        state.in_use = false;
        self.committed = true;
    }
}

impl Drop for SessionUse {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut state = self.session.inner.lock().unwrap();
        // 中断・解析失敗・CLI異常終了後のセッションは途中ターンを含む可能性がある。
        // 次回へ無言で引き継がず、新しい明示セッションからやり直す。
        state.id = None;
        state.system = None;
        state.in_use = false;
    }
}

fn kind(cfg: &Config) -> Kind {
    match cfg
        .get("PICOAGENT_AGENT_CLI_KIND", "claude")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "codex" => Kind::Codex,
        "custom" => Kind::Custom,
        _ => Kind::Claude,
    }
}

/// マルチエージェントからモデル名として `claude` / `codex` を指定した場合は、
/// グローバル設定を変えずにその実行だけCLIの種類を切り替える。
fn kind_for_model(cfg: &Config, model: &str) -> Kind {
    match model
        .trim()
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "claude" => Kind::Claude,
        "codex" => Kind::Codex,
        "custom" => Kind::Custom,
        _ => kind(cfg),
    }
}

fn program_for_kind(cfg: &Config, selected: Kind) -> String {
    // 設定した実行パスは、設定した種類のCLIにだけ使う。マルチAIの候補側で
    // `claude` / `codex` を指定して切り替えた場合にこのパスを流用すると、
    // 片方のCLIの実行ファイルへもう片方の引数（`exec --sandbox read-only` など）を
    // 渡してしまうため、切り替えた側は既定のコマンド名で起動する。
    if selected == kind(cfg) {
        let configured = cfg.get("PICOAGENT_AGENT_CLI_PATH", "");
        let trimmed = configured.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    match selected {
        Kind::Codex => "codex".to_string(),
        _ => "claude".to_string(),
    }
}

fn program(cfg: &Config) -> String {
    program_for_kind(cfg, kind(cfg))
}

/// セッションを使わない非対話1往復の引数。プロンプト本体はこの後ろへ足す。
fn lead_args_for_kind(cfg: &Config, selected: Kind) -> Vec<String> {
    // 追加引数も実行パスと同じく、設定した種類のCLIにだけ渡す。
    // 候補側で切り替えた別CLIへ、そのCLIが解釈できない引数を混ぜない。
    let configured = if selected == kind(cfg) {
        cfg.get("PICOAGENT_AGENT_CLI_ARGS", "")
    } else {
        String::new()
    };
    let trimmed = configured.trim();
    let configured: Vec<String> = if trimmed.is_empty() {
        Vec::new()
    } else {
        trimmed.split_whitespace().map(str::to_string).collect()
    };
    match selected {
        Kind::Codex => {
            let mut args = vec!["exec".to_string()];
            args.extend(configured);
            // 安全指定は追加引数より後ろへ置き、同名オプションで上書きされないようにする。
            args.extend([
                "--ignore-user-config".to_string(),
                "--sandbox".to_string(),
                "read-only".to_string(),
                "--ephemeral".to_string(),
                "--skip-git-repo-check".to_string(),
            ]);
            args
        }
        Kind::Claude => {
            let mut args = vec!["-p".to_string()];
            args.extend(configured);
            args.extend([
                "--safe-mode".to_string(),
                "--tools".to_string(),
                String::new(),
                "--permission-mode".to_string(),
                "plan".to_string(),
                "--no-session-persistence".to_string(),
            ]);
            args
        }
        // custom はどの CLI か判断できないため、明示された引数をそのまま使う。
        Kind::Custom => configured,
    }
}

fn session_args_for_kind(cfg: &Config, selected: Kind, resume_id: Option<&str>) -> Vec<String> {
    let mut args = lead_args_for_kind(cfg, selected);
    match selected {
        Kind::Claude => {
            args.retain(|arg| arg != "--no-session-persistence");
            args.extend(["--output-format".to_string(), "json".to_string()]);
            if let Some(id) = resume_id {
                args.extend(["--resume".to_string(), id.to_string()]);
            }
        }
        Kind::Codex => {
            args.retain(|arg| arg != "--ephemeral");
            args.push("--json".to_string());
            // Codexのexec共通オプションはresumeより前へ置く必要がある。
            if let Some(id) = resume_id {
                args.extend(["resume".to_string(), id.to_string()]);
            }
        }
        Kind::Custom => {}
    }
    args
}

#[cfg(test)]
fn lead_args(cfg: &Config) -> Vec<String> {
    lead_args_for_kind(cfg, kind(cfg))
}

/// モデル指定は新規・再開の両方へ適用し、Codexではresumeより前へ置く。
fn request_args(
    cfg: &Config,
    selected: Kind,
    model: &str,
    resume: Option<Option<&str>>,
) -> anyhow::Result<Vec<String>> {
    let mut args = match resume {
        Some(id) => session_args_for_kind(cfg, selected, id),
        None => lead_args_for_kind(cfg, selected),
    };
    if selected == Kind::Custom {
        return Ok(args);
    }
    let explicit = model
        .trim()
        .split_once(':')
        .filter(|(kind, _)| {
            kind.eq_ignore_ascii_case("claude") || kind.eq_ignore_ascii_case("codex")
        })
        .map(|(_, model)| model.trim())
        .filter(|model| !model.is_empty());
    // 既存の追加引数によるモデル指定は、モデル選択が「既定」のときだけ維持する。
    let has_argument = args
        .iter()
        .any(|arg| arg == "--model" || arg == "-m" || arg.starts_with("--model="));
    let chosen = if let Some(model) = explicit {
        let mut filtered = Vec::new();
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            if arg == "--model" || arg == "-m" {
                iter.next();
            } else if !arg.starts_with("--model=") {
                filtered.push(arg);
            }
        }
        args = filtered;
        Some(model.to_owned())
    } else if has_argument {
        None
    } else {
        models::default_model(selected)?
    };
    if let Some(model) = chosen {
        let index = if selected == Kind::Codex && resume.flatten().is_some() {
            args.len() - 2
        } else {
            args.len()
        };
        args.splice(index..index, ["--model".to_owned(), model]);
    }
    Ok(args)
}

fn timeout_secs(cfg: &Config) -> u64 {
    cfg.get("PICOAGENT_AGENT_CLI_TIMEOUT_SECS", "180")
        .parse::<u64>()
        .ok()
        .filter(|value| (10..=600).contains(value))
        .unwrap_or(180)
}

fn model_choices(
    default: String,
    mut read: impl FnMut(Kind) -> anyhow::Result<Vec<String>>,
) -> Vec<String> {
    let mut choices = vec![default];
    for (selected, prefix) in [(Kind::Claude, "claude"), (Kind::Codex, "codex")] {
        if !choices.iter().any(|value| value == prefix) {
            choices.push(prefix.into());
        }
        match read(selected) {
            Ok(models) => {
                choices.extend(models.into_iter().map(|model| format!("{prefix}:{model}")))
            }
            // フォールバック許可: 候補取得の失敗はCLIごとに隔離する。実行時の既定設定エラーは
            // request_argsで伝播させ、誤ったモデルによる推論は開始しない。
            Err(error) => crate::log_paths::log_detached(&format!(
                "{prefix}のモデル候補を取得できませんでした: {error}"
            )),
        }
    }
    choices
}

/// 正規形会話をCLIの1ターン用プロンプトへ畳む。初回またはセッション非対応CLIでは
/// 履歴全体、明示セッション再開時は必要な差分を渡す。長すぎる場合は**古い方から**落とす。
fn truncate_keep_edges(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    const MARKER: &str = "\n［...中略...］\n";
    let marker_len = MARKER.chars().count();
    if limit <= marker_len + 2 {
        return text.chars().take(limit).collect();
    }
    let content_limit = limit - marker_len;
    let head_len = content_limit * 2 / 3;
    let tail_len = content_limit - head_len;
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text.chars().skip(count.saturating_sub(tail_len)).collect();
    format!("{head}{MARKER}{tail}")
}

fn flatten(messages: &[Message], limit: usize) -> String {
    let mut blocks: Vec<(bool, String)> = Vec::new();
    for message in messages {
        if message.content.trim().is_empty() {
            continue;
        }
        let label = match message.role.as_str() {
            "system" => "［前提］",
            "assistant" => "［これまでのあなたの返答］",
            "tool" => "［ツールの結果］",
            _ => "［ユーザー］",
        };
        blocks.push((
            message.role == "system",
            format!("{label}\n{}", message.content.trim()),
        ));
    }
    let full = blocks
        .iter()
        .map(|(_, block)| block.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if full.chars().count() <= limit {
        return full;
    }

    // 古い履歴を落とす場合も system は先頭へ残す。旧実装は文字列全体の先頭を
    // 切っていたため、会話が長くなると人格・安全指示から先に消えていた。
    let system = blocks
        .iter()
        .filter(|(is_system, _)| *is_system)
        .map(|(_, block)| block.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let system_limit = system.chars().count().min(MAX_SYSTEM_CHARS).min(limit);
    let system = truncate_keep_edges(&system, system_limit);
    let separator = if system.is_empty() { 0 } else { 2 };
    let recent_limit = limit.saturating_sub(system.chars().count() + separator);
    let recent = blocks
        .iter()
        .filter(|(is_system, _)| !*is_system)
        .map(|(_, block)| block.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let recent_count = recent.chars().count();
    let recent = if recent_count > recent_limit {
        recent
            .chars()
            .skip(recent_count - recent_limit)
            .collect::<String>()
    } else {
        recent
    };
    match (system.is_empty(), recent.is_empty()) {
        (false, false) => format!("{system}\n\n{recent}"),
        (false, true) => system,
        _ => recent,
    }
}

/// 再開済みCLIは以前のassistant発話まで知っている。通常会話とツールループでは、
/// 最後のassistantより後ろだけを送り、履歴全文を二重投入しない。分担計画→統合のように
/// 独立したメッセージ列にはassistantが無いため、その指示全体を次ターンとして送る。
fn flatten_for_resume(messages: &[Message], limit: usize) -> String {
    let start = messages
        .iter()
        .rposition(|message| message.role == "assistant")
        .map(|index| index + 1)
        .unwrap_or(0);
    flatten(&messages[start..], limit)
}

/// 会話のsystemを1つの文字列にまとめる。検索連動の長期メモリ・プロジェクト文脈・関係性文脈は
/// リクエストごとにここへ差し込まれるため、再開ターンでも変化を検出できるようにする。
fn system_text(messages: &[Message]) -> String {
    messages
        .iter()
        .filter(|message| message.role == "system" && !message.content.trim().is_empty())
        .map(|message| message.content.trim())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 再開中のCLIは初回ターンのsystemしか知らない。前提が変わったターンでは最新版を明示して渡し、
/// 古い長期メモリやプロジェクト文脈のまま答えさせない。
fn resumed_system_block(system: &str, limit: usize) -> String {
    const HEAD: &str = "［前提の更新］ここまでの前提は次の内容へ置き換えてください。";
    let body_limit = limit
        .saturating_sub(HEAD.chars().count() + 1)
        .min(MAX_SYSTEM_CHARS);
    if body_limit == 0 {
        return String::new();
    }
    format!("{HEAD}\n{}", truncate_keep_edges(system.trim(), body_limit))
}

/// CLIは更新案内やログイン案内を本文の前後へ混ぜることがある。1行ずつ試して
/// **JSONとして読めた行だけ**を応答として扱い、案内文でターンごと失敗させない。
fn first_json_value(text: &str) -> Option<Value> {
    serde_json::from_str::<Value>(text.trim()).ok().or_else(|| {
        text.lines()
            .find_map(|line| serde_json::from_str(line.trim()).ok())
    })
}

fn parse_claude_session_output(text: &str) -> Result<(String, String), String> {
    let value =
        first_json_value(text).ok_or_else(|| "Claude CLIのJSON応答を読めません。".to_string())?;
    if value.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Err(value
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("Claude CLIがエラーを返しました。")
            .to_string());
    }
    let content = value
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() || session_id.is_empty() {
        return Err("Claude CLIのJSON応答に本文またはsession_idがありません。".to_string());
    }
    Ok((content, session_id))
}

fn parse_codex_session_output(text: &str) -> Result<(String, String), String> {
    let mut session_id = String::new();
    let mut content = String::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        // JSONL以外の案内行（更新通知・キャッシュ警告など）は読み飛ばす。
        // 必要な `thread.started` と本文が取れなければ、最後にまとめて失敗させる。
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                session_id = value
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            Some("item.completed")
                if value
                    .get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("agent_message") =>
            {
                content = value
                    .get("item")
                    .and_then(|item| item.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            _ => {}
        }
    }
    if content.is_empty() || session_id.is_empty() {
        return Err("Codex CLIのJSONL応答に最終本文またはthread_idがありません。".to_string());
    }
    Ok((content, session_id))
}

/// 使えるスキルの一覧と、ツールを使うときの返し方を伝える指示文を作る。
///
/// CLI は指示追従が堅いので、`tools` の JSON Schema をそのまま見せるより、
/// 名前・説明・引数キーだけに削った方が読み違えが少ない。
fn tool_instructions(tools: &[Value]) -> String {
    let mut lines = Vec::new();
    for tool in tools {
        let Some(function) = tool.get("function") else {
            continue;
        };
        let Some(name) = function.get("name").and_then(Value::as_str) else {
            continue;
        };
        let description = function
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(160)
            .collect::<String>();
        let params = function
            .get("parameters")
            .and_then(|parameters| parameters.get("properties"))
            .and_then(Value::as_object)
            .map(|properties| properties.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        if params.is_empty() {
            lines.push(format!("- {name}: {description}"));
        } else {
            lines.push(format!("- {name}（引数: {params}）: {description}"));
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let instructions = format!(
        "［使えるツール］\n{}\n\n         ［ツールを使うときの返し方］\n         上のツールが必要なときは、前置きも説明も書かず、次の形の JSON だけを返してください。\n         {{\"tool\": \"ツール名\", \"args\": {{ 引数 }}}}\n         ツールが不要なら、いつもどおり日本語の文章で答えてください。",
        lines.join("\n")
    );
    truncate_keep_edges(&instructions, MAX_TOOL_INSTRUCTION_CHARS)
}

/// CLI の出力からツール呼び出しを取り出す。JSON でなければ `None`（＝通常の返答）。
///
/// 約束を守らなかった出力で壊れないよう、判定は保守的にする。
/// コードフェンスは剥がし、前後に文章が付いていても最初の JSON オブジェクトだけを見る。
/// **一覧に無いツール名は採用しない**（実行できないうえ、幻覚をそのまま実行に回さない）。
fn parse_tool_call(text: &str, tools: &[Value]) -> Option<ToolCall> {
    let known: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool.get("function")?.get("name")?.as_str())
        .collect();

    let mut body = text.trim();
    if let Some(rest) = body.strip_prefix("```") {
        // ```json … ``` の形。言語名の行を落として閉じフェンスまでを見る。
        let rest = rest.split_once('\n').map(|(_, rest)| rest).unwrap_or(rest);
        body = rest.split("```").next().unwrap_or(rest).trim();
    }
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    if end <= start {
        return None;
    }
    let parsed: Value = serde_json::from_str(&body[start..=end]).ok()?;
    let name = parsed.get("tool").and_then(Value::as_str)?.trim();
    if name.is_empty() || !known.contains(&name) {
        return None;
    }
    let arguments = parsed
        .get("args")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    Some(ToolCall {
        id: format!("agent_cli-{name}"),
        name: name.to_string(),
        arguments,
    })
}

#[async_trait]
impl Provider for AgentCli {
    fn id(&self) -> &'static str {
        "agent_cli"
    }

    fn label(&self) -> &'static str {
        "手元の CLI エージェント（開発用）"
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_AGENT_CLI_KIND",
                "使う CLI（claude / codex / custom）",
                "text",
                "claude",
            ),
            Field::new(
                "PICOAGENT_AGENT_CLI_PATH",
                "CLI の実行パス（空なら種類から自動）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_AGENT_CLI_ARGS",
                "追加の実行引数（Claude / Codex には安全な非対話引数を常に付加）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_AGENT_CLI_TIMEOUT_SECS",
                "1回の応答を待つ秒数（10〜600）",
                "text",
                "180",
            ),
        ]
    }

    fn default_model(&self, cfg: &Config) -> String {
        // モデルの選択は CLI 側の設定に従う。ここでは種類を表示名として返す。
        match kind(cfg) {
            Kind::Codex => "codex".to_string(),
            Kind::Custom => "custom".to_string(),
            Kind::Claude => "claude".to_string(),
        }
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        Ok(model_choices(self.default_model(cfg), models::list))
    }

    async fn is_up(&self, cfg: &Config) -> bool {
        let mut command = crate::tools::process::command(program(cfg));
        command.arg("--version");
        crate::tools::process::output(command, 15, "CLI エージェントの確認")
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    async fn chat(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
    ) -> anyhow::Result<Message> {
        let selected = kind_for_model(cfg, model);
        let session = ACTIVE_SESSION.try_with(Clone::clone).ok();
        // custom CLIは応答形式もセッション引数も分からないため、従来どおり1往復に限定する。
        let mut session_use = if selected == Kind::Custom {
            None
        } else {
            session
                .map(|session| SessionUse::begin(session, selected))
                .transpose()?
        };
        let instructions = tool_instructions(tools);
        let instruction_chars = instructions.chars().count();
        let conversation_limit = MAX_PROMPT_CHARS
            .saturating_sub(instruction_chars.saturating_add(2))
            .max(1_000);
        let current_system = system_text(messages);
        let mut prompt = if session_use
            .as_ref()
            .and_then(|session| session.resume_id.as_ref())
            .is_some()
        {
            // 再開ターンは履歴を送り直さないので、systemが変わっていれば更新分だけ先に渡す。
            // これが無いと、検索連動の長期メモリ・プロジェクト文脈・関係性文脈が初回ターンの
            // 内容のまま固定され、切り替えても古い前提で答え続ける。
            let system_block = if session_use
                .as_ref()
                .is_some_and(|session| session.system_changed(&current_system))
                && !current_system.trim().is_empty()
            {
                resumed_system_block(&current_system, conversation_limit / 2)
            } else {
                String::new()
            };
            let rest_limit = conversation_limit
                .saturating_sub(system_block.chars().count() + 2)
                .max(1_000);
            // 自己評価の再生成のように、末尾がassistantのまま呼ばれると差分が空になる。
            // そのまま空プロンプトで失敗させるとセッションIDまで破棄され、次ターンの
            // 会話継続が黙って切れるため、この呼び出しだけ会話全文へ戻す（systemを含む）。
            let diff = flatten_for_resume(messages, rest_limit);
            if diff.trim().is_empty() {
                flatten(messages, conversation_limit)
            } else if system_block.is_empty() {
                diff
            } else {
                format!("{system_block}\n\n{diff}")
            }
        } else {
            flatten(messages, conversation_limit)
        };
        if !instructions.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&instructions);
        }
        if prompt.trim().is_empty() {
            anyhow::bail!("CLI へ渡す内容がありません。");
        }
        let program = program_for_kind(cfg, selected);
        let mut command = crate::tools::process::command(&program);
        command.args(request_args(
            cfg,
            selected,
            model,
            session_use
                .as_ref()
                .map(|session| session.resume_id.as_deref()),
        )?);
        // プロンプトは引数として渡す。シェルを経由しないので解釈の事故が起きない。
        command.arg(&prompt);

        let output = crate::tools::process::output(command, timeout_secs(cfg), "CLI エージェント")
            .await
            .map_err(|error| {
                anyhow::anyhow!("{error}（{program} が入っているか、PATH と PICOAGENT_AGENT_CLI_PATH を確認してください）")
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "CLI エージェントが失敗しました: {}",
                stderr.trim().chars().take(500).collect::<String>()
            );
        }
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let content = match selected {
            Kind::Claude if session_use.is_some() => {
                let (content, returned_id) =
                    parse_claude_session_output(&raw).map_err(anyhow::Error::msg)?;
                if let Some(requested) = session_use
                    .as_ref()
                    .and_then(|session| session.resume_id.as_deref())
                {
                    if requested != returned_id {
                        anyhow::bail!(
                            "Claude CLIが指定したセッションを再開せず、別のsession_idを返しました。"
                        );
                    }
                }
                session_use
                    .take()
                    .unwrap()
                    .commit(returned_id, current_system.clone());
                content
            }
            Kind::Codex if session_use.is_some() => {
                let (content, returned_id) =
                    parse_codex_session_output(&raw).map_err(anyhow::Error::msg)?;
                if let Some(requested) = session_use
                    .as_ref()
                    .and_then(|session| session.resume_id.as_deref())
                {
                    // Codexは存在しないIDを渡すと成功扱いで新規threadを作る版がある。
                    // ID一致を必須にし、無言で文脈を失ったまま続行しない。
                    if requested != returned_id {
                        anyhow::bail!(
                            "Codex CLIが指定したセッションを再開せず、別のthread_idを返しました。"
                        );
                    }
                }
                session_use
                    .take()
                    .unwrap()
                    .commit(returned_id, current_system.clone());
                content
            }
            _ => raw,
        };
        if content.is_empty() {
            anyhow::bail!("CLI エージェントが空の応答を返しました（ログインが済んでいるか確認してください）。");
        }
        if let Some(call) = parse_tool_call(&content, tools) {
            let mut message = Message::new("assistant", String::new());
            message.tool_calls = Some(vec![call]);
            return Ok(message);
        }
        Ok(Message::new("assistant", content))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(AgentCli) } }

#[cfg(test)]
mod tests {
    #[test]
    fn 片方の候補取得に失敗しても正常側と既定選択を残す() {
        let choices = super::model_choices("claude".into(), |selected| {
            if selected == super::Kind::Codex {
                anyhow::bail!("キャッシュ破損");
            }
            Ok(vec!["opus".into(), "future-claude".into()])
        });
        assert_eq!(
            choices,
            vec!["claude", "claude:opus", "claude:future-claude", "codex"]
        );
    }
    #[test]
    fn 選択モデルは新規と再開へ安全指定を保って渡す() {
        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_AGENT_CLI_KIND", "codex");
        cfg.set("PICOAGENT_AGENT_CLI_ARGS", "--model old-model");
        for resume in [None, Some(None), Some(Some("thread-id"))] {
            let args = super::request_args(&cfg, super::Kind::Codex, "codex:future-model", resume)
                .unwrap();
            assert!(args
                .windows(2)
                .any(|pair| pair == ["--model", "future-model"]));
            assert!(!args.contains(&"old-model".into()));
            assert!(args.contains(&"--ignore-user-config".into()));
            if let Some(index) = args.iter().position(|arg| arg == "resume") {
                assert!(args.iter().position(|arg| arg == "--model").unwrap() < index);
            }
        }
        assert_eq!(
            super::kind_for_model(&cfg, "claude:sonnet"),
            super::Kind::Claude
        );
        let args =
            super::request_args(&cfg, super::Kind::Claude, "claude:future-claude", None).unwrap();
        assert!(args.contains(&"--safe-mode".into()));
        assert!(args.contains(&"future-claude".into()));
        assert_ne!(
            super::session_identity(&cfg, "codex:a"),
            super::session_identity(&cfg, "codex:b")
        );
    }
    use super::{
        flatten, flatten_for_resume, kind, kind_for_model, lead_args, lead_args_for_kind,
        parse_claude_session_output, parse_codex_session_output, parse_tool_call, program,
        program_for_kind, resumed_system_block, session_args_for_kind, system_text, timeout_secs,
        tool_instructions, Kind, Session, SessionUse, MAX_PROMPT_CHARS,
    };
    use crate::config::Config;
    use crate::message::Message;

    #[test]
    fn 再開時の差分が空になる会話では全文へ戻す() {
        // 自己評価の再生成は末尾がassistantのまま呼ばれるため、差分は空になる。
        // 空のまま送るとCLI呼び出しが失敗し、SessionUseのDropでセッションIDまで消える。
        let messages = [
            Message::new("system", "人格"),
            Message::new("user", "質問"),
            Message::new("assistant", "短い回答"),
        ];
        assert!(flatten_for_resume(&messages, 1_000).trim().is_empty());
        assert!(!flatten(&messages, 1_000).trim().is_empty());
        // 末尾がuserなら従来どおり差分だけを送る。
        let next = [
            Message::new("user", "前の質問"),
            Message::new("assistant", "前の回答"),
            Message::new("user", "次の質問"),
        ];
        let diff = flatten_for_resume(&next, 1_000);
        assert!(diff.contains("次の質問"));
        assert!(!diff.contains("前の質問"));
    }

    #[test]
    fn 再開中にsystemが変わったターンだけ前提を送り直す() {
        let session = Session::new();
        // 初回ターン：全文を送るので、そのときのsystemを覚える。
        SessionUse::begin(session.clone(), Kind::Claude)
            .unwrap()
            .commit(
                "session-1".to_string(),
                "人格\n\n[プロジェクト文脈]\nA\n[/プロジェクト文脈]".to_string(),
            );

        // 同じ前提の再開ターンでは更新分を足さない。
        let same = SessionUse::begin(session.clone(), Kind::Claude).unwrap();
        assert_eq!(same.resume_id.as_deref(), Some("session-1"));
        assert!(!same.system_changed("人格\n\n[プロジェクト文脈]\nA\n[/プロジェクト文脈]"));
        // プロジェクトを切り替えた（＝systemが書き換わった）ターンでは送り直す。
        assert!(same.system_changed("人格\n\n[プロジェクト文脈]\nB\n[/プロジェクト文脈]"));
        same.commit(
            "session-1".to_string(),
            "人格\n\n[プロジェクト文脈]\nB\n[/プロジェクト文脈]".to_string(),
        );

        // 送り直した内容が次ターンの基準になる。
        let next = SessionUse::begin(session, Kind::Claude).unwrap();
        assert!(!next.system_changed("人格\n\n[プロジェクト文脈]\nB\n[/プロジェクト文脈]"));

        let block =
            resumed_system_block("人格\n\n[プロジェクト文脈]\nB\n[/プロジェクト文脈]", 1_000);
        assert!(block.starts_with("［前提の更新］"));
        assert!(block.contains("[プロジェクト文脈]\nB"));
    }

    #[test]
    fn systemの本文だけを取り出して変化を見る() {
        let messages = [
            Message::new("system", "  人格  "),
            Message::new("user", "質問"),
            Message::new("system", "追加の前提"),
            Message::new("system", "   "),
        ];
        assert_eq!(system_text(&messages), "人格\n\n追加の前提");
        assert!(system_text(&[Message::new("user", "質問")]).is_empty());
    }

    #[test]
    fn cliの案内行が混ざってもjson応答を読み取る() {
        let claude = parse_claude_session_output(
            "新しいバージョンがあります\n{\"result\":\"7\",\"session_id\":\"abc\"}",
        )
        .unwrap();
        assert_eq!(claude, ("7".to_string(), "abc".to_string()));

        let codex = parse_codex_session_output(
            "モデルキャッシュを読み込めませんでした\n{\"type\":\"thread.started\",\"thread_id\":\"xyz\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"7\"}}",
        )
        .unwrap();
        assert_eq!(codex.1, "xyz");
        assert_eq!(codex.0, "7");
    }

    #[test]
    fn 種類ごとに既定の実行ファイルと引数が変わる() {
        let cfg = Config::new();
        assert_eq!(program(&cfg), "claude");
        let claude_args = lead_args(&cfg);
        assert_eq!(claude_args.first().map(String::as_str), Some("-p"));
        assert!(claude_args.windows(2).any(|pair| pair == ["--tools", ""]));
        assert!(claude_args.iter().any(|arg| arg == "--safe-mode"));

        let codex = Config::new();
        codex.set("PICOAGENT_AGENT_CLI_KIND", "codex");
        assert!(matches!(kind(&codex), Kind::Codex));
        assert_eq!(program(&codex), "codex");
        let codex_args = lead_args(&codex);
        assert_eq!(codex_args.first().map(String::as_str), Some("exec"));
        assert!(codex_args
            .windows(2)
            .any(|pair| pair == ["--sandbox", "read-only"]));
        assert!(codex_args.iter().any(|arg| arg == "--ignore-user-config"));
        assert!(matches!(kind_for_model(&cfg, "codex"), Kind::Codex));
        assert!(matches!(kind_for_model(&codex, "claude"), Kind::Claude));
    }

    #[test]
    fn セッション利用時だけ永続化と明示id再開を有効にする() {
        let claude = Config::new();
        let first = session_args_for_kind(&claude, Kind::Claude, None);
        assert!(!first.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(first
            .windows(2)
            .any(|pair| pair == ["--output-format", "json"]));
        let resumed = session_args_for_kind(&claude, Kind::Claude, Some("claude-session"));
        assert!(resumed
            .windows(2)
            .any(|pair| pair == ["--resume", "claude-session"]));

        let codex = Config::new();
        codex.set("PICOAGENT_AGENT_CLI_KIND", "codex");
        let first = session_args_for_kind(&codex, Kind::Codex, None);
        assert!(!first.iter().any(|arg| arg == "--ephemeral"));
        assert!(first.iter().any(|arg| arg == "--json"));
        let resumed = session_args_for_kind(&codex, Kind::Codex, Some("codex-thread"));
        assert!(resumed
            .windows(2)
            .any(|pair| pair == ["resume", "codex-thread"]));
        let resume_index = resumed.iter().position(|arg| arg == "resume").unwrap();
        assert!(resumed[..resume_index]
            .windows(2)
            .any(|pair| pair == ["--sandbox", "read-only"]));
    }

    #[test]
    fn cliの構造化出力から本文と明示セッションidを取る() {
        let (content, id) = parse_claude_session_output(
            r#"{"type":"result","is_error":false,"result":"回答","session_id":"claude-1"}"#,
        )
        .unwrap();
        assert_eq!((content.as_str(), id.as_str()), ("回答", "claude-1"));

        let codex = "{\"type\":\"thread.started\",\"thread_id\":\"codex-1\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"途中\"}}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"最終回答\"}}";
        let (content, id) = parse_codex_session_output(codex).unwrap();
        assert_eq!((content.as_str(), id.as_str()), ("最終回答", "codex-1"));
    }

    #[test]
    fn 再開時は最後のassistant以後だけを送る() {
        let messages = vec![
            Message::new("system", "人格"),
            Message::new("user", "最初"),
            Message::new("assistant", "前の回答"),
            Message::new("user", "続き"),
        ];
        let prompt = flatten_for_resume(&messages, MAX_PROMPT_CHARS);
        assert_eq!(prompt, "［ユーザー］\n続き");
    }

    #[test]
    fn 中断または失敗したcliセッションは次回へ引き継がない() {
        let session = Session::new();
        SessionUse::begin(session.clone(), Kind::Claude)
            .unwrap()
            .commit("session-1".to_string(), "人格".to_string());
        let resumed = SessionUse::begin(session.clone(), Kind::Claude).unwrap();
        assert_eq!(resumed.resume_id.as_deref(), Some("session-1"));
        drop(resumed); // commitせず終了＝中断・解析失敗
        let fresh = SessionUse::begin(session, Kind::Claude).unwrap();
        assert!(fresh.resume_id.is_none());
    }

    #[test]
    fn 実行パスと追加引数を明示指定できる() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_AGENT_CLI_PATH", "/opt/bin/my-cli");
        cfg.set("PICOAGENT_AGENT_CLI_ARGS", "run --quiet");
        assert_eq!(program(&cfg), "/opt/bin/my-cli");
        let args = lead_args(&cfg);
        assert_eq!(&args[..3], ["-p", "run", "--quiet"]);
        assert!(args.iter().any(|arg| arg == "--safe-mode"));
    }

    #[test]
    fn 明示指定の実行パスと追加引数は設定した種類のcliにだけ使う() {
        // マルチAIの候補が `codex` を指定した場合、claude 用に設定したパスと追加引数を
        // 流用すると、claude の実行ファイルへ codex の引数を渡してしまう。
        let cfg = Config::new();
        cfg.set("PICOAGENT_AGENT_CLI_PATH", "/opt/bin/my-claude");
        cfg.set("PICOAGENT_AGENT_CLI_ARGS", "--model opus");
        assert_eq!(program_for_kind(&cfg, Kind::Claude), "/opt/bin/my-claude");
        assert_eq!(program_for_kind(&cfg, Kind::Codex), "codex");
        assert!(lead_args_for_kind(&cfg, Kind::Claude).contains(&"opus".to_string()));
        assert!(!lead_args_for_kind(&cfg, Kind::Codex).contains(&"opus".to_string()));
        assert_eq!(
            lead_args_for_kind(&cfg, Kind::Codex)
                .first()
                .map(String::as_str),
            Some("exec")
        );
    }

    #[test]
    fn 会話は役割ラベル付きで畳まれる() {
        let text = flatten(
            &[
                Message::new("system", "あなたはむぎ"),
                Message::new("user", "こんにちは"),
                Message::new("assistant", "やあ"),
            ],
            MAX_PROMPT_CHARS,
        );
        assert!(text.contains("［前提］\nあなたはむぎ"), "text: {text}");
        assert!(text.contains("［ユーザー］\nこんにちは"), "text: {text}");
        assert!(
            text.contains("［これまでのあなたの返答］\nやあ"),
            "text: {text}"
        );
    }

    #[test]
    fn 空の内容は畳み込みから省く() {
        let text = flatten(
            &[Message::new("user", "  "), Message::new("user", "本題")],
            MAX_PROMPT_CHARS,
        );
        assert_eq!(text, "［ユーザー］\n本題");
    }

    #[test]
    fn 長すぎる会話は古い方から落とす() {
        let long = "あ".repeat(MAX_PROMPT_CHARS);
        let text = flatten(
            &[
                Message::new("user", long),
                Message::new("user", "最後の指示"),
            ],
            MAX_PROMPT_CHARS,
        );
        assert_eq!(text.chars().count(), MAX_PROMPT_CHARS);
        assert!(text.ends_with("最後の指示"), "直近の指示は必ず残す");
    }

    #[test]
    fn 長い会話でもsystemの先頭と末尾を残す() {
        let system = format!(
            "人格の先頭{}安全指示の末尾",
            "前提".repeat(MAX_PROMPT_CHARS)
        );
        let text = flatten(
            &[
                Message::new("system", system),
                Message::new("user", "直近の依頼".repeat(3_000)),
            ],
            MAX_PROMPT_CHARS,
        );
        assert!(text.starts_with("［前提］\n人格の先頭"), "text: {text}");
        assert!(text.contains("安全指示の末尾"), "system 末尾も残す");
        assert!(text.ends_with("直近の依頼"), "直近の依頼も残す");
        assert!(text.chars().count() <= MAX_PROMPT_CHARS);
    }

    fn sample_tools() -> Vec<serde_json::Value> {
        vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "get_time",
                "description": "現在時刻を返す",
                "parameters": { "type": "object", "properties": { "timezone": {} } }
            }
        })]
    }

    #[test]
    fn ツール一覧と返し方を指示文に載せる() {
        let text = tool_instructions(&sample_tools());
        assert!(
            text.contains("get_time（引数: timezone）: 現在時刻を返す"),
            "text: {text}"
        );
        assert!(text.contains("\"tool\""), "返し方の JSON 形を示す: {text}");
    }

    #[test]
    fn ツールが無ければ指示文は空() {
        assert!(tool_instructions(&[]).is_empty());
    }

    #[test]
    fn 決められたjsonをツール呼び出しへ変換する() {
        let call = parse_tool_call(
            r#"{"tool":"get_time","args":{"timezone":"Asia/Tokyo"}}"#,
            &sample_tools(),
        )
        .expect("ツール呼び出しとして解釈する");
        assert_eq!(call.name, "get_time");
        assert_eq!(call.arguments["timezone"], "Asia/Tokyo");
    }

    #[test]
    fn コードフェンスや前後の文章があっても取り出す() {
        let fenced = "```json\n{\"tool\":\"get_time\",\"args\":{}}\n```";
        assert!(
            parse_tool_call(fenced, &sample_tools()).is_some(),
            "フェンス付きを剥がす"
        );
        let chatty = "了解です。\n{\"tool\":\"get_time\",\"args\":{}}";
        assert!(
            parse_tool_call(chatty, &sample_tools()).is_some(),
            "前置きがあっても拾う"
        );
    }

    #[test]
    fn 通常の文章はツール呼び出しにしない() {
        assert!(parse_tool_call("こんにちは。今日はいい天気だね。", &sample_tools()).is_none());
    }

    #[test]
    fn 一覧に無いツール名は採用しない() {
        // 幻覚したツール名をそのまま実行へ回さない（実行できないうえ危険）。
        let text = r#"{"tool":"rm_rf_everything","args":{}}"#;
        assert!(parse_tool_call(text, &sample_tools()).is_none());
    }

    #[test]
    fn 引数が無い場合は空オブジェクトにする() {
        let call = parse_tool_call(r#"{"tool":"get_time"}"#, &sample_tools()).unwrap();
        assert!(call.arguments.is_object());
    }

    #[test]
    fn 待ち時間は範囲外なら既定へ戻す() {
        assert_eq!(timeout_secs(&Config::new()), 180);
        let cfg = Config::new();
        cfg.set("PICOAGENT_AGENT_CLI_TIMEOUT_SECS", "99999");
        assert_eq!(timeout_secs(&cfg), 180);
    }
}
