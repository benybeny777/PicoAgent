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

mod ownership;
use ownership::{OwnedAsyncChild, SpawnFailure};
pub(crate) use ownership::{OwnedChild, ProcessSnapshot};

/// タイムアウトの上限（どんな設定値でもこれを超えて待たない）。
const MAX_TIMEOUT_SECS: u64 = 7200;
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

/// OSへ存在確認だけを問い合わせる。確認不能を不存在として資産削除へ進めない。
pub(crate) fn process_alive(pid: u32) -> anyhow::Result<bool> {
    anyhow::ensure!(pid > 0, "確認対象のプロセスIDが不正です");
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::STILL_ACTIVE,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // OSから取得したハンドルだけをOwnedHandleへ移し、成功・失敗の両方で閉じる。
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return process_open_error(pid, std::io::Error::last_os_error());
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut code = 0;
        if unsafe { GetExitCodeProcess(handle.as_raw_handle(), &mut code) } == 0 {
            return Err(anyhow::anyhow!(
                "プロセス {pid} の実行状態を確認できません: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(code == STILL_ACTIVE as u32)
    }
    #[cfg(unix)]
    {
        let pid_os = i32::try_from(pid)
            .map_err(|error| anyhow::anyhow!("確認対象のプロセスIDが不正です: {error}"))?;
        // kill 0は確認専用。別プロセスへシグナルを送らない。
        if unsafe { libc::kill(pid_os, 0) } == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            Some(libc::EPERM) => Ok(true),
            _ => Err(anyhow::anyhow!(
                "プロセス {pid} の実行状態を確認できません: {error}"
            )),
        }
    }
}

#[cfg(windows)]
fn process_open_error(pid: u32, error: std::io::Error) -> anyhow::Result<bool> {
    use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;
    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        Ok(false)
    } else {
        Err(anyhow::anyhow!(
            "プロセス {pid} の実行状態を確認できません: {error}"
        ))
    }
}

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
    let mut child = OwnedAsyncChild::spawn(&mut command).map_err(|failure| match failure {
        SpawnFailure::Spawn(error) => {
            RunFailure::NotStarted(format!("{label}を起動できません: {error}"))
        }
        SpawnFailure::Ownership(error) => {
            RunFailure::Interrupted(format!("{label}の所有を確立できません: {error}"))
        }
    })?;
    let captures = OutputCapture {
        stdout: tokio::spawn(read_capture(child.take_stdout())),
        stderr: tokio::spawn(read_capture(child.take_stderr())),
    };
    let result: Result<Output, String> = async {
        let waited = tokio::select! {
            biased;
            _ = &mut abandon => return Err(format!("{label}を中断しました。")),
            waited = tokio::time::timeout(Duration::from_secs(timeout), child.wait()) => waited,
        };
        let status = waited
            .map_err(|_| format!("{label}が {timeout} 秒でタイムアウトしました。"))?
            .map_err(|error| format!("{label}の終了を待てません: {error}"))?;
        // 根が先に終了した後も、読取失敗・期限切れ・呼出future破棄を所有者へ伝える。
        let (stdout, stderr) = tokio::select! {
            biased;
            _ = &mut abandon => return Err(format!("{label}を中断しました。")),
            captured = captures.collect(label) => captured?,
        };
        // 非0終了コードも従来どおりOutputとして返す。正常EOFなら意図した起動先を残す。
        child.release().await?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
    .await;
    match result {
        Ok(output) => Ok(output),
        Err(error) => {
            let message = match child.terminate(label).await {
                Ok(()) => error,
                Err(cleanup) => format!("{error}\n子プロセスの回収にも失敗しました: {cleanup}"),
            };
            Err(RunFailure::Interrupted(message))
        }
    }
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

/// 出力を読むタスクを指定期限まで待ち、上限を超えたら中断して `None` を返す。
///
/// 子を停止しても、`sh -c` / `powershell -Command` が産んだ孫や、子がさらに起動した
/// プロセスがパイプの書き込み端を握っていると終端（EOF）が来ない。ここで待ち切ると
/// タイムアウトも中断も効かなくなるため、待ち方を共通化する。
pub async fn join_capture<T>(
    task: &mut tokio::task::JoinHandle<T>,
    deadline: tokio::time::Instant,
) -> Option<Result<T, tokio::task::JoinError>> {
    let joined = tokio::time::timeout_at(deadline, &mut *task).await.ok();
    if joined.is_none() {
        task.abort();
    }
    joined
}

/// 片方の失敗・待機の中断・子の終了確認失敗でも、両方の読取タスクを回収する。
struct OutputCapture {
    stdout: tokio::task::JoinHandle<Result<Vec<u8>, String>>,
    stderr: tokio::task::JoinHandle<Result<Vec<u8>, String>>,
}

impl Drop for OutputCapture {
    fn drop(&mut self) {
        self.stdout.abort();
        self.stderr.abort();
    }
}

impl OutputCapture {
    async fn collect(self, label: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
        self.collect_until(
            label,
            tokio::time::Instant::now() + Duration::from_secs(CAPTURE_DRAIN_SECS),
        )
        .await
    }

    async fn collect_until(
        mut self,
        label: &str,
        deadline: tokio::time::Instant,
    ) -> Result<(Vec<u8>, Vec<u8>), String> {
        // 両系統を同時に待ち、片方の失敗も同じ終了後期限も相手の待機へ埋もれさせない。
        tokio::try_join!(
            collect(&mut self.stdout, label, "標準出力", deadline),
            collect(&mut self.stderr, label, "標準エラー", deadline)
        )
    }
}

/// タスクの所有者を残したまま上限付きで待ち、失敗は所有者による両系統の回収へ返す。
async fn collect(
    task: &mut tokio::task::JoinHandle<Result<Vec<u8>, String>>,
    label: &str,
    kind: &str,
    deadline: tokio::time::Instant,
) -> Result<Vec<u8>, String> {
    match join_capture(task, deadline).await {
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
pub(crate) mod tests {
    #[cfg(windows)]
    use super::process_open_error;
    use super::{command, command_sync, output, output_or_install_hint, process_alive, OwnedChild};
    use std::process::Stdio;

    #[tokio::test]
    async fn 出力回収の失敗と中断で両方の読取タスクを残さない() {
        for reversed in [false, true] {
            for mode in ["読取失敗", "回収中断", "終了確認失敗"] {
                let (released, released_rx) = tokio::sync::oneshot::channel::<()>();
                let (ready, ready_rx) = tokio::sync::oneshot::channel();
                let stderr = tokio::spawn(async move {
                    // 書込側が生き続けるパイプと同じく、回収されるまで終わらない。
                    let _released = released;
                    ready.send(()).unwrap();
                    std::future::pending::<Result<Vec<u8>, String>>().await
                });
                ready_rx.await.unwrap();
                let stdout = tokio::spawn(async move {
                    if mode == "読取失敗" {
                        Err("読取エラー".to_string())
                    } else {
                        std::future::pending::<Result<Vec<u8>, String>>().await
                    }
                });
                let cleanup = [stdout.abort_handle(), stderr.abort_handle()];
                let captures = if reversed {
                    super::OutputCapture {
                        stdout: stderr,
                        stderr: stdout,
                    }
                } else {
                    super::OutputCapture { stdout, stderr }
                };
                match mode {
                    "読取失敗" => {
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(1),
                            captures.collect("検査"),
                        )
                        .await
                        .expect("他方の待機で読取失敗が遅延しました");
                        assert_eq!(result.unwrap_err(), "読取エラー")
                    }
                    "回収中断" => {
                        let task = tokio::spawn(async move { captures.collect("検査").await });
                        tokio::task::yield_now().await;
                        task.abort();
                        assert!(task.await.unwrap_err().is_cancelled());
                    }
                    _ => drop(captures),
                }
                let reclaimed =
                    tokio::time::timeout(std::time::Duration::from_secs(1), released_rx).await;
                // 退行検査が失敗する場合でも、この検査自身の読取タスクは残さない。
                for task in cleanup {
                    task.abort();
                }
                assert!(
                    reclaimed.is_ok(),
                    "{mode}・反転={reversed}で読取タスクが残りました"
                );
            }
        }
    }

    #[tokio::test]
    async fn 両出力の回収期限は片側の完了時に延長されない() {
        for reversed in [false, true] {
            let start = tokio::time::Instant::now();
            let stdout = tokio::spawn(async move {
                tokio::time::sleep_until(start + std::time::Duration::from_millis(150)).await;
                Ok(b"early".to_vec())
            });
            let stderr = tokio::spawn(async move {
                tokio::time::sleep_until(start + std::time::Duration::from_millis(450)).await;
                Ok(b"late".to_vec())
            });
            let captures = if reversed {
                super::OutputCapture {
                    stdout: stderr,
                    stderr: stdout,
                }
            } else {
                super::OutputCapture { stdout, stderr }
            };
            let error = captures
                .collect_until("期限検査", start + std::time::Duration::from_millis(350))
                .await
                .unwrap_err();
            let delayed = if reversed {
                "標準出力"
            } else {
                "標準エラー"
            };
            assert!(
                error.contains(delayed) && error.contains("回収できません"),
                "{error}"
            );
        }
    }

    #[test]
    fn 実プロセスの生存と終了を確認コマンドなしで問い合わせる() {
        assert!(process_alive(std::process::id()).unwrap());
        assert!(process_alive(0).is_err());
        #[cfg(windows)]
        let mut command = command_sync("powershell.exe");
        #[cfg(windows)]
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ]);
        #[cfg(unix)]
        let mut command = command_sync("sleep");
        #[cfg(unix)]
        command.arg("30");
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        let alive = process_alive(pid);
        let stopped = child.kill();
        let waited = child.wait();
        assert!(stopped.is_ok(), "検査用の子を停止できません: {stopped:?}");
        assert!(waited.is_ok(), "検査用の子を回収できません: {waited:?}");
        assert!(alive.unwrap());
        assert!(!process_alive(pid).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn windowsの不存在だけをfalseにして権限不足と確認不能を理由付きで返す() {
        use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER};
        assert!(!process_open_error(
            123,
            std::io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as i32)
        )
        .unwrap());
        for code in [ERROR_ACCESS_DENIED as i32, 1] {
            let error = process_open_error(123, std::io::Error::from_raw_os_error(code))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("プロセス 123")
                    && error.contains(&std::io::Error::from_raw_os_error(code).to_string()),
                "{error}"
            );
        }
        assert!(!process_alive(u32::MAX).unwrap());
    }

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
    pub(crate) fn read_marker_ids(marker: &std::path::Path) -> Result<Vec<u32>, String> {
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

    pub(crate) fn fixture_command(marker: &std::path::Path, role: &str) -> std::process::Command {
        let mut command = command_sync(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "tools::process::tests::所有範囲の検査用プロセス",
            "--nocapture",
        ]);
        command
            .env("PICOAGENT_PROCESS_TEST_ROLE", role)
            .env("PICOAGENT_PROCESS_TEST_MARKER", marker);
        command
    }

    fn fixture_stop_path(marker: &std::path::Path) -> std::path::PathBuf {
        marker.with_extension("stop")
    }

    /// 同じ検査実行ファイルで親先行終了とpipe継承を作り、シェル固有の起動仕様を避ける。
    pub(crate) fn tree_test_command(
        marker: &std::path::Path,
        wait_for_child: bool,
        inherit_output: bool,
        exit_code: i32,
    ) -> std::process::Command {
        let mut command = fixture_command(marker, "root");
        command
            .env(
                "PICOAGENT_PROCESS_TEST_WAIT",
                if wait_for_child { "1" } else { "0" },
            )
            .env(
                "PICOAGENT_PROCESS_TEST_PIPE",
                if inherit_output { "1" } else { "0" },
            )
            .env("PICOAGENT_PROCESS_TEST_EXIT", exit_code.to_string());
        command
    }

    #[test]
    fn 所有範囲の検査用プロセス() {
        let Ok(role) = std::env::var("PICOAGENT_PROCESS_TEST_ROLE") else {
            return;
        };
        let marker =
            std::path::PathBuf::from(std::env::var_os("PICOAGENT_PROCESS_TEST_MARKER").unwrap());
        if role == "marker" {
            std::fs::write(marker, "started").unwrap();
            return;
        }
        if role == "leaf" {
            let stop = fixture_stop_path(&marker);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !stop.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            return;
        }
        assert_eq!(role, "root");
        let mut command = fixture_command(&marker, "leaf");
        command.stdin(Stdio::null());
        if std::env::var("PICOAGENT_PROCESS_TEST_PIPE").unwrap() == "0" {
            // 子の標準出力をnullへ変えても、Windowsでは根のpipeハンドルが別途
            // 継承され得る。専用fixture内だけで継承フラグを外してEOF条件を作る。
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
                for handle in [
                    std::io::stdout().as_raw_handle(),
                    std::io::stderr().as_raw_handle(),
                ] {
                    assert_ne!(
                        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) },
                        0
                    );
                }
            }
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        let mut child = command.spawn().unwrap();
        std::fs::write(&marker, format!("{},{}", std::process::id(), child.id())).unwrap();
        if std::env::var("PICOAGENT_PROCESS_TEST_WAIT").unwrap() == "1" {
            child.wait().unwrap();
        }
        std::process::exit(
            std::env::var("PICOAGENT_PROCESS_TEST_EXIT")
                .unwrap()
                .parse()
                .unwrap(),
        );
    }

    pub(crate) fn wait_marker_ids(marker: &std::path::Path) -> Vec<u32> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(ids) = read_marker_ids(marker) {
                return ids;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "検査用の親子PIDがそろいません"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// 起動時に渡した固有の終了要求ファイルだけでfixtureを回収する。
    /// PIDは観測用であり、終了済みPIDが再利用されても停止操作には使わない。
    pub(crate) struct TestProcessIds(pub(crate) Vec<u32>, std::path::PathBuf);

    impl TestProcessIds {
        pub(crate) fn capture(marker: &std::path::Path) -> Self {
            // markerの待機が失敗しても、Dropで同じfixtureへ終了を要求する。
            let mut ids = Self(Vec::new(), marker.to_owned());
            ids.0 = wait_marker_ids(marker);
            ids
        }
    }

    pub(crate) fn test_process_running(pid: u32) -> bool {
        #[cfg(target_os = "linux")]
        {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return false;
            };
            let state = stat
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .next()
                .unwrap();
            state != "Z" && state != "X"
        }
        #[cfg(target_os = "macos")]
        {
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as i32;
            let read = unsafe {
                libc::proc_pidinfo(
                    pid as i32,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    &mut info as *mut _ as _,
                    size,
                )
            };
            read == size && info.pbi_status != libc::SZOMB
        }
        #[cfg(windows)]
        process_alive(pid).unwrap_or(true)
    }

    impl Drop for TestProcessIds {
        fn drop(&mut self) {
            if let Err(error) = std::fs::write(fixture_stop_path(&self.1), []) {
                eprintln!("検査プロセスへ終了を要求できません: {error}");
                return;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                if self.0.is_empty() {
                    if let Ok(ids) = read_marker_ids(&self.1) {
                        self.0 = ids;
                    }
                }
                if !self.0.is_empty() && self.0.iter().all(|pid| !test_process_running(*pid)) {
                    return;
                }
                if std::time::Instant::now() >= deadline {
                    // 観測失敗やPID再利用の疑いがあっても、番号指定の停止へ切り替えない。
                    eprintln!("検査プロセスの終了を期限内に確認できません: {:?}", self.1);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    #[test]
    fn 根が先に終了した孫を回収し正常出力時の起動先は残す() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 同名の別ジョブは、各回収で巻き込まれないことを同じ検査の中で確かめる。
            let independent_dir = tempfile::tempdir().unwrap();
            let independent_marker = independent_dir.path().join("independent.txt");
            let mut independent_cmd = tree_test_command(&independent_marker, true, false, 0);
            independent_cmd.stdout(Stdio::null()).stderr(Stdio::null());
            let mut independent = OwnedChild::spawn(&mut independent_cmd).unwrap();
            let independent_ids = TestProcessIds::capture(&independent_marker);
            for (inherit_pipe, cancel, exit_code) in [
                (false, false, 0),
                (false, false, 7),
                (true, false, 0),
                (true, true, 0),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let marker = directory.path().join("owned.txt");
                let mut command = tokio::process::Command::from(tree_test_command(
                    &marker,
                    false,
                    inherit_pipe,
                    exit_code,
                ));
                command.stdout(Stdio::piped()).stderr(Stdio::piped());
                let running = tokio::spawn(output(command, 10, "親先行終了検査"));
                let ids = TestProcessIds::capture(&marker);
                if cancel {
                    // 根の終了後に出力回収中のfutureを破棄する。
                    for _ in 0..200 {
                        if !test_process_running(ids.0[0]) {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    assert!(
                        !test_process_running(ids.0[0]),
                        "親退出後の取消検査で根が終了していません"
                    );
                    running.abort();
                    let _ = running.await;
                } else {
                    let result = running.await.unwrap();
                    if inherit_pipe {
                        assert!(result.unwrap_err().contains("回収できません"));
                    } else {
                        assert_eq!(result.unwrap().status.code(), Some(exit_code));
                        assert!(
                            test_process_running(ids.0[1]),
                            "正常EOFで起動先を停止しました"
                        );
                    }
                }
                if inherit_pipe {
                    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
                    while test_process_running(ids.0[1]) && tokio::time::Instant::now() < deadline {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    assert!(
                        !test_process_running(ids.0[1]),
                        "親退出後の孫が残っています"
                    );
                }
                assert!(
                    process_alive(independent.id()).unwrap(),
                    "別ジョブを停止しました"
                );
                assert!(
                    process_alive(independent_ids.0[1]).unwrap(),
                    "別ジョブの孫を停止しました"
                );
            }
            independent.terminate("独立ジョブの検査後始末").unwrap();
        });
    }

    #[cfg(windows)]
    #[test]
    fn 呼び出しfutureを破棄しても孫プロセスまで回収する() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("owned.txt");
            let mut command =
                tokio::process::Command::from(tree_test_command(&marker, true, true, 0));
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
            let running = tokio::spawn(output(command, 30, "破棄検査"));
            let ids = TestProcessIds::capture(&marker);
            running.abort();
            let _ = running.await;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
            while ids.0.iter().any(|pid| test_process_running(*pid))
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert!(
                ids.0.iter().all(|pid| !test_process_running(*pid)),
                "中断後も検査プロセスが残っています: {:?}",
                ids.0
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

    fn is_process_layer(source: &guard_scan::Source) -> bool {
        source.path == std::path::Path::new("src-tauri/src/tools/process.rs")
            || source.path.starts_with("src-tauri/src/tools/process")
    }

    /// 外部プロセスの起動は `tools::process::command()` 経由に統一すること。
    #[test]
    fn プロセス起動作法を自前実装しない() {
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            let name = &source.name;
            if is_process_layer(source) {
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
            if is_process_layer(source) {
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
    /// 残り続ける。共通層は `OwnedAsyncChild` が所有する範囲を回収する。
    /// 実際に `ms_office` / `image_tools` / `pdf_tools` の4箇所がこの形で残っていた。
    #[test]
    fn 一度きりの実行は共通層を通す() {
        let mut violations = Vec::new();
        for source in guard_scan::all_sources() {
            if is_process_layer(source) {
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
