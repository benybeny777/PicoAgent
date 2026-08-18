// skills/code_runner/mod.rs - 他スキルでは要望を満たせないときの最終手段。
//
// モデルが PowerShell または Python のスクリプトを書いて実行し、結果を見て自力で
// タスクを達成するためのフォールバック実行スキル。任意コード実行のため既定は無効で、
// PICOAGENT_CODE_RUNNER_ENABLED=true のときだけ動く。作業ディレクトリと許可判定は
// 全スキル共通の permissions（許可ルート・保護パス）を参照する。物理削除系のコードは拒否する。

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::permissions;
use crate::tools::run_command::trim_output;

const MAX_CODE_CHARS: usize = 100_000;
const DEFAULT_TIMEOUT_SECS: u64 = 60;

pub struct CodeRunnerTool;

/// スクリプト本文に物理削除・破壊的操作らしきパターンが含まれるか（best-effort）。
/// 含まれる場合はその名称を返し、実行を拒否する（ファイル削除はゴミ箱スキル経由に限る方針）。
fn destructive_op(code: &str) -> Option<&'static str> {
    let lower = code.to_lowercase();
    const PATTERNS: &[&str] = &[
        // PowerShell / シェル系
        "remove-item",
        "clear-content",
        "[io.file]::delete",
        "[io.directory]::delete",
        "[system.io.file]::delete",
        "[system.io.directory]::delete",
        "format-volume",
        "diskpart",
        "rmdir",
        // Python
        "os.remove",
        "os.unlink",
        "os.rmdir",
        "shutil.rmtree",
        ".unlink(",
        ".rmdir(",
    ];
    if let Some(found) = PATTERNS.iter().find(|pattern| lower.contains(**pattern)) {
        return Some(found);
    }
    // 単語境界が必要な短いトークン（rm / del / rd / erase）は誤検知を避けて別途判定する。
    for token in ["rm", "del", "rd", "erase"] {
        if word_present(&lower, token) {
            return Some(match token {
                "rm" => "rm",
                "del" => "del",
                "rd" => "rd",
                _ => "erase",
            });
        }
    }
    None
}

fn word_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .any(|part| part == token)
}

fn temp_script_path(ext: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("picoagent_code_{nanos}.{ext}"))
}

