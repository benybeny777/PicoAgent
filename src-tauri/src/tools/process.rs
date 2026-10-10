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

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

/// タイムアウトの上限（どんな設定値でもこれを超えて待たない）。
const MAX_TIMEOUT_SECS: u64 = 7200;
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

/// 物理削除コマンドかどうか。
///
/// スキル経由のファイル削除は必ず OS のゴミ箱へ移す決まりで、`rm` / `del` /
/// `Remove-Item` / `git clean` などの物理削除は実行させない（[AGENTS.md] のルール）。
/// この判定は**ユーザー / AI が指定したプログラムを実行するすべての経路**で使う。
/// 以前は run_command だけが持っていて、同じ allowlist で動くスケジューラには無く、
/// 許可コマンドに `powershell` があれば予約実行から `Remove-Item` を通せてしまった。
///
/// **シェル本体が許可コマンドになっている場合は、その引数の中まで見る。** Windows の
/// `powershell` / `cmd` だけを見ていた頃は、対応OSのもう一方である macOS の
/// `sh -c "rm -rf ~/Documents"` が素通りした（同じ形の穴が OS 違いで残っていた）。
pub fn is_physical_delete(program: &str, args: &[String]) -> bool {
    let program = program_stem(program);
    // シェルを入れ子にした場合も、どのシェルの削除語彙も見落とさない。
    // allowlist にシェル本体を追加した利用者設定でも物理削除禁止を保つ。
    let shell_family = match program.as_str() {
        "powershell" | "pwsh" => Some(ShellFamily::PowerShell),
        "cmd" => Some(ShellFamily::Cmd),
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" => Some(ShellFamily::Posix),
        _ => None,
    };
    match program.as_str() {
        "rm" | "del" | "erase" | "rmdir" | "rd" | "unlink" | "shred" => true,
        "git" => args
            .first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case("clean")),
        "powershell" | "pwsh" | "cmd" | "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" => {
            args.iter().any(|arg| {
                shell_family.is_some_and(|family| contains_delete_token(arg, family))
                    || (contains_shell_name(arg)
                        && [
                            ShellFamily::PowerShell,
                            ShellFamily::Cmd,
                            ShellFamily::Posix,
                        ]
                        .into_iter()
                        .any(|family| contains_delete_token(arg, family)))
            })
        }
        _ => false,
    }
}

fn contains_shell_name(arg: &str) -> bool {
    arg.to_ascii_lowercase()
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .any(|part| {
            matches!(
                part,
                "powershell" | "pwsh" | "cmd" | "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish"
            )
        })
}

/// 削除語彙はシェルごとに違う（`Remove-Item` は PowerShell だけ、`unlink` は POSIX だけ）。
#[derive(Clone, Copy)]
enum ShellFamily {
    Cmd,
    PowerShell,
    Posix,
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
///
/// 語彙はシェルごとに**別**にする。共通の土台に `del` / `erase` / `rd` を置くと、
/// それらを削除語として持たない POSIX シェルで `bash -c "python del.py"` のような
/// 無関係な語まで拒否してしまう（実行できないほうへ倒れるので気づきにくい）。
fn contains_delete_token(arg: &str, shell: ShellFamily) -> bool {
    let tokens: &[&str] = match shell {
        ShellFamily::Cmd => &["del", "erase", "rmdir", "rd"],
        // `rm` / `ri` は Remove-Item の別名。cmd の内部コマンドも解釈できる。
        ShellFamily::PowerShell => &[
            "remove-item",
            "rm",
            "ri",
            "del",
            "erase",
            "rmdir",
            "rd",
            "unlink",
        ],
        // `-delete` は `find . -delete` の形を拾う（分割時に `-` を残すので語として一致する）。
        ShellFamily::Posix => &["rm", "rmdir", "unlink", "shred", "-delete"],
    };
    arg.to_ascii_lowercase()
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
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_no_window(_command: &mut Command) {}

/// macOS の GUI アプリが継承しない、Homebrew の既定設置先。
///
/// Finder / Dock から起動したアプリの PATH は `launchd` の素の値で、`/opt/homebrew/bin` を
/// 含まない。ターミナルから起動したときだけ動く、という食い違いの原因になるためここも見る。
#[cfg(target_os = "macos")]
const EXTRA_EXECUTABLE_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];
#[cfg(not(target_os = "macos"))]
const EXTRA_EXECUTABLE_DIRS: &[&str] = &[];

/// 外部コマンドの実行ファイルを絶対パスで解決する。見つからなければ `None`。
///
/// **存在確認を `sh -lc "command -v ..."`（ログインシェル）で行ってはいけない。**
/// ログインシェルは `/etc/paths.d` から PATH を組み直すため「ある」と判定されるのに、
/// 実際に spawn する側は GUI アプリが継承した素の PATH しか見ない。この食い違いで、
/// p7zip を導入済みの macOS でも Aivis の分割書庫展開が実行時に
/// `No such file or directory` で失敗していた。判定と実行を同じ探索で揃えるため、
/// 存在確認も起動も**この関数が返した絶対パス**を使う。
pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    resolve_executable_in(
        name,
        std::env::var_os("PATH").as_deref(),
        EXTRA_EXECUTABLE_DIRS,
    )
}

