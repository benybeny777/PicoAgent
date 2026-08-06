// providers/agent_cli.rs - 手元の CLI エージェント（Claude Code / Codex）を叩く AI として使う。
//
// ねらいは**開発・個人利用**。同梱の小型ローカルモデルでは判断が怪しい場面で、
// 手元に入っている高性能な CLI をそのまま応答生成へ回せると、挙動の切り分けが速い。
//
// 制約（重要）:
// - **ツール呼び出し（function calling）は行わない。** CLI は自前のエージェントループを
//   持っており、こちらの 60 本超のスキルとは役割が衝突する。ここでは1往復の
//   テキスト応答だけを受け取り、`tool_calls` は常に空で返す。スキルを使いたい場合は
//   local / OpenAI 互換など通常のプロバイダを選ぶこと。
// - CLI の起動に毎回1〜3秒かかる。常駐アプリの会話としては重い。
// - 配布物の既定として使うと、利用者側に CLI の導入とログインを要求することになり、
//   「Ollama も要らない」という前提と逆行する。**販売する構成での既定にはしない。**
//
// プロンプトは引数として渡す（シェルを経由しない）ので、文字列の解釈による事故は起きない。


use async_trait::async_trait;
use serde_json::Value;

use super::{Provider, ProviderReg};
use crate::config::Config;
use crate::message::{Field, Message};

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
    if !trimmed.is_empty() {
        return trimmed.split_whitespace().map(str::to_string).collect();
    }
    match kind(cfg) {
        Kind::Codex => vec!["exec".to_string()],
        Kind::Claude => vec!["-p".to_string()],
        Kind::Custom => Vec::new(),
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

#[async_trait]
impl Provider for AgentCli {
    fn id(&self) -> &'static str {
        "agent_cli"
    }

    fn label(&self) -> &'static str {
        "手元の CLI エージェント（開発用・スキル非対応）"
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
                "非対話実行の引数（空なら種類ごとの既定。claude は -p、codex は exec）",
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
        // ツール定義は受け取るが使わない（このプロバイダは1往復のテキスト応答だけを返す）。
        let _ = tools;
        let prompt = flatten(messages);
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
        Ok(Message::new("assistant", content))
    }
}

inventory::submit! { ProviderReg { make: || Box::new(AgentCli) } }

#[cfg(test)]
mod tests {
    use super::{flatten, kind, lead_args, program, timeout_secs, Kind, MAX_PROMPT_CHARS};
    use crate::config::Config;
    use crate::message::Message;

    #[test]
    fn 種類ごとに既定の実行ファイルと引数が変わる() {
        let cfg = Config::new();
        assert_eq!(program(&cfg), "claude");
        assert_eq!(lead_args(&cfg), vec!["-p".to_string()]);

        let codex = Config::new();
        codex.set("PICOAGENT_AGENT_CLI_KIND", "codex");
        assert!(matches!(kind(&codex), Kind::Codex));
        assert_eq!(program(&codex), "codex");
        assert_eq!(lead_args(&codex), vec!["exec".to_string()]);
    }

    #[test]
    fn 明示指定は種類より優先する() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_AGENT_CLI_PATH", "/opt/bin/my-cli");
        cfg.set("PICOAGENT_AGENT_CLI_ARGS", "run --quiet");
        assert_eq!(program(&cfg), "/opt/bin/my-cli");
        assert_eq!(lead_args(&cfg), vec!["run".to_string(), "--quiet".to_string()]);
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

    #[test]
    fn 待ち時間は範囲外なら既定へ戻す() {
        assert_eq!(timeout_secs(&Config::new()), 180);
        let cfg = Config::new();
        cfg.set("PICOAGENT_AGENT_CLI_TIMEOUT_SECS", "99999");
        assert_eq!(timeout_secs(&cfg), 180);
    }
}
