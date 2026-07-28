// tools/process.rs - 外部プロセス実行の共通層。
//
// スキルが外部コマンドを起動するときの作法（Windows でコンソール窓を出さない、
// 中断時に子プロセスを残さない、必ずタイムアウトを取る）を1箇所へ集約する。
//
// これらは以前スキルごとに `#[cfg(windows)] fn hide_window` をコピーする運用で、
// 7ファイルに同じ実装が散らばり、4スキル（clipboard_text / doc_search /
// whisper_transcribe / meeting_recorder）では丸ごと抜けていた。
// 抜けると Windows でコマンド実行のたびに黒いコンソール窓が明滅し、
// 中断時には子プロセスが残る。`tools::process::guard` のテストが直接 `Command::new` を
// 使っているスキルを検出して失敗させる。

use std::ffi::OsStr;
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::process::Command;

/// タイムアウトの上限（どんな設定値でもこれを超えて待たない）。
const MAX_TIMEOUT_SECS: u64 = 7200;

/// 物理削除コマンドかどうか。
///
/// スキル経由のファイル削除は必ず OS のゴミ箱へ移す決まりで、`rm` / `del` /
/// `Remove-Item` / `git clean` などの物理削除は実行させない（[AGENTS.md] のルール）。
/// この判定は**ユーザー / AI が指定したプログラムを実行するすべての経路**で使う。
/// 以前は run_command だけが持っていて、同じ allowlist で動くスケジューラには無く、
/// 許可コマンドに `powershell` があれば予約実行から `Remove-Item` を通せてしまった。
pub fn is_physical_delete(program: &str, args: &[String]) -> bool {
    let program = program_stem(program);
    match program.as_str() {
        "rm" | "del" | "erase" | "rmdir" | "rd" | "unlink" => true,
        "git" => args
            .first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case("clean")),
        "powershell" | "pwsh" => args.iter().any(|arg| contains_delete_token(arg, true)),
        "cmd" => args.iter().any(|arg| contains_delete_token(arg, false)),
        _ => false,
    }
}

/// 物理削除を断るときの共通メッセージ。
pub fn physical_delete_error() -> String {
    "エラー: スキル経由のファイル削除は物理削除ではなくゴミ箱へ移動する必要があります。rm / del / Remove-Item / git clean などの物理削除コマンドは実行できません。"
        .to_string()
}

/// パス・拡張子を落としたプログラム名（`C:\\Windows\\System32\\cmd.exe` → `cmd`）。
fn program_stem(program: &str) -> String {
    let trimmed = program.trim().trim_matches('"');
    let file = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    file.rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file)
        .to_ascii_lowercase()
}

/// 単語境界で削除コマンドを含むか（`-Recurse` などの中に埋もれた一致は拾わない）。
fn contains_delete_token(arg: &str, powershell: bool) -> bool {
    let lower = arg.to_ascii_lowercase();
    let mut tokens = vec!["del", "erase", "rmdir", "rd"];
    if powershell {
        tokens.extend_from_slice(&["remove-item", "rm"]);
    }
    lower
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .any(|part| tokens.iter().any(|token| part.eq_ignore_ascii_case(token)))
}

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

/// 同期版（`std::process::Command`）に安全な既定を適用して作る。
///
/// 常駐プロセス（同梱エンジン・AivisSpeech）のように「future の終了後も動き続けてほしい」
/// ものや、`spawn()` して子ハンドルを保持するものはこちらを使う。stdio と待ち方は
/// 用途ごとに異なるので触らず、**Windows のコンソール窓抑止だけ**を共通化する
/// （ここが各所へコピーされていた `hide_window` の中身）。
pub fn command_sync(program: impl AsRef<OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    apply_no_window_sync(&mut command);
    command
}

#[cfg(windows)]
fn apply_no_window_sync(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_no_window_sync(_command: &mut std::process::Command) {}

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
            assert!(
                started.elapsed().as_secs() < 5,
                "経過: {:?}",
                started.elapsed()
            );
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

    /// 直接 `Command::new` を許すスキル。現在は例外なし。
    /// 追加する場合は、なぜ共通層に載せられないのかを理由として書き添えること。
    const RAW_COMMAND_ALLOWED: [&str; 0] = [];

    use crate::guard_scan;

    /// 外部プロセスの起動は `tools::process::command()` 経由に統一すること。
    #[test]
    fn プロセス起動作法を自前実装しない() {
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            let name = &source.name;
            if name == "process.rs" {
                continue; // 共通層の実装そのもの
            }
            if RAW_COMMAND_ALLOWED.contains(&name.as_str()) {
                continue;
            }
            let body = source.body();
            if body.contains("fn hide_window") || body.contains("CREATE_NO_WINDOW") {
                violations.push(format!("{name}: hide_window/CREATE_NO_WINDOW の自前実装"));
            }
            // `Command::new(` の直接呼び出し（process::command 経由でないもの）。
            for line in body.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("Command::new(")
                    && !line.contains("process::command(")
                    && !line.contains("process::command_sync(")
                {
                    violations.push(format!("{name}: `{}`", trimmed.trim()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "外部プロセスを直接起動しています。`tools::process::command()` を使ってください\n（Windows のコンソール窓抑止・中断時の子プロセス回収が漏れます）:\n{}",
            violations.join("\n")
        );
    }

    /// `PICOAGENT_COMMAND_ALLOWLIST` でユーザー / AI 指定のコマンドを実行する経路は、
    /// 物理削除の拒否（`process::is_physical_delete`）も必ず通すこと。
    ///
    /// run_command だけがこの判定を持っていて、同じ allowlist で動くスケジューラには
    /// 無かった。許可コマンドに `powershell` があれば、予約実行から `Remove-Item` を
    /// 通せてしまう状態だった。
    #[test]
    fn 許可コマンド実行経路は物理削除を拒否する() {
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "process.rs" {
                continue; // 判定の定義元
            }
            let body = source.body();
            if body.contains("PICOAGENT_COMMAND_ALLOWLIST") && !body.contains("is_physical_delete")
            {
                violations.push(source.path.display().to_string());
            }
        }
        assert!(
            violations.is_empty(),
            "許可コマンドを実行する経路が物理削除を拒否していません。`process::is_physical_delete()` を通してください: {violations:?}"
        );
    }
}