#[async_trait]
impl Tool for CodeRunnerTool {
    fn name(&self) -> &'static str {
        "code_runner"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["スクリプト", "コードを書いて", "コード実行", "プログラムで", "自力で", "なんとかして", "script", "code"]
    }

    fn label(&self) -> &'static str {
        "自力実行（コード実行）"
    }

    // OSレベルのサンドボックスが無い任意コードをモデルへ公開すると、cwd の検査だけでは
    // 許可ルート外の読書きや難読化した削除を防げない。安全な隔離実行を用意するまで
    // エージェントのツール一覧には出さない。
    fn agent_callable(&self) -> bool {
        false
    }

    fn description(&self) -> String {
        "安全な隔離実行を準備中のため、現在はエージェントから呼び出せない内部機能。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "language": {
                    "type": "string",
                    "enum": ["powershell", "python"],
                    "description": "スクリプトの言語。"
                },
                "code": {
                    "type": "string",
                    "description": "実行するスクリプト本文。結果は標準出力へ print/Write-Output する。"
                },
                "cwd": {
                    "type": "string",
                    "description": "作業ディレクトリ。許可ルート配下のみ。省略時は許可ルート直下。"
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 300,
                    "description": "実行待ち上限秒。省略時は設定値。"
                }
            },
            "required": ["language", "code"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "既存スキルで無理な集計をスクリプトでやって",
            "このフォルダのファイル名を一括で規則変更して",
        ]
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_CODE_RUNNER_ENABLED",
                "自力実行（コード実行）を有効化（true/false）",
                "text",
                "false",
            ),
            Field::new(
                "PICOAGENT_CODE_RUNNER_PYTHON",
                "Python 実行ファイル（省略時は python）",
                "text",
                "python",
            ),
            Field::new(
                "PICOAGENT_CODE_RUNNER_TIMEOUT_SECS",
                "自力実行の既定タイムアウト秒（1〜300）",
                "text",
                "60",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        if !cfg.get_bool("PICOAGENT_CODE_RUNNER_ENABLED", false) {
            return "エラー: 自力実行（コード実行）は無効です。設定画面の「許可（ファイル/実行）」タブで PICOAGENT_CODE_RUNNER_ENABLED を true にしてください。任意コードを実行するため、内容を理解したうえで有効化してください。".to_string();
        }
        let language = args
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let code = args.get("code").and_then(Value::as_str).unwrap_or("");
        if code.trim().is_empty() {
            return "エラー: code が空です。実行するスクリプト本文を指定してください。".to_string();
        }
        if code.chars().count() > MAX_CODE_CHARS {
            return format!("エラー: code が長すぎます（{MAX_CODE_CHARS}文字以内）。");
        }
        if let Some(op) = destructive_op(code) {
            return format!(
                "エラー: 物理削除・破壊的操作らしきコード（{op}）は実行できません。ファイル削除はゴミ箱移動スキル（trash_file）を使ってください。"
            );
        }

        // 作業ディレクトリを共通の許可ルールで検証する。
        let requested_cwd = args.get("cwd").and_then(Value::as_str).unwrap_or(".");
        let cwd = match permissions::resolve(cfg, requested_cwd) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        };

        let default_timeout = cfg
            .get("PICOAGENT_CODE_RUNNER_TIMEOUT_SECS", "60")
            .parse::<u64>()
            .ok()
            .filter(|value| (1..=300).contains(value))
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .filter(|value| (1..=300).contains(value))
            .unwrap_or(default_timeout);

        // 言語ごとにインタプリタと拡張子を決める。
        let (program, ext, mut interpreter_args): (String, &str, Vec<String>) = match language.as_str()
        {
            "python" => (
                cfg.get("PICOAGENT_CODE_RUNNER_PYTHON", "python"),
                "py",
                Vec::new(),
            ),
            "powershell" => {
                let program = if cfg!(windows) { "powershell" } else { "pwsh" };
                (
                    program.to_string(),
                    "ps1",
                    vec![
                        "-NoProfile".to_string(),
                        "-NonInteractive".to_string(),
                        "-ExecutionPolicy".to_string(),
                        "Bypass".to_string(),
                        "-File".to_string(),
                    ],
                )
            }
            other => {
                return format!(
                    "エラー: 未対応の language です: {other}（powershell か python を指定してください）"
                )
            }
        };

        // スクリプトを一時ファイルへ書き出す（内部一時ファイルなので実行後に削除する）。
        let script_path = temp_script_path(ext);
        if let Err(error) = std::fs::write(&script_path, code) {
            return format!("エラー: スクリプトの書き出しに失敗しました（{error}）");
        }
        interpreter_args.push(script_path.to_string_lossy().to_string());

        let mut command = crate::tools::process::command(&program);
        command
            .args(&interpreter_args)
            .current_dir(&cwd)
            ;

        let mut result =
            match tokio::time::timeout(Duration::from_secs(timeout_secs), command.output()).await {
                Ok(Ok(output)) => {
                    let mut out = format!(
                        "終了コード: {}\n言語: {}\n作業ディレクトリ: {}\n",
                        output
                            .status
                            .code()
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "シグナル終了".to_string()),
                        language,
                        cwd.display()
                    );
                    out.push_str(&trim_output("stdout", &output.stdout));
                    out.push('\n');
                    out.push_str(&trim_output("stderr", &output.stderr));
                    out
                }
                Ok(Err(error)) => format!(
                    "エラー: インタプリタ「{program}」の起動に失敗しました（{error}）。Python の場合は PICOAGENT_CODE_RUNNER_PYTHON に実行パスを設定してください。"
                ),
                Err(_) => format!("エラー: 実行が {timeout_secs} 秒でタイムアウトしました。"),
            };

        // 一時スクリプトを後片付けする（内部一時ファイルの掃除）。
        if let Err(error) = std::fs::remove_file(&script_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                result.push_str(&format!(
                    "\n警告: 一時スクリプトを削除できませんでした: {error}"
                ));
            }
        }
        result
    }
}

inventory::submit! { ToolReg { make: || Box::new(CodeRunnerTool) } }

#[cfg(test)]
mod tests {
    use super::destructive_op;

    #[test]
    fn 破壊的コードを検出する() {
        assert!(destructive_op("Remove-Item foo.txt").is_some());
        assert!(destructive_op("import shutil; shutil.rmtree('x')").is_some());
        assert!(destructive_op("os.remove('a')").is_some());
        assert!(destructive_op("rm -rf build").is_some());
        // 通常の処理は許可。
        assert!(destructive_op("print(sum(range(10)))").is_none());
        assert!(destructive_op("Write-Output (1+2)").is_none());
    }
}