/// 実行ファイルが見つかるかどうかだけを見る。
pub fn has_executable(name: &str) -> bool {
    resolve_executable(name).is_some()
}

/// 探索先を明示する本体（テストから実ディレクトリを渡せるように分けている）。
fn resolve_executable_in(
    name: &str,
    path_env: Option<&OsStr>,
    extra_dirs: &[&str],
) -> Option<PathBuf> {
    let names = executable_candidates(name);
    let from_path = path_env
        .into_iter()
        .flat_map(|paths| std::env::split_paths(paths).collect::<Vec<_>>());
    let from_extra = extra_dirs.iter().map(PathBuf::from);
    for dir in from_path.chain(from_extra) {
        for candidate in &names {
            let path = dir.join(candidate);
            if is_executable_file(&path) {
                return std::path::absolute(path).ok();
            }
        }
    }
    None
}

pub(super) fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// 探すファイル名の候補。Windows は `where.exe` と同じく PATHEXT の拡張子まで見る
/// （`pandoc.cmd` のようなラッパーを取りこぼさない）。拡張子付きで渡された名前はそのまま。
fn executable_candidates(name: &str) -> Vec<OsString> {
    if !cfg!(windows) || Path::new(name).extension().is_some() {
        return vec![OsString::from(name)];
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut candidates: Vec<OsString> = pathext
        .split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| OsString::from(format!("{name}{ext}")))
        .collect();
    if candidates.is_empty() {
        candidates.push(OsString::from(format!("{name}.exe")));
    }
    candidates
}

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

/// `command_sync()` で spawn した常駐プロセスを、その配下ごと停止して回収する。
///
/// Windows の `Child::kill()` は**直下の子だけ**を止める。ブラウザのように自分で
/// 別プロセスを立てて役割を分けるものは、親を落としても実体（描画・GPU・ネットワーク）が
/// 生き残り、握ったポートやプロファイルもそのまま占有され続ける。実際に
/// `browser_control` の headless Chrome が9プロセス残り、以後の起動が
/// 「PicoAgent が管理していないブラウザに使われています」で必ず失敗していた。
/// 非同期側の `terminate_process_tree()` と同じ考え方の同期版。
pub fn terminate_tree_sync(child: &mut std::process::Child, label: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut killer = command_sync("taskkill.exe");
        killer.args(["/PID", &child.id().to_string(), "/T", "/F"]);
        let result = killer
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("{label}のプロセスツリーを停止できません: {error}"))?;
        // taskkill は「既に終了している」場合も失敗を返す。実際に残っているときだけ errorにする。
        if !result.success()
            && child
                .try_wait()
                .map_err(|error| format!("{label}の終了状態を確認できません: {error}"))?
                .is_none()
        {
            return Err(format!("{label}のプロセスツリーを停止できませんでした"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Err(error) = child.kill() {
            if error.kind() != std::io::ErrorKind::InvalidInput {
                return Err(format!("{label}を停止できません: {error}"));
            }
        }
    }
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("{label}の終了ハンドルを回収できません: {error}"))
}

/// `command()` で作ったコマンドをタイムアウト付きで実行する。
/// `label` は失敗時のメッセージに使う日本語の処理名（例: 「画像変換」）。
/// 実行が失敗した理由。**起動できなかった場合だけ**呼び出し側が導入案内を足せるよう区別する。
/// タイムアウトにも同じ案内を付けると、入っているのに「導入してください」と言うことになる。
enum RunFailure {
    /// 実行ファイルを起動できなかった（未導入・パス誤りなど）。
    NotStarted(String),
    /// 起動はしたが、上限時間・終了待ち・出力回収のいずれかで失敗した。
    Interrupted(String),
}

