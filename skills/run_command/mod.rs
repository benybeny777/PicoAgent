// skills/run_command/mod.rs - allowlist 制限付きのコマンド実行スキル。
//
// shell は使わず program + args で起動する。既定は無効で、設定で有効化し、
// PICOAGENT_COMMAND_ALLOWLIST に完全一致するコマンドだけを実行する。
//
// `background=true` のときは待たずに起動して job_id を返す（開発サーバ・長いビルド用）。
// 起動後の様子見と停止は `command_job` スキルが担う。実体は `jobs.rs`。

pub mod jobs;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_ARGS: usize = 32;
const MAX_ARG_CHARS: usize = 500;
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;
const DEFAULT_ALLOWLIST: &str = "git status,git diff,npm test,cargo test";
const APPROVAL_MARKER: &str = "__PICOAGENT_COMMAND_APPROVAL__";

pub struct RunCommandTool;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingCommandApproval {
    pub token: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub timeout_secs: u64,
    pub reason: String,
}

// 実行の許可ルートは全スキル共通の permissions を参照する。
pub fn command_root(cfg: &Config) -> PathBuf {
    crate::permissions::primary_root(cfg)
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

pub fn bounded_cwd(cfg: &Config, cwd: Option<&str>) -> Result<PathBuf, String> {
    crate::permissions::resolve(cfg, cwd.unwrap_or("."))
        .map(|path| path.as_path().to_path_buf())
}

// Windows / Program Files などの保護領域は全スキル共通の permissions で判定する。
pub fn is_protected_edit_path(path: &Path) -> bool {
    crate::permissions::is_system_protected(path)
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

fn command_entry(program: &str, args: &[String]) -> String {
    command_tokens(program, args).join(" ")
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
    allowlist_entries(cfg).iter().any(|entry| entry == &tokens)
}


fn approval_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("cmd{nanos}")
}

fn approval_marker(pending: &PendingCommandApproval) -> String {
    format!(
        "{}{}",
        APPROVAL_MARKER,
        serde_json::to_string(pending).unwrap_or_else(|_| "{}".to_string())
    )
}

pub fn parse_approval_marker(text: &str) -> Option<PendingCommandApproval> {
    let raw = text.strip_prefix(APPROVAL_MARKER)?;
    serde_json::from_str(raw).ok()
}

pub fn approval_prompt(pending: &PendingCommandApproval) -> String {
    format!(
        "このコマンドはまだ許可されていません。\n\nコマンド: {} {}\n作業ディレクトリ: {}\n理由: {}\n\n実行する場合は次のどれかを送ってください。\n- 「許可」: 今回だけ実行\n- 「永続許可」: 今後も同じコマンド全体と作業ディレクトリを許可して実行\n- 「キャンセル」: 実行しない",
        pending.program,
        pending.args.join(" "),
        pending.cwd,
        pending.reason
    )
}

pub fn trim_output(label: &str, bytes: &[u8]) -> String {
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

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn label(&self) -> &'static str {
        "コマンド実行"
    }

    fn description(&self) -> String {
        "許可ルート内で、設定の allowlist に一致するローカルコマンドだけを実行する。ビルド、テスト、git 状態確認などに使う。完了を待てない長い処理は background=true で起動し、command_job で様子を見る。既定では無効。"
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
                    "description": "実行待ち上限秒。省略時は設定値。background=true のときは無視する。"
                },
                "background": {
                    "type": "boolean",
                    "description": "true なら完了を待たずに起動し、job_id を返す。開発サーバや長いビルドに使う。途中出力の取得と停止は command_job で行う。"
                }
            },
            "required": ["program"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_COMMAND_ENABLED", "run_command を有効化（true/false）", "text", "false"),
            Field::new(
                "PICOAGENT_COMMAND_ALLOWLIST",
                "run_command の許可コマンド（カンマ区切り・完全一致）",
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
        if crate::tools::process::is_physical_delete(program, &parsed_args) {
            return crate::tools::process::physical_delete_error();
        }
        let base = command_root(cfg);
        let requested_cwd = args.get("cwd").and_then(Value::as_str).unwrap_or(".");
        let cwd = normalize(&base, requested_cwd);
        if is_protected_edit_path(&cwd) {
            return format!(
                "エラー: Windows / Program Files 配下は編集不可のため、コマンド実行先にできません: {}",
                cwd.display()
            );
        }
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

        let permission_error = if !allowed(cfg, program, &parsed_args) {
            Some(format!(
                "allowlist 未登録: {} {}",
                program,
                parsed_args.join(" ")
            ))
        } else {
            bounded_cwd(cfg, Some(requested_cwd)).err()
        };
        if let Some(reason) = permission_error {
            return approval_marker(&PendingCommandApproval {
                token: approval_token(),
                program: program.to_string(),
                args: parsed_args,
                cwd: cwd.to_string_lossy().to_string(),
                timeout_secs,
                reason,
            });
        }

        // バックグラウンド起動も、ここまでの許可判定（allowlist・cwd・物理削除拒否）を
        // 通ったものだけが到達する。
        if args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return match jobs::spawn(program, &parsed_args, &cwd) {
                Ok(id) => format!(
                    "OK: バックグラウンドで起動しました（{id}）: {} {}\n途中経過は command_job の output、停止は stop を使ってください。",
                    program,
                    parsed_args.join(" ")
                ),
                Err(error) => format!("エラー: {error}"),
            };
        }

        execute_approved(program, &parsed_args, &cwd, timeout_secs).await
    }
}

