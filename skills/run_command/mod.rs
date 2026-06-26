// skills/run_command/mod.rs - allowlist 制限付きのコマンド実行スキル。
//
// shell は使わず program + args で起動する。既定は無効で、設定で有効化し、
// PICOAGENT_COMMAND_ALLOWLIST に一致する prefix だけを実行する。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_ARGS: usize = 32;
const MAX_ARG_CHARS: usize = 500;
const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const DEFAULT_ALLOWLIST: &str = "git status,git diff,npm test,cargo test";

pub struct RunCommandTool;

fn home() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn command_root(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.get("PICOAGENT_COMMAND_ROOT", &home()))
}

fn normalize(base: &Path, input: &str) -> PathBuf {
    let joined = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        base.join(input)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => {
                out.pop();
            }
            CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn bounded_cwd(base: &Path, cwd: Option<&str>) -> Result<PathBuf, String> {
    let target = normalize(base, cwd.unwrap_or("."));
    if target == base || target.starts_with(base) {
        Ok(target)
    } else {
        Err(format!(
            "許可ルート（{}）の外では実行できません: {}",
            base.display(),
            target.display()
        ))
    }
}

fn parse_args(args: &Value) -> Result<Vec<String>, String> {
    let Some(values) = args.get("args") else {
        return Ok(Vec::new());
    };
    let array = values
        .as_array()
        .ok_or_else(|| "args は文字列配列で指定してください。".to_string())?;
    if array.len() > MAX_ARGS {
        return Err(format!("args は {} 個以内で指定してください。", MAX_ARGS));
    }
    let mut out = Vec::new();
    for value in array {
        let arg = value
            .as_str()
            .ok_or_else(|| "args は文字列配列で指定してください。".to_string())?;
        if arg.chars().count() > MAX_ARG_CHARS {
            return Err(format!("各 arg は {} 文字以内で指定してください。", MAX_ARG_CHARS));
        }
        out.push(arg.to_string());
    }
    Ok(out)
}

fn command_tokens(program: &str, args: &[String]) -> Vec<String> {
    let mut tokens = Vec::with_capacity(args.len() + 1);
    tokens.push(program.to_string());
    tokens.extend(args.iter().cloned());
    tokens
}

fn allowlist_entries(cfg: &Config) -> Vec<Vec<String>> {
    cfg.get("PICOAGENT_COMMAND_ALLOWLIST", DEFAULT_ALLOWLIST)
        .split(',')
        .filter_map(|entry| {
            let tokens: Vec<String> = entry
                .split_whitespace()
                .filter(|token| !token.is_empty())
                .map(ToString::to_string)
                .collect();
            if tokens.is_empty() {
                None
            } else {
                Some(tokens)
            }
        })
        .collect()
}

fn allowed(cfg: &Config, program: &str, args: &[String]) -> bool {
    let tokens = command_tokens(program, args);
    allowlist_entries(cfg).iter().any(|entry| {
        entry.len() <= tokens.len()
            && entry
                .iter()
                .zip(tokens.iter())
                .all(|(allowed, actual)| allowed == actual)
    })
}

fn trim_output(label: &str, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= MAX_OUTPUT_BYTES {
        return format!("{}:\n{}", label, text);
    }
    let mut end = MAX_OUTPUT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}:\n{}\n…（{} バイトで打ち切り）", label, &text[..end], MAX_OUTPUT_BYTES)
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn label(&self) -> &'static str {
        "コマンド実行"
    }

    fn description(&self) -> String {
        "許可ルート内で、設定の allowlist に一致するローカルコマンドだけを実行する。ビルド、テスト、git 状態確認などに使う。既定では無効。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "program": {
                    "type": "string",
                    "description": "実行するプログラム名。shell は使われない。"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "プログラムへ渡す引数。"
                },
                "cwd": {
                    "type": "string",
                    "description": "作業ディレクトリ。許可ルート配下のみ。省略時は許可ルート。"
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 120,
                    "description": "実行待ち上限秒。省略時は設定値。"
                }
            },
            "required": ["program"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_COMMAND_ENABLED", "run_command を有効化（true/false）", "text", "false"),
            Field::new("PICOAGENT_COMMAND_ROOT", "run_command の実行許可ルート", "text", &home()),
            Field::new(
                "PICOAGENT_COMMAND_ALLOWLIST",
                "run_command の許可コマンド（カンマ区切り prefix）",
                "textarea",
                DEFAULT_ALLOWLIST,
            ),
            Field::new("PICOAGENT_COMMAND_TIMEOUT_SECS", "run_command の既定タイムアウト秒（1〜120）", "text", "30"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        if !cfg.get_bool("PICOAGENT_COMMAND_ENABLED", false) {
            return "エラー: run_command は無効です。設定画面で PICOAGENT_COMMAND_ENABLED を true にしてください。"
                .to_string();
        }
        let program = match args.get("program").and_then(Value::as_str).map(str::trim) {
            Some(program) if !program.is_empty() => program,
            _ => return "エラー: program が指定されていません。".to_string(),
        };
        let parsed_args = match parse_args(&args) {
            Ok(parsed_args) => parsed_args,
            Err(error) => return format!("エラー: {}", error),
        };
        if !allowed(cfg, program, &parsed_args) {
            return format!(
                "エラー: 許可されていないコマンドです: {} {}",
                program,
                parsed_args.join(" ")
            );
        }

        let base = command_root(cfg);
        let cwd = match bounded_cwd(&base, args.get("cwd").and_then(Value::as_str)) {
            Ok(cwd) => cwd,
            Err(error) => return format!("エラー: {}", error),
        };
        let default_timeout = cfg
            .get("PICOAGENT_COMMAND_TIMEOUT_SECS", "30")
            .parse::<u64>()
            .ok()
            .filter(|value| (1..=120).contains(value))
            .unwrap_or(30);
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .filter(|value| (1..=120).contains(value))
            .unwrap_or(default_timeout);

        let mut command = Command::new(program);
        command
            .args(&parsed_args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        hide_window(&mut command);

        let output = match tokio::time::timeout(Duration::from_secs(timeout_secs), command.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => return format!("エラー: コマンド起動に失敗しました（{}）", error),
            Err(_) => return format!("エラー: コマンドが {} 秒でタイムアウトしました。", timeout_secs),
        };
        let mut result = format!(
            "終了コード: {}\n作業ディレクトリ: {}\n",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "シグナル終了".to_string()),
            cwd.display()
        );
        result.push_str(&trim_output("stdout", &output.stdout));
        result.push('\n');
        result.push_str(&trim_output("stderr", &output.stderr));
        result
    }
}

inventory::submit! { ToolReg { make: || Box::new(RunCommandTool) } }

#[cfg(test)]
mod tests {
    use super::{bounded_cwd, command_tokens};
    use std::path::PathBuf;

    #[test]
    fn コマンドトークンをprefix比較用に組み立てる() {
        assert_eq!(
            command_tokens("git", &["status".to_string(), "--short".to_string()]),
            vec!["git", "status", "--short"]
        );
    }

    #[test]
    fn 実行cwdのルート外を拒否する() {
        let base = PathBuf::from(r"C:\work\root");
        assert!(bounded_cwd(&base, Some(r"..\other")).is_err());
    }
}