impl RunFailure {
    fn into_message(self) -> String {
        match self {
            Self::NotStarted(message) | Self::Interrupted(message) => message,
        }
    }
}

/// 未導入がありうる外部CLI（LibreOffice・ImageMagick など）を実行する。
/// 起動できなかったときだけ `hint` を続けて返す。
pub async fn output_or_install_hint(
    command: Command,
    timeout_secs: u64,
    label: &str,
    hint: &str,
) -> Result<Output, String> {
    run(command, timeout_secs, label)
        .await
        .map_err(|failure| match failure {
            RunFailure::NotStarted(message) => format!("{message}\n{hint}"),
            RunFailure::Interrupted(message) => message,
        })
}

pub async fn output(command: Command, timeout_secs: u64, label: &str) -> Result<Output, String> {
    run(command, timeout_secs, label)
        .await
        .map_err(RunFailure::into_message)
}

async fn run(command: Command, timeout_secs: u64, label: &str) -> Result<Output, RunFailure> {
    // 呼び出し元が停止要求との `select!` でこのfutureを破棄しても、実行本体は専用タスクに
    // 残して中断通知を受け取らせる。`kill_on_drop` だけでは直下のラッパーしか止まらず、
    // Codex／Claude CLIなどが起動した孫プロセスを残すため、必ず共通のツリー回収へ通す。
    let (abandon_tx, abandon_rx) = tokio::sync::oneshot::channel();
    let label_owned = label.to_string();
    let worker = tokio::spawn(async move {
        output_worker(command, timeout_secs, &label_owned, abandon_rx).await
    });
    let mut abandon = AbandonOutput(Some(abandon_tx));
    let result = worker.await.map_err(|error| {
        RunFailure::Interrupted(format!("{label}の実行タスクを回収できません: {error}"))
    })?;
    abandon.0.take();
    result
}