pub async fn execute_pending(pending: &PendingCommandApproval) -> String {
    let cwd = PathBuf::from(&pending.cwd);
    if crate::tools::process::is_physical_delete(&pending.program, &pending.args) {
        return crate::tools::process::physical_delete_error();
    }
    if is_protected_edit_path(&cwd) {
        return format!(
            "エラー: Windows / Program Files 配下は編集不可のため、コマンド実行先にできません: {}",
            cwd.display()
        );
    }
    execute_approved(&pending.program, &pending.args, &cwd, pending.timeout_secs).await
}

pub async fn execute_with_persistent_approval(
    cfg: &Config,
    pending: &PendingCommandApproval,
) -> String {
    let cwd = PathBuf::from(&pending.cwd);
    if crate::tools::process::is_physical_delete(&pending.program, &pending.args) {
        return crate::tools::process::physical_delete_error();
    }
    if is_protected_edit_path(&cwd) {
        return format!(
            "エラー: Windows / Program Files 配下は編集不可のため、コマンド実行先にできません: {}",
            cwd.display()
        );
    }

    let mut patch = Map::new();
    let mut saved = Vec::new();
    if !allowed(cfg, &pending.program, &pending.args) {
        let entry = command_entry(&pending.program, &pending.args);
        let current = cfg.get("PICOAGENT_COMMAND_ALLOWLIST", DEFAULT_ALLOWLIST);
        let next = if current.trim().is_empty() {
            entry.clone()
        } else {
            format!("{},{}", current.trim(), entry)
        };
        patch.insert(
            "PICOAGENT_COMMAND_ALLOWLIST".to_string(),
            Value::String(next),
        );
        saved.push(format!("コマンド `{entry}`"));
    }
    if bounded_cwd(cfg, Some(&pending.cwd)).is_err() {
        let entry = cwd.to_string_lossy().to_string();
        let current = cfg.get("PICOAGENT_ALLOWED_PATHS", "");
        let exists = current
            .split(['\n', ';'])
            .map(str::trim)
            .any(|item| item == entry);
        if !exists {
            let next = if current.trim().is_empty() {
                entry.clone()
            } else {
                format!("{}\n{}", current.trim(), entry)
            };
            patch.insert(
                "PICOAGENT_ALLOWED_PATHS".to_string(),
                Value::String(next),
            );
            saved.push(format!("追加許可パス `{entry}`"));
        }
    }
    if !patch.is_empty() {
        if let Err(error) = cfg.set_all(patch) {
            return format!("エラー: 永続許可の保存に失敗しました（{}）", error);
        }
    }

    let mut result = if saved.is_empty() {
        "永続許可は既に保存済みでした。\n".to_string()
    } else {
        format!("永続許可として保存しました: {}\n", saved.join("、"))
    };
    result.push_str(&execute_pending(pending).await);
    result
}

