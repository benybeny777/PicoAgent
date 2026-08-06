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
// - CLI の起動に毎回1〜3秒かかる。常駐アプリの会話としては重い。
// - 配布物の既定として使うと、利用者側に CLI の導入とログインを要求することになり、
//   「Ollama も要らない」という前提と逆行する。**販売する構成での既定にはしない。**
//
// プロンプトは引数として渡す（シェルを経由しない）ので、文字列の解釈による事故は起きない。
// Claude / Codex 自身のツールは既定で封じ、PicoAgent が提示したスキルだけを JSON 経由で
// 呼ばせる。ここを開けると PicoAgent の権限境界・監査・物理削除拒否を迂回できてしまう。


use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message, ToolCall};

pub struct AgentCli;

/// CLI へ渡すプロンプトの上限。これを超える分は古い履歴から落とす。
const MAX_PROMPT_CHARS: usize = 24_000;

/// 対応している CLI の種類。
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Claude,
    Codex,
    Custom,
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

fn program(cfg: &Config) -> String {
    let configured = cfg.get("PICOAGENT_AGENT_CLI_PATH", "");
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    match kind(cfg) {
        Kind::Codex => "codex".to_string(),
        _ => "claude".to_string(),
    }
}

/// 非対話で1往復だけ実行するための引数。プロンプト本体はこの後ろへ足す。
fn lead_args(cfg: &Config) -> Vec<String> {
    let configured = cfg.get("PICOAGENT_AGENT_CLI_ARGS", "");
    let trimmed = configured.trim();
    let configured: Vec<String> = if trimmed.is_empty() {
        Vec::new()
    } else {
        trimmed.split_whitespace().map(str::to_string).collect()
    };
    match kind(cfg) {
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

fn timeout_secs(cfg: &Config) -> u64 {
    cfg.get("PICOAGENT_AGENT_CLI_TIMEOUT_SECS", "180")
        .parse::<u64>()
        .ok()
        .filter(|value| (10..=600).contains(value))
        .unwrap_or(180)
}

/// 会話を1つのプロンプト文字列へ畳む。CLI は1往復しか受け取れないため、
/// 役割が分かるように明示のラベルを付ける。長すぎる場合は**古い方から**落とす。
fn flatten(messages: &[Message]) -> String {
    let mut blocks: Vec<String> = Vec::new();
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
        blocks.push(format!("{label}\n{}", message.content.trim()));
    }
    let mut text = blocks.join("\n\n");
    if text.chars().count() > MAX_PROMPT_CHARS {
        // 先頭（＝古い履歴）から削る。直近のやり取りと最後の指示を残すため。
        let skip = text.chars().count() - MAX_PROMPT_CHARS;
        text = text.chars().skip(skip).collect();
    }
    text
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
    format!(
        "［使えるツール］\n{}\n\n         ［ツールを使うときの返し方］\n         上のツールが必要なときは、前置きも説明も書かず、次の形の JSON だけを返してください。\n         {{\"tool\": \"ツール名\", \"args\": {{ 引数 }}}}\n         ツールが不要なら、いつもどおり日本語の文章で答えてください。",
        lines.join("\n")
    )
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
        // CLI 側が選ぶため、こちらからは列挙しない。
        Ok(vec![self.default_model(cfg)])
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
        _model: &str,
        messages: &[Message],
        tools: &[Value],
    ) -> anyhow::Result<Message> {
        let mut prompt = flatten(messages);
        let instructions = tool_instructions(tools);
        if !instructions.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&instructions);
        }
        if prompt.trim().is_empty() {
            anyhow::bail!("CLI へ渡す内容がありません。");
        }
        let program = program(cfg);
        let mut command = crate::tools::process::command(&program);
        command.args(lead_args(cfg));
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
        let content = String::from_utf8_lossy(&output.stdout).trim().to_string();
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
    use super::{
        flatten, kind, lead_args, parse_tool_call, program, timeout_secs, tool_instructions, Kind,
        MAX_PROMPT_CHARS,
    };
    use crate::config::Config;
    use crate::message::Message;

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
        assert!(codex_args.windows(2).any(|pair| pair == ["--sandbox", "read-only"]));
        assert!(codex_args.iter().any(|arg| arg == "--ignore-user-config"));
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
    fn 会話は役割ラベル付きで畳まれる() {
        let text = flatten(&[
            Message::new("system", "あなたはむぎ"),
            Message::new("user", "こんにちは"),
            Message::new("assistant", "やあ"),
        ]);
        assert!(text.contains("［前提］\nあなたはむぎ"), "text: {text}");
        assert!(text.contains("［ユーザー］\nこんにちは"), "text: {text}");
        assert!(text.contains("［これまでのあなたの返答］\nやあ"), "text: {text}");
    }

    #[test]
    fn 空の内容は畳み込みから省く() {
        let text = flatten(&[Message::new("user", "  "), Message::new("user", "本題")]);
        assert_eq!(text, "［ユーザー］\n本題");
    }

    #[test]
    fn 長すぎる会話は古い方から落とす() {
        let long = "あ".repeat(MAX_PROMPT_CHARS);
        let text = flatten(&[Message::new("user", long), Message::new("user", "最後の指示")]);
        assert_eq!(text.chars().count(), MAX_PROMPT_CHARS);
        assert!(text.ends_with("最後の指示"), "直近の指示は必ず残す");
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
        assert!(text.contains("get_time（引数: timezone）: 現在時刻を返す"), "text: {text}");
        assert!(text.contains("\"tool\""), "返し方の JSON 形を示す: {text}");
    }

    #[test]
    fn ツールが無ければ指示文は空() {
        assert!(tool_instructions(&[]).is_empty());
    }

    #[test]
    fn 決められたjsonをツール呼び出しへ変換する() {
        let call = parse_tool_call(r#"{"tool":"get_time","args":{"timezone":"Asia/Tokyo"}}"#, &sample_tools())
            .expect("ツール呼び出しとして解釈する");
        assert_eq!(call.name, "get_time");
        assert_eq!(call.arguments["timezone"], "Asia/Tokyo");
    }

    #[test]
    fn コードフェンスや前後の文章があっても取り出す() {
        let fenced = "```json\n{\"tool\":\"get_time\",\"args\":{}}\n```";
        assert!(parse_tool_call(fenced, &sample_tools()).is_some(), "フェンス付きを剥がす");
        let chatty = "了解です。\n{\"tool\":\"get_time\",\"args\":{}}";
        assert!(parse_tool_call(chatty, &sample_tools()).is_some(), "前置きがあっても拾う");
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