/// `output()` の呼び出しfutureが破棄されたことを、子プロセスを所有する実行タスクへ伝える。
struct AbandonOutput(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for AbandonOutput {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

async fn output_worker(
    mut command: Command,
    timeout_secs: u64,
    label: &str,
    mut abandon: tokio::sync::oneshot::Receiver<()>,
) -> Result<Output, RunFailure> {
    let timeout = timeout_secs.clamp(1, MAX_TIMEOUT_SECS);
    let mut child = command
        .spawn()
        .map_err(|error| RunFailure::NotStarted(format!("{label}を起動できません: {error}")))?;
    let stdout_task = tokio::spawn(read_capture(child.stdout.take()));
    let stderr_task = tokio::spawn(read_capture(child.stderr.take()));
    let waited = tokio::select! {
        biased;
        _ = &mut abandon => {
            let cleanup_error = terminate_process_tree(&mut child, label).await.err();
            stdout_task.abort();
            stderr_task.abort();
            return Err(RunFailure::Interrupted(match cleanup_error {
                Some(error) => format!("{label}の中断後に子プロセスを回収できません: {error}"),
                None => format!("{label}を中断しました。"),
            }));
        }
        waited = tokio::time::timeout(Duration::from_secs(timeout), child.wait()) => waited,
    };
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return Err(RunFailure::Interrupted(format!(
                "{label}の終了を待てません: {error}"
            )))
        }
        Err(_) => {
            let cleanup_error = terminate_process_tree(&mut child, label).await.err();
            // 出力の回収を待たずに打ち切る。`sh -c` / `powershell -Command` のように
            // 子がさらにプロセスを産む形では、子を kill しても孫がパイプの書き込み端を
            // 握ったままで終端（EOF）が来ない。ここで待つと孫が終わるまで戻れず、
            // タイムアウトを設けた意味がなくなる（実際に1秒指定が30秒待ちになっていた）。
            stdout_task.abort();
            stderr_task.abort();
            return Err(RunFailure::Interrupted(match cleanup_error {
                Some(error) => format!(
                    "{label}が {timeout} 秒でタイムアウトし、子プロセスの回収にも失敗しました: {error}"
                ),
                None => format!("{label}が {timeout} 秒でタイムアウトしました。"),
            }));
        }
    };
    // 子が正常終了しても、孫がパイプを握っていれば同じ理由で終端が来ない。
    // 終了後の回収にも上限を置き、待ち続けずエラーとして返す。
    let stdout = collect(stdout_task, label, "標準出力")
        .await
        .map_err(RunFailure::Interrupted)?;
    let stderr = collect(stderr_task, label, "標準エラー")
        .await
        .map_err(RunFailure::Interrupted)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// タイムアウトしたプロセスと、その配下で起動した孫プロセスを回収する。
///
/// Windows の `Child::kill()` は直下の子だけを止める。`cmd /c` の配下に残った孫が
/// stdout/stderr のパイプを握ると、呼び出し元へ戻った後も Tokio ランタイムの終了が
/// 孫の寿命まで待たされるため、`taskkill /T` でツリーごと止める。
#[cfg(windows)]
pub async fn terminate_process_tree(child: &mut Child, label: &str) -> Result<(), String> {
    let pid = child
        .id()
        .ok_or_else(|| format!("{label}のプロセスIDを取得できません"))?;
    let mut killer = command("taskkill.exe");
    killer.args(["/PID", &pid.to_string(), "/T", "/F"]);
    let result = tokio::time::timeout(Duration::from_secs(3), killer.output())
        .await
        .map_err(|_| format!("{label}のプロセスツリー停止が3秒でタイムアウトしました"))?
        .map_err(|error| format!("{label}のプロセスツリーを停止できません: {error}"))?;
    if !result.status.success()
        && child
            .try_wait()
            .map_err(|error| format!("{label}の終了状態を確認できません: {error}"))?
            .is_none()
    {
        return Err(format!(
            "taskkillが失敗しました: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .map_err(|_| format!("{label}の停止完了待ちが3秒でタイムアウトしました"))?
        .map_err(|error| format!("{label}の停止完了を待てません: {error}"))?;
    Ok(())
}

#[cfg(not(windows))]
pub async fn terminate_process_tree(child: &mut Child, label: &str) -> Result<(), String> {
    child
        .kill()
        .await
        .map_err(|error| format!("{label}を停止できません: {error}"))?;
    child
        .wait()
        .await
        .map_err(|error| format!("{label}の停止完了を待てません: {error}"))?;
    Ok(())
}

/// 子プロセス終了後に出力の回収を待つ上限。
/// 終端が来ないのは孫プロセスがパイプを握っている場合だけなので、短くてよい。
pub const CAPTURE_DRAIN_SECS: u64 = 5;

/// 出力を読むタスクを上限付きで待ち、上限を超えたら中断して `None` を返す。
///
/// 子を停止しても、`sh -c` / `powershell -Command` が産んだ孫や、子がさらに起動した
/// プロセスがパイプの書き込み端を握っていると終端（EOF）が来ない。ここで待ち切ると
/// タイムアウトも中断も効かなくなるため、待ち方を共通化する。
pub async fn join_capture<T>(
    mut task: tokio::task::JoinHandle<T>,
) -> Option<Result<T, tokio::task::JoinError>> {
    match tokio::time::timeout(Duration::from_secs(CAPTURE_DRAIN_SECS), &mut task).await {
        Ok(joined) => Some(joined),
        Err(_) => {
            task.abort();
            None
        }
    }
}

/// 出力回収タスクを上限付きで待つ。上限を超えたらタスクを中断し、握りつぶさずエラーで返す。
async fn collect(
    task: tokio::task::JoinHandle<Result<Vec<u8>, String>>,
    label: &str,
    kind: &str,
) -> Result<Vec<u8>, String> {
    match join_capture(task).await {
        Some(Ok(result)) => result,
        Some(Err(error)) => Err(format!("{label}の{kind}回収に失敗しました: {error}")),
        None => Err(format!(
            "{label}の{kind}を回収できません（終了後もパイプを握った孫プロセスが残っています）。"
        )),
    }
}

async fn read_capture<R: tokio::io::AsyncRead + Unpin>(
    reader: Option<R>,
) -> Result<Vec<u8>, String> {
    let Some(mut reader) = reader else {
        return Ok(Vec::new());
    };
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| format!("プロセス出力を読み取れません: {error}"))?;
        if count == 0 {
            break;
        }
        let remaining = MAX_CAPTURE_BYTES.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..count.min(remaining)]);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::{command, output, output_or_install_hint};
    use std::process::Stdio;

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
            let result = output(cmd, 30, "テスト実行").await.unwrap();
            let text = String::from_utf8_lossy(&result.stdout).to_string();
            assert!(text.contains("picoagent"), "text: {text}");
        });
    }

    #[test]
    fn 実行ファイルは環境変数と既知の設置先から解決する() {
        // GUI アプリが継承する PATH に無くても、既知の設置先にあれば見つける
        // （macOS の Homebrew 経路。判定と実行を同じ探索へ揃えるための性質）。
        let dir = std::env::temp_dir().join(format!(
            "picoagent-resolve-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let name = "picoagent-fake-tool";
        let file = dir.join(super::executable_candidates(name).first().unwrap());
        std::fs::write(&file, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let dir_text = dir.to_string_lossy().to_string();

        // PATH にも既知の設置先にも無ければ見つからない。
        assert_eq!(super::resolve_executable_in(name, None, &[]), None);
        // 既知の設置先だけにあっても見つける。
        assert_eq!(
            super::resolve_executable_in(name, None, &[dir_text.as_str()]),
            Some(file.clone())
        );
        // PATH にあれば見つける。
        let path_env = std::ffi::OsString::from(&dir_text);
        assert_eq!(
            super::resolve_executable_in(name, Some(&path_env), &[]),
            Some(file.clone())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let fallback = dir.join("fallback");
            std::fs::create_dir_all(&fallback).unwrap();
            let executable = fallback.join(name);
            std::fs::write(&executable, b"").unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            let paths = std::env::join_paths([&dir, &fallback]).unwrap();
            assert_eq!(
                super::resolve_executable_in(name, Some(&paths), &[]),
                Some(executable)
            );
        }
        // ディレクトリを実行ファイルと取り違えない。
        assert_eq!(
            super::resolve_executable_in(
                dir.file_name().unwrap().to_str().unwrap(),
                None,
                &[std::env::temp_dir().to_string_lossy().as_ref()],
            ),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 拡張子付きで渡した名前へ拡張子を重ねない() {
        let candidates = super::executable_candidates("soffice.exe");
        assert_eq!(candidates, vec![std::ffi::OsString::from("soffice.exe")]);
        // 拡張子なしは、Windows だけ PATHEXT の候補へ広がる。
        let plain = super::executable_candidates("7z");
        if cfg!(windows) {
            assert!(plain.len() > 1, "candidates: {plain:?}");
            assert!(
                plain.contains(&std::ffi::OsString::from("7z.EXE"))
                    || plain.contains(&std::ffi::OsString::from("7z.exe"))
            );
        } else {
            assert_eq!(plain, vec![std::ffi::OsString::from("7z")]);
        }
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

    /// 導入案内は「起動できなかったとき」だけ足す。
    /// タイムアウトにも付けると、入っているのに「導入してください」と言うことになる。
    #[test]
    fn 導入案内は起動できなかったときだけ足す() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let missing = output_or_install_hint(
                command("picoagent-not-a-real-command"),
                10,
                "存在しない実行",
                "導入手順はこちら。",
            )
            .await
            .unwrap_err();
            assert!(missing.contains("起動できません"), "error: {missing}");
            assert!(missing.contains("導入手順はこちら。"), "error: {missing}");

            let mut slow = if cfg!(windows) {
                let mut cmd = command("cmd");
                cmd.args(["/c", "ping -n 31 127.0.0.1"]);
                cmd
            } else {
                let mut cmd = command("sh");
                cmd.args(["-c", "sleep 30"]);
                cmd
            };
            slow.stdout(Stdio::piped()).stderr(Stdio::piped());
            let timed_out = output_or_install_hint(slow, 1, "遅い実行", "導入手順はこちら。")
                .await
                .unwrap_err();
            assert!(timed_out.contains("タイムアウト"), "error: {timed_out}");
            assert!(
                !timed_out.contains("導入手順はこちら。"),
                "error: {timed_out}"
            );
        });
    }

    // タイムアウトで打ち切っても、プロセスツリーを回収して子孫を残さないこと。
    // シェル経由（孫プロセスあり）でも指定秒数で戻ること。孫がパイプを握っていると
    // 出力の終端が来ないため、回収を待つ実装では孫が終わるまで戻れなかった。
    #[cfg(unix)]
    #[test]
    fn タイムアウトで子プロセスを残さない() {
        let mut cmd = command("sh");
        cmd.args(["-c", "sleep 30"]);
        タイムアウトが指定秒数で戻る(cmd);
    }

    // 対応OSのもう一方でも同じ形（`cmd /c` の孫プロセス）で検査する。
    // unix だけで検査していたため、Windows 中心の開発では退行に気づけなかった。
    #[cfg(windows)]
    #[test]
    fn タイムアウトで子プロセスを残さない() {
        let mut cmd = command("cmd");
        cmd.args(["/c", "ping -n 31 127.0.0.1"]);
        タイムアウトが指定秒数で戻る(cmd);
    }

    #[cfg(any(unix, windows))]
    fn タイムアウトが指定秒数で戻る(cmd: tokio::process::Command) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
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

    /// 検査ファイルから親子PIDを読む。書き込み途中・共有違反は「まだ」として扱う。
    #[cfg(windows)]
    fn read_marker_ids(marker: &std::path::Path) -> Result<Vec<u32>, String> {
        let text = std::fs::read_to_string(marker).map_err(|error| error.to_string())?;
        let values = text
            .trim()
            .split(',')
            .map(|value| {
                value
                    .trim()
                    .parse::<u32>()
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 2 {
            return Err(format!("PIDが2個そろっていません: {:?}", text.trim()));
        }
        Ok(values)
    }

    #[cfg(windows)]
    #[test]
    fn 呼び出しfutureを破棄しても孫プロセスまで回収する() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let marker = std::env::temp_dir().join(format!(
                "picoagent-process-cancel-{}-{}.txt",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let escaped = marker.to_string_lossy().replace('\'', "''");
            let script = format!(
                "$child = Start-Process -FilePath 'ping.exe' -ArgumentList '-n','31','127.0.0.1' -PassThru -WindowStyle Hidden; \"$PID,$($child.Id)\" | Set-Content -Encoding ascii -LiteralPath '{escaped}'; Wait-Process -Id $child.Id"
            );
            let mut cmd = command("powershell.exe");
            cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            let running = tokio::spawn(output(cmd, 30, "破棄検査"));

            // 存在を待つだけでは足りない。書き手（PowerShell の Set-Content）は作った直後も
            // ハンドルを持っていて、その一瞬は共有違反で読めず、読めても内容が途中のことがある。
            // 「期待する形へ読み切れる」までを待機条件にする。
            let mut last_error = "まだ作られていません".to_string();
            let mut ids = Vec::new();
            let marker_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while std::time::Instant::now() < marker_deadline {
                match read_marker_ids(&marker) {
                    Ok(values) => {
                        ids = values;
                        break;
                    }
                    Err(reason) => last_error = reason,
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            // 起動確認に失敗した経路でも、assertする前に呼び出しを破棄して回収する。
            running.abort();
            let _ = running.await;
            if ids.is_empty() {
                // PIDを書き込む前に停止した場合も、自分の固有マーカーを起動引数に持つ
                // PowerShellだけを探す。名前だけで他のPowerShellを停止しない。
                let cleanup_script = format!(
                    "$targets = Get-CimInstance Win32_Process -Filter \"Name='powershell.exe'\" | Where-Object {{ $_.ProcessId -ne $PID -and $_.CommandLine -and $_.CommandLine.Contains('{escaped}') }}; foreach ($target in $targets) {{ & taskkill.exe /PID $target.ProcessId /T /F | Out-Null }}"
                );
                let mut cleanup = command("powershell.exe");
                cleanup.args(["-NoProfile", "-NonInteractive", "-Command", &cleanup_script]);
                let cleanup_result = output(cleanup, 10, "検査プロセスの回収").await;
                let _ = std::fs::remove_file(&marker);
                assert!(
                    cleanup_result.as_ref().is_ok_and(|result| result.status.success()),
                    "検査プロセスの回収に失敗しました: {cleanup_result:?}"
                );
                panic!("親子PIDを書いた検査ファイルを10秒以内に読み切れませんでした: {last_error}");
            }
            let mut alive = ids.clone();
            for _ in 0..120 {
                alive.retain(|pid| windows_process_exists(*pid));
                if alive.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            if !alive.is_empty() {
                // テスト失敗でも、自分が起動してPIDを記録したツリーだけを回収する。
                let _ = std::process::Command::new("taskkill.exe")
                    .args(["/PID", &ids[0].to_string(), "/T", "/F"])
                    .output();
            }
            let _ = std::fs::remove_file(&marker);
            assert!(alive.is_empty(), "中断後も残った検査プロセス: {alive:?}");
        });
    }

    #[cfg(windows)]
    fn windows_process_exists(pid: u32) -> bool {
        let filter = format!("PID eq {pid}");
        let Ok(result) = std::process::Command::new("tasklist.exe")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
        else {
            return true;
        };
        String::from_utf8_lossy(&result.stdout).contains(&format!("\"{pid}\""))
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

    /// 子プロセスの完了待ちに上限を付けること。
    ///
    /// `command.output().await` をそのまま呼ぶと**待ち時間の上限が無くなる**。
    /// 共通層の `process::output()` / `output_or_install_hint()` を使えばこの形にはならない。
    /// 実際に `image_tools` の ImageMagick 名乗り確認だけが素の `.output().await` で残っており、
    /// 壊れた実行ファイルを掴むと画像変換が永久に止まる状態だった。
    ///
    /// なお `tokio::time::timeout(..., command.output())` で包むのも不可（下の
    /// `一度きりの実行は共通層を通す` が別途検出する）。上限は付くが、打ち切り時に
    /// 回収されるのは直下の子だけで孫が残る。
    #[test]
    fn 子プロセスの完了待ちに上限を付ける() {
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "process.rs" {
                continue; // 上限を実装している共通層そのもの
            }
            for (number, line) in source.body().lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains(".output().await") {
                    violations.push(format!("{}:{}", source.path.display(), number + 1));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "子プロセスの完了待ちに上限がありません。`process::output()` を使うか \
             `tokio::time::timeout` で包んでください: {violations:?}"
        );
    }

    /// 一度きりの実行（起動 → 出力回収 → 終了待ち）は共通層の `process::output()` を通すこと。
    ///
    /// 同じ処理を各所で書き直すと、**出力回収の待ち方だけが抜ける**。実際に
    /// `whisper_transcribe` と `meeting_recorder` が共通層と同型の実装を持ち、
    /// どちらもタイムアウト後に回収タスクを待ち切っていた（孫プロセスがパイプを
    /// 握っていると終端が来ないため、上限を付けた意味が消える）。常駐プロセスや
    /// 進捗つき長時間処理は `child` を自分で持ってよいが、その場合も回収の待ちは
    /// `process::join_capture()` を使う。
    ///
    /// `tokio::time::timeout(..., command.output())` も同じ理由で検出する。上限は付くが、
    /// 打ち切りで `output()` のfutureを捨てても `kill_on_drop` が届くのは**直下の子だけ**で、
    /// 起動直後に本体を立て直す実行ファイル（LibreOffice の `soffice` → `soffice.bin` など）は
    /// 残り続ける。共通層は `terminate_process_tree()` でツリーごと回収する。
    /// 実際に `ms_office` / `image_tools` / `pdf_tools` の4箇所がこの形で残っていた。
    #[test]
    fn 一度きりの実行は共通層を通す() {
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "process.rs" {
                continue; // 共通層の実装そのもの
            }
            for (number, line) in source.body().lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("tokio::time::timeout")
                    && (line.contains(".wait()") || line.contains(".output()"))
                {
                    violations.push(format!("{}:{}", source.path.display(), number + 1));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "子プロセスの起動と終了待ちを自前で組み立てています。`process::output()` を使ってください: {violations:?}"
        );
    }

    #[test]
    fn 許可リストは共通判定だけが読み取る() {
        let violations: Vec<_> = guard_scan::all_sources()
            .into_iter()
            .filter(|source| source.name != "process.rs")
            .filter(|source| {
                let body = source.body();
                body.contains(r#"get("PICOAGENT_COMMAND_ALLOWLIST""#)
            })
            .map(|source| source.path.clone())
            .collect();
        assert!(
            violations.is_empty(),
            "許可リストはcommand_policyから読み取ってください: {violations:?}"
        );
        for body in [
            include_str!("../../../skills/run_command/mod.rs"),
            include_str!("../scheduler.rs"),
        ] {
            assert!(body.contains("command_policy::check"));
        }
    }

    /// 停止・回収・出力確定の失敗を捨てると、プロセスやファイルが残ったまま成功扱いになる。
    #[test]
    fn リソース解放の失敗を無視しない() {
        let operations = [
            ".start_kill(",
            ".kill(",
            ".wait(",
            ".join(",
            ".finalize(",
            ".write_sample(",
        ];
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "process.rs" {
                continue;
            }
            for line in source.body().lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("let _ =")
                    && operations
                        .iter()
                        .any(|operation| trimmed.contains(operation))
                {
                    violations.push(format!("{}: {trimmed}", source.path.display()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "リソース解放の失敗は伝播または診断出力してください:\n{}",
            violations.join("\n")
        );
    }
}
/// 対話・予約・承認後の実行が共有するコマンド許可の正本。
pub mod command_policy {
    use crate::config::Config;
    pub const DEFAULT_ALLOWLIST: &str = "git status,git diff,npm test,cargo test";
    const KEY: &str = "PICOAGENT_COMMAND_ALLOWLIST";

    pub fn tokens(program: &str, args: &[String]) -> Vec<String> {
        std::iter::once(program.to_string())
            .chain(args.iter().cloned())
            .collect()
    }

    pub fn validate_args(args: &[String]) -> Result<(), String> {
        if args.len() > 32 {
            return Err("args は32個以内で指定してください".into());
        }
        if args
            .iter()
            .any(|arg| arg.chars().count() > 500 || arg.contains('\0'))
        {
            return Err("各 arg は500文字以内で、NULを含めず指定してください".into());
        }
        Ok(())
    }

    /// 承認でも突破できない条件。allowlistへの登録前と承認後にも適用する。
    pub fn validate(program: &str, args: &[String]) -> Result<(), String> {
        if program.trim().is_empty() || program.contains('\0') {
            return Err("program が不正です".into());
        }
        validate_args(args)?;
        if super::is_physical_delete(program, args) {
            return Err(super::physical_delete_error()
                .trim_start_matches("エラー: ")
                .to_string());
        }
        Ok(())
    }

    pub fn check(cfg: &Config, program: &str, args: &[String]) -> Result<(), String> {
        validate(program, args)?;
        let requested = tokens(program, args);
        if cfg.get(KEY, DEFAULT_ALLOWLIST).split(',').any(|entry| {
            entry
                .split_whitespace()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                == requested
        }) {
            Ok(())
        } else {
            Err(format!(
                "許可されていないコマンドです: {}",
                requested.join(" ")
            ))
        }
    }

    /// 永続許可の保存値を作る。呼び出し元は状態ロック下で設定保存まで済ませる。
    pub fn approval_patch(
        cfg: &Config,
        program: &str,
        args: &[String],
    ) -> Result<Option<(String, String)>, String> {
        validate(program, args)?;
        if check(cfg, program, args).is_ok() {
            return Ok(None);
        }
        let entry = tokens(program, args).join(" ");
        // 保存形式が空白区切りなので、表せない引数を誤った許可へ書き換えない。
        if tokens(program, args)
            .iter()
            .any(|token| token.contains(',') || token.chars().any(char::is_whitespace))
        {
            return Err(
                "空白・カンマを含むコマンドは永続許可できません。今回だけ許可してください".into(),
            );
        }
        let current = cfg.get(KEY, DEFAULT_ALLOWLIST);
        let next = if current.trim().is_empty() {
            entry
        } else {
            format!("{},{entry}", current.trim())
        };
        Ok(Some((KEY.into(), next)))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn 許可表と承認不能条件を共通で判定する() {
            let cfg = crate::config::Config::new();
            cfg.set(
                "PICOAGENT_COMMAND_ALLOWLIST",
                "git status,rm item,powershell -Command Remove-Item",
            );
            for (program, args, expected) in [
                ("git", vec!["status"], true),
                ("git", vec!["diff"], false),
                ("rm", vec!["item"], false),
                ("powershell", vec!["-Command", "Remove-Item"], false),
            ] {
                let args: Vec<String> = args.into_iter().map(str::to_string).collect();
                assert_eq!(super::check(&cfg, program, &args).is_ok(), expected);
                if super::super::is_physical_delete(program, &args) {
                    assert!(super::approval_patch(&cfg, program, &args).is_err());
                }
            }
            assert!(super::validate("git", &vec!["a".into(); 33]).is_err());
            assert!(super::validate("git", &["a".repeat(501)]).is_err());
            assert!(super::approval_patch(&cfg, "git", &["a b".into()]).is_err());
        }
    }
}