async fn execute_approved(
    program: &str,
    parsed_args: &[String],
    cwd: &Path,
    timeout_secs: u64,
) -> String {
    if crate::tools::process::is_physical_delete(program, parsed_args) {
        return crate::tools::process::physical_delete_error();
    }
    let mut command = crate::tools::process::command(program);
    command
        .args(parsed_args)
        .current_dir(cwd)
        ;

    let output = match crate::tools::process::output(command, timeout_secs, "コマンド").await {
        Ok(output) => output,
        Err(error) => return format!("エラー: {error}"),
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

inventory::submit! { ToolReg { make: || Box::new(RunCommandTool) } }

#[cfg(test)]
mod tests {
    use super::{allowed, bounded_cwd, command_tokens};
    use crate::config::Config;
    use crate::tools::process::is_physical_delete;

    #[test]
    fn コマンドトークンを完全一致比較用に組み立てる() {
        assert_eq!(
            command_tokens("git", &["status".to_string(), "--short".to_string()]),
            vec!["git", "status", "--short"]
        );
    }

    #[test]
    fn 許可コマンドへ後続引数を足しても通らない() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_COMMAND_ALLOWLIST", "git diff,cargo test");
        assert!(allowed(&cfg, "git", &["diff".to_string()]));
        assert!(!allowed(
            &cfg,
            "git",
            &[
                "diff".to_string(),
                "--no-index".to_string(),
                "C:\\secret.txt".to_string(),
            ]
        ));
        assert!(!allowed(
            &cfg,
            "cargo",
            &[
                "test".to_string(),
                "--manifest-path".to_string(),
                "..\\outside\\Cargo.toml".to_string(),
            ]
        ));
    }

    #[test]
    fn 実行cwdのルート外を拒否する_スラッシュ区切り() {
        let base = std::env::temp_dir().join(format!(
            "picoagent-cmd-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let cfg = Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", base.to_string_lossy().as_ref());
        assert!(bounded_cwd(&cfg, Some("../other")).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn 物理削除コマンドを拒否判定する() {
        assert!(is_physical_delete("rm", &["target.txt".to_string()]));
        assert!(is_physical_delete(
            "powershell",
            &["-Command".to_string(), "Remove-Item target.txt".to_string()]
        ));
        assert!(is_physical_delete(
            "cmd",
            &["/C".to_string(), "del target.txt".to_string()]
        ));
        assert!(is_physical_delete("git", &["clean".to_string(), "-fd".to_string()]));
        assert!(is_physical_delete(
            "cmd",
            &[
                "/C".to_string(),
                "powershell -Command Remove-Item target.txt".to_string(),
            ]
        ));
        assert!(!is_physical_delete("git", &["status".to_string()]));
    }

    /// 対応OSのもう一方（macOS）のシェルが許可コマンドに入っていると、
    /// PowerShell と同じ形で物理削除を通せてしまっていた。
    #[test]
    fn posixシェル経由の物理削除も拒否する() {
        for shell in ["sh", "bash", "zsh", "/bin/bash"] {
            assert!(
                is_physical_delete(shell, &["-c".to_string(), "rm -rf ~/Documents".to_string()]),
                "{shell} 経由の rm を拒否していない"
            );
        }
        assert!(is_physical_delete(
            "bash",
            &["-c".to_string(), "find . -name '*.tmp' -delete".to_string()]
        ));
        assert!(is_physical_delete("shred", &["secret.txt".to_string()]));
        // 削除でないシェル実行までは止めない。POSIX に無い cmd 語彙（del / erase / rd）を
        // 巻き込むと、無関係なファイル名やサブコマンドまで拒否してしまう。
        assert!(!is_physical_delete(
            "bash",
            &["-c".to_string(), "cargo test --release".to_string()]
        ));
        assert!(!is_physical_delete(
            "bash",
            &["-c".to_string(), "python del.py && ls erase/".to_string()]
        ));
    }
}
