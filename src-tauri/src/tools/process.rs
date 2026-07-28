// tools/process.rs - 外部プロセス実行の共通層。
//
// スキルが外部コマンドを起動するときの作法（Windows でコンソール窓を出さない、
// 中断時に子プロセスを残さない、必ずタイムアウトを取る）を1箇所へ集約する。
//
// これらは以前スキルごとに `#[cfg(windows)] fn hide_window` をコピーする運用で、
// 7ファイルに同じ実装が散らばり、4スキル（clipboard_text / doc_search /
// whisper_transcribe / meeting_recorder）では丸ごと抜けていた。
// 抜けると Windows でコマンド実行のたびに黒いコンソール窓が明滅し、
// 中断時には子プロセスが残る。`tools::guard` のテストが直接 `Command::new` を
// 使っているスキルを検出して失敗させる。

use std::ffi::OsStr;
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::process::Command;

/// タイムアウトの上限（どんな設定値でもこれを超えて待たない）。
const MAX_TIMEOUT_SECS: u64 = 7200;

/// 安全な既定を適用した `Command` を作る。
///
/// - Windows: `CREATE_NO_WINDOW`（コンソール窓を出さない）
/// - stdin を閉じ、stdout/stderr を捕捉する
/// - `kill_on_drop`: 中断・タイムアウトで future が落ちても子プロセスを残さない
///
/// 引数の追加は返り値へ通常どおり `.arg()` / `.args()` する。
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    apply_no_window(&mut command);
    command
}

#[cfg(windows)]
fn apply_no_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_no_window(_command: &mut Command) {}

/// `command()` で作ったコマンドをタイムアウト付きで実行する。
/// `label` は失敗時のメッセージに使う日本語の処理名（例: 「画像変換」）。
pub async fn output(
    mut command: Command,
    timeout_secs: u64,
    label: &str,
) -> Result<Output, String> {
    let timeout = timeout_secs.clamp(1, MAX_TIMEOUT_SECS);
    match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(format!("{label}を起動できません: {error}")),
        // kill_on_drop により、ここで打ち切られた子プロセスは確実に停止される。
        Err(_) => Err(format!("{label}が {timeout} 秒でタイムアウトしました。")),
    }
}

/// 実行して標準出力を文字列で返す（失敗時は stderr を含むエラー文字列）。
/// 出力の中身より成否が主な用途（クリップボード取得など）向け。
pub async fn output_text(
    command: Command,
    timeout_secs: u64,
    label: &str,
) -> Result<String, String> {
    let result = output(command, timeout_secs, label).await?;
    if !result.status.success() {
        return Err(format!(
            "{label}が失敗しました: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::{command, output, output_text};

    #[test]
    fn 既定設定で実行し出力を取れる() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut cmd = command(if cfg!(windows) { "cmd" } else { "echo" });
            if cfg!(windows) {
                cmd.args(["/C", "echo picoagent"]);
            } else {
                cmd.arg("picoagent");
            }
            let text = output_text(cmd, 30, "テスト実行").await.unwrap();
            assert!(text.contains("picoagent"), "text: {text}");
        });
    }

    #[test]
    fn 存在しないコマンドは起動エラーになる() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let cmd = command("picoagent-not-a-real-command");
            let error = output(cmd, 10, "存在しない実行").await.unwrap_err();
            assert!(error.contains("起動できません"), "error: {error}");
        });
    }

    // タイムアウトで打ち切っても、kill_on_drop により子プロセスが残らないこと。
    #[cfg(unix)]
    #[test]
    fn タイムアウトで子プロセスを残さない() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut cmd = command("sh");
            cmd.args(["-c", "sleep 30"]);
            let started = std::time::Instant::now();
            let error = output(cmd, 1, "長い処理").await.unwrap_err();
            assert!(error.contains("タイムアウト"), "error: {error}");
            // 30秒待たずに戻る＝待ち続けていない。
            assert!(started.elapsed().as_secs() < 5, "経過: {:?}", started.elapsed());
        });
    }
}

/// 設計ガード: スキルが外部プロセスの起動作法を自前で持ち込んでいないかを検査する。
///
/// 以前は `#[cfg(windows)] fn hide_window` を各スキルへコピーする運用で、7ファイルに
/// 同じ実装が散らばり4スキルで丸ごと抜けていた（Windows でコンソール窓が明滅し、
/// 中断時に子プロセスが残る）。人のレビューに頼らずテストで検出する。
#[cfg(test)]
mod guard {
    use std::path::{Path, PathBuf};

    /// 直接 `Command::new` を許すスキル（意図的な例外のみ。追加には理由を添えること）。
    /// - `meeting_recorder`: 録音は自前スレッドで長時間動かすため、別途プロセス制御を持つ
    const RAW_COMMAND_ALLOWED: [&str; 0] = [];

    fn skill_sources() -> Vec<(String, String)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri の親")
            .join("skills");
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).expect("skills ディレクトリ").flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path: PathBuf = entry.path().join("mod.rs");
            if let Ok(text) = std::fs::read_to_string(&path) {
                out.push((name, text));
            }
        }
        assert!(out.len() > 40, "スキルの読み込みに失敗している可能性: {}", out.len());
        out
    }

    /// テストコード部分（`#[cfg(test)]` 以降）を落とす。テストの外部コマンド有無判定は対象外。
    fn without_tests(text: &str) -> &str {
        match text.find("#[cfg(test)]") {
            Some(pos) => &text[..pos],
            None => text,
        }
    }

    /// スキルは `tools::process::command()` 経由でプロセスを起動すること。
    #[test]
    fn スキルはプロセス起動作法を自前実装しない() {
        let mut violations = Vec::new();
        for (name, text) in skill_sources() {
            if RAW_COMMAND_ALLOWED.contains(&name.as_str()) {
                continue;
            }
            let body = without_tests(&text);
            if body.contains("fn hide_window") || body.contains("CREATE_NO_WINDOW") {
                violations.push(format!("{name}: hide_window/CREATE_NO_WINDOW の自前実装"));
            }
            // `Command::new(` の直接呼び出し（process::command 経由でないもの）。
            for line in body.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("Command::new(") && !line.contains("process::command(") {
                    violations.push(format!("{name}: `{}`", trimmed.trim()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "スキルが外部プロセスを直接起動しています。`tools::process::command()` を使ってください\n（Windows のコンソール窓抑止・中断時の子プロセス回収が漏れます）:\n{}",
            violations.join("\n")
        );
    }
}
