// permissions.rs - 全スキル共通のファイル/ディレクトリ許可・拒否ルート。
//
// これまで各スキルが個別に持っていた許可ルート（PICOAGENT_TOOLS_ROOT /
// PICOAGENT_WORKSPACE_ROOT / PICOAGENT_DOCUMENT_ROOT / PICOAGENT_COMMAND_ROOT など）を
// この1箇所へ集約する。すべてのファイル・ディレクトリ・コマンド実行スキルはここを参照する。
//
// 正本キー:
//   PICOAGENT_ALLOWED_ROOT   … 主たる許可ルート（相対パスの基準。既定はホーム）
//   PICOAGENT_ALLOWED_PATHS  … 追加の許可ルート（改行または ; 区切り）
//   PICOAGENT_DENIED_PATHS   … 追加の拒否パス（改行または ; 区切り）
// 常に拒否: C:\Windows, C:\Program Files, C:\Program Files (x86)（Windows）。
//
// 旧キーからの移行: 新キーが未設定でも、既存ユーザーの設定を壊さないよう
// 旧ルートキーが設定されていればそれを許可ルートとして引き継ぐ。

use std::path::{Path, PathBuf};

use crate::config::Config;

/// 旧ルートキー（新キー未設定時に許可ルートへ引き継ぐ）。
const LEGACY_ROOT_KEYS: &[&str] = &[
    "PICOAGENT_TOOLS_ROOT",
    "PICOAGENT_WORKSPACE_ROOT",
    "PICOAGENT_DOCUMENT_ROOT",
    "PICOAGENT_COMMAND_ROOT",
];

/// 廃止済みだが、既存設定が指す場所を追加許可ルートとしてだけ引き継ぐキー。
const LEGACY_ADDITIONAL_ROOT_KEYS: &[&str] = &["PICOAGENT_MEETING_ROOT"];

pub fn home() -> String {
    dirs::home_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn split_paths(value: &str) -> Vec<PathBuf> {
    value
        .split(['\n', ';'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// 主たる許可ルート（相対パスの基準）。新キー→旧キー→ホームの順で解決する。
pub fn primary_root(cfg: &Config) -> PathBuf {
    let configured = cfg.get("PICOAGENT_ALLOWED_ROOT", "");
    if !configured.trim().is_empty() {
        return PathBuf::from(configured);
    }
    for key in LEGACY_ROOT_KEYS {
        let value = cfg.get(key, "");
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    PathBuf::from(home())
}

/// 許可ルート一式（主ルート＋追加許可パス＋設定済みの旧ルートキー）。
/// このいずれかの配下にある対象だけを読み書き・実行の対象にできる。
pub fn allowed_roots(cfg: &Config) -> Vec<PathBuf> {
    let mut roots = vec![primary_root(cfg)];
    roots.extend(split_paths(&cfg.get("PICOAGENT_ALLOWED_PATHS", "")));
    // 旧キーの追加許可パスも引き継ぐ（run_command の従来設定を壊さない）。
    roots.extend(split_paths(&cfg.get("PICOAGENT_COMMAND_ALLOWED_PATHS", "")));
    // 設定済みの旧ルートキーも許可ルートとして扱う（移行期の互換）。
    for key in LEGACY_ROOT_KEYS {
        let value = cfg.get(key, "");
        if !value.trim().is_empty() {
            roots.push(PathBuf::from(value));
        }
    }
    for key in LEGACY_ADDITIONAL_ROOT_KEYS {
        let value = cfg.get(key, "");
        if !value.trim().is_empty() {
            roots.push(PathBuf::from(value));
        }
    }
    // 正規化して重複を除く。
    let mut normalized: Vec<PathBuf> = Vec::new();
    for root in roots {
        let root = normalize(&root, ".");
        if !normalized.contains(&root) {
            normalized.push(root);
        }
    }
    normalized
}

/// パスを正規化する（絶対パスはそのまま、相対は base 基準。`..` は畳む）。
pub fn normalize(base: &Path, input: &str) -> PathBuf {
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

/// 既存の最深祖先だけを実体化し、残りの未作成部分を戻した検査用パスを返す。
///
/// 許可ルート内のシンボリックリンクや Windows のジャンクションを経由して、実体が
/// ルート外へ出るのを防ぐ。既存パスの実体化に失敗した場合は、安全側に倒して拒否する。
fn path_for_permission_check(path: &Path) -> Result<PathBuf, String> {
    let mut cursor = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(&cursor) {
            Ok(mut resolved) => {
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(error) => {
                if std::fs::symlink_metadata(&cursor).is_ok() {
                    return Err(format!(
                        "パスの実体を確認できないため操作できません: {} ({error})",
                        cursor.display()
                    ));
                }
                let Some(name) = cursor.file_name().map(|value| value.to_os_string()) else {
                    return Ok(path.to_path_buf());
                };
                missing.push(name);
                if !cursor.pop() {
                    return Ok(path.to_path_buf());
                }
            }
        }
    }
}

/// 常に操作を拒否する保護パスか（C:\Windows・Program Files、および PICOAGENT_DENIED_PATHS 配下）。
pub fn is_protected(cfg: &Config, path: &Path) -> bool {
    if is_system_protected(path) {
        return true;
    }
    for denied in split_paths(&cfg.get("PICOAGENT_DENIED_PATHS", "")) {
        let denied = normalize(&denied, ".");
        let target = normalize(path, ".");
        if target == denied || target.starts_with(&denied) {
            return true;
        }
    }
    false
}

/// OS の保護領域（Windows / Program Files）。設定に関わらず常に拒否する。
#[cfg(windows)]
pub fn is_system_protected(path: &Path) -> bool {
    let lower = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let lower = lower.strip_prefix(r"\\?\").unwrap_or(&lower);
    lower.starts_with(r"c:\windows")
        || lower.starts_with(r"c:\program files\")
        || lower.starts_with(r"c:\program files (x86)\")
        || lower == r"c:\program files"
        || lower == r"c:\program files (x86)"
}

#[cfg(not(windows))]
pub fn is_system_protected(_path: &Path) -> bool {
    false
}

/// 許可判定を通過したパス。
///
/// **スキルがユーザー由来のパスを扱うときは、必ずこの型を経由する。**
/// 生成経路は `resolve()` / `SafePath::join()` / `SafePath::sibling()` だけに限定してあり、
/// 内部の `PathBuf` は非公開なので「検証を通さずに組み立てたパス」を
/// 誤って渡せない。スキル側で許可判定を自前実装するのは禁止
/// （`permissions::guard` のテストが検出して失敗させる）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafePath(PathBuf);

impl SafePath {
    /// 検証済みの実パス。プロセス起動やファイル操作へ渡す用。
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// エラーメッセージ表示用。
    pub fn display(&self) -> std::path::Display<'_> {
        self.0.display()
    }

    pub fn exists(&self) -> bool {
        self.0.exists()
    }

    /// 拡張子（小文字）。形式判定用。
    pub fn extension_lossy(&self) -> String {
        self.0
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default()
    }

    pub fn file_stem_lossy(&self) -> String {
        self.0
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// 配下の相対パスを足す。`..` で外へ出る指定を防ぐため必ず再検証する。
    pub fn join(&self, cfg: &Config, relative: &str) -> Result<SafePath, String> {
        if Path::new(relative).is_absolute() {
            return Err(format!(
                "配下へ追加するパスは相対パスで指定してください: {relative}"
            ));
        }
        let joined = normalize(&self.0, relative);
        if joined != self.0 && !joined.starts_with(&self.0) {
            return Err(format!(
                "基準フォルダの外へ出る相対パスは使えません: {relative}"
            ));
        }
        resolve(cfg, &joined.to_string_lossy())
    }

    /// 同じ親ディレクトリ内の別ファイル（拡張子違いなど）。こちらも再検証する。
    pub fn sibling(&self, cfg: &Config, file_name: &str) -> Result<SafePath, String> {
        let parent = self.0.parent().unwrap_or(Path::new("."));
        let input = Path::new(file_name);
        if input.is_absolute() || input.components().count() != 1 {
            return Err(format!(
                "同じフォルダ内のファイル名だけを指定してください: {file_name}"
            ));
        }
        resolve(cfg, &parent.join(input).to_string_lossy())
    }

    /// 親ディレクトリ。ルート直下などで親が無い場合は主ルートを返す。
    /// 親は検証済みパスの祖先なので、許可ルート内であることは保証される。
    pub fn parent_or_root(&self, cfg: &Config) -> SafePath {
        match self.0.parent() {
            Some(parent) if parent.as_os_str().is_empty() => SafePath(primary_root(cfg)),
            Some(parent) => SafePath(parent.to_path_buf()),
            None => SafePath(primary_root(cfg)),
        }
    }

    pub fn file_name_lossy(&self) -> String {
        self.0
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// アプリが自分で決めた内部パス（app_data_dir 配下・一時ファイルなど、
    /// ユーザー入力を含まないもの）を許可済みとして扱う。
    /// **ユーザー由来の文字列には使わないこと**（使うと許可判定を素通りする）。
    pub fn app_owned(path: PathBuf) -> SafePath {
        SafePath(path)
    }
}

// 検証済みパスは、そのままファイル操作やプロセス引数へ渡せるようにする。
// 安全性の要は「生成経路が resolve / join / sibling に限られること」なので、
// 参照変換を許してもユーザー入力が検証を迂回することはない。
impl AsRef<Path> for SafePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for SafePath {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_os_str()
    }
}

/// 許可済みパスに対するファイル操作。`SafePath` しか受け取らないため、
/// 検証を通していないパスを書き込み・読み取りへ渡せない。
// 現時点で呼び出しが無い関数も、スキル追加時の正規の入口として提供しておく。
#[allow(dead_code)]
pub mod fs {
    use super::SafePath;

    pub fn metadata(path: &SafePath) -> Result<std::fs::Metadata, String> {
        std::fs::metadata(path.as_path()).map_err(|error| {
            format!(
                "ファイル情報を取得できません（{}）: {}",
                path.display(),
                error
            )
        })
    }

    pub fn read_dir(path: &SafePath) -> Result<std::fs::ReadDir, String> {
        std::fs::read_dir(path.as_path()).map_err(|error| {
            format!(
                "フォルダ一覧を取得できません（{}）: {}",
                path.display(),
                error
            )
        })
    }

    pub fn read_to_string(path: &SafePath) -> Result<String, String> {
        std::fs::read_to_string(path.as_path())
            .map_err(|error| format!("ファイルを読めません（{}）: {}", path.display(), error))
    }

    pub fn read(path: &SafePath) -> Result<Vec<u8>, String> {
        std::fs::read(path.as_path())
            .map_err(|error| format!("ファイルを読めません（{}）: {}", path.display(), error))
    }

    pub fn write(path: &SafePath, contents: impl AsRef<[u8]>) -> Result<(), String> {
        if let Some(parent) = path.as_path().parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "フォルダを作成できません（{}）: {}",
                    parent.display(),
                    error
                )
            })?;
        }
        std::fs::write(path.as_path(), contents)
            .map_err(|error| format!("ファイルを書き込めません（{}）: {}", path.display(), error))
    }

    pub fn append(path: &SafePath, contents: impl AsRef<[u8]>) -> Result<(), String> {
        use std::io::Write;

        if let Some(parent) = path.as_path().parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "フォルダを作成できません（{}）: {}",
                    parent.display(),
                    error
                )
            })?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_path())
            .and_then(|mut file| file.write_all(contents.as_ref()))
            .map_err(|error| format!("ファイルへ追記できません（{}）: {}", path.display(), error))
    }

    pub fn create_dir_all(path: &SafePath) -> Result<(), String> {
        std::fs::create_dir_all(path.as_path())
            .map_err(|error| format!("フォルダを作成できません（{}）: {}", path.display(), error))
    }
}

/// 入力パスを主ルート基準で正規化し、許可ルート内かつ保護パスでないことを検証して返す。
/// 違反時はモデルへ返す日本語エラー文字列を Err で返す。
///
/// スキルからユーザー由来のパスを扱う入口はこの関数だけにする（自前の許可判定は禁止）。
pub fn resolve(cfg: &Config, input: &str) -> Result<SafePath, String> {
    resolve_raw(cfg, input).map(SafePath)
}

fn resolve_raw(cfg: &Config, input: &str) -> Result<PathBuf, String> {
    let target = normalize(&primary_root(cfg), input);
    if is_protected(cfg, &target) {
        return Err(format!(
            "許可されていない保護パスです（C:\\Windows・Program Files や拒否設定の配下は操作できません）: {}",
            target.display()
        ));
    }
    let checked_target = path_for_permission_check(&target)?;
    if is_protected(cfg, &checked_target) {
        return Err(format!(
            "許可されていない保護パスです（リンク先を含め、C:\\Windows・Program Files や拒否設定の配下は操作できません）: {}",
            target.display()
        ));
    }
    let mut within = false;
    for root in allowed_roots(cfg) {
        let root = normalize(&root, ".");
        let checked_root = path_for_permission_check(&root)?;
        if checked_target == checked_root || checked_target.starts_with(&checked_root) {
            within = true;
            break;
        }
    }
    if within {
        Ok(target)
    } else {
        Err(format!(
            "許可ルートの外は操作できません（設定の「安全・許可」タブで許可ルートを追加してください）: {}",
            target.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::is_system_protected;
    use super::normalize;
    #[cfg(windows)]
    use std::path::PathBuf;

    // Windows のパス区切り（\）とドライブレターを前提にしたテスト。
    // Linux では区切りが / のため期待値が一致しない。Windows 限定で実行する。
    #[cfg(windows)]
    #[test]
    fn 相対パスをルート基準に正規化する() {
        let base = PathBuf::from(r"C:\work");
        assert_eq!(normalize(&base, "a/b"), PathBuf::from(r"C:\work\a\b"));
        // `..` はルート内で畳む。
        assert_eq!(normalize(&base, "a/../b"), PathBuf::from(r"C:\work\b"));
    }

    // どの OS でも成り立つ正規化チェック。
    #[test]
    fn 相対パスをルート基準に正規化する_共通() {
        let base = std::env::temp_dir().join("picoagent-perm-test");
        assert_eq!(normalize(&base, "a/../b"), base.join("b"));
    }

    #[test]
    fn 配下結合と同階層名は基準フォルダから脱出しない() {
        let base = std::env::temp_dir().join(format!("picoagent-join-test-{}", std::process::id()));
        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", base.to_string_lossy().as_ref());
        let root = super::SafePath::app_owned(base.clone());
        assert_eq!(
            root.join(&cfg, "child/file.txt").unwrap().as_path(),
            base.join("child/file.txt")
        );
        assert!(root.join(&cfg, "../outside.txt").is_err());
        assert!(root
            .join(&cfg, &base.join("absolute.txt").to_string_lossy())
            .is_err());
        let file = super::SafePath::app_owned(base.join("file.txt"));
        assert!(file.sibling(&cfg, "../outside.txt").is_err());
        assert!(file.sibling(&cfg, "sub/file.txt").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windowsとprogram_filesは常に保護() {
        assert!(is_system_protected(&PathBuf::from(r"C:\Windows\System32")));
        assert!(is_system_protected(&PathBuf::from(r"C:\Program Files\App")));
        assert!(is_system_protected(&PathBuf::from(
            r"C:\Program Files (x86)\App"
        )));
        assert!(!is_system_protected(&PathBuf::from(r"C:\Users\me\doc.txt")));
        assert!(is_system_protected(&PathBuf::from(
            r"\\?\C:\Windows\System32"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn シンボリックリンク経由の許可ルート外を拒否する() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("picoagent-perm-link-{}", std::process::id()));
        let allowed = base.join("allowed");
        let outside = base.join("outside");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, allowed.join("escape")).unwrap();

        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", allowed.to_string_lossy().as_ref());
        assert!(super::resolve(&cfg, "escape/secret.txt").is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}

/// 設計ガード: スキルがファイル許可の実装を自前で持ち込んでいないかを機械的に検査する。
///
/// 過去に「各スキルが独自の許可判定をコピーして持ち、リンク実体検査・追加許可ルート・
/// 拒否パス判定が抜ける」という同じ不具合を繰り返した。人のレビューに頼らず、
/// テストで検出して落とす。
#[cfg(test)]
mod guard {

    /// 自前の許可判定を持つことを許すスキル（意図的な例外のみ。追加には理由を添えること）。
    /// - `run_command` / `code_runner`: 実行系。cwd と allowlist を含む独自モデルを持つ
    /// - `trash_file`: 削除専用。実在必須＋許可ルート自体の削除拒否という追加制約がある
    const OWN_CHECK_ALLOWED: [&str; 3] = ["run_command", "code_runner", "trash_file"];

    use crate::guard_scan;

    /// スキル内に許可判定の再実装（`starts_with(base)` 方式のコピー）が無いこと。
    #[test]
    fn スキルは許可判定を自前実装しない() {
        let mut violations = Vec::new();
        for source in guard_scan::skill_sources() {
            let (name, text) = (&source.name, &source.text);
            if OWN_CHECK_ALLOWED.contains(&name.as_str()) {
                continue;
            }
            for marker in [
                "fn bounded_path(base:",
                "fn normalize(base:",
                "target.starts_with(base)",
            ] {
                if text.contains(marker) {
                    violations.push(format!("{name}: `{marker}`"));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "スキルが許可判定を自前実装しています。`permissions::resolve()` を使ってください\n（リンク実体検査・追加許可ルート・拒否パス判定が漏れます）:\n{}",
            violations.join("\n")
        );
    }

    /// パス引数を受け取るスキルは、必ず共通の許可判定を通していること。
    #[test]
    fn パスを受け取るスキルは共通の許可判定を通す() {
        let mut violations = Vec::new();
        for source in guard_scan::skill_sources() {
            let (name, text) = (&source.name, &source.text);
            if OWN_CHECK_ALLOWED.contains(&name.as_str()) {
                continue;
            }
            // parameters() に *_path / *_dir を宣言している＝ユーザー由来のパスを受け取る。
            let takes_path = text.contains("_path\": {") || text.contains("_dir\": {");
            if !takes_path {
                continue;
            }
            if !text.contains("permissions::resolve") {
                violations.push(name.clone());
            }
        }
        assert!(
            violations.is_empty(),
            "パス引数を持つのに `permissions::resolve()` を通していないスキルがあります:\n{}",
            violations.join("\n")
        );
    }

    /// 許可済みパスへの書き込みは `permissions::fs` のファサードを通すこと。
    ///
    /// `SafePath` は `AsRef<Path>` なので `std::fs::write(&safe_path, ..)` がそのまま
    /// 書けてしまい、ファサードが持つ親フォルダ自動作成と統一エラーメッセージを
    /// 素通りする。将来ファサードに監査ログやサイズ上限を足したときも漏れる。
    #[test]
    fn 許可済みパスへの書き込みはファサードを通す() {
        /// 一時ファイル経由の書き込みなど、ファサードに載せられない箇所（理由必須）。
        /// - `ms_office`: 出力を壊さないための一時ファイル＋rename（app_owned な中間パス）
        /// - `trash_file`: ゴミ箱の .trashinfo（XDG 仕様のメタファイルで許可ルート外）
        const FACADE_EXEMPT: [(&str, &str); 3] = [
            ("ms_office", "一時ファイル経由の原子的な差し替え"),
            ("trash_file", "ゴミ箱メタファイル（.trashinfo）"),
            (
                "code_runner",
                "実行用スクリプトの内部一時ファイル（ユーザー指定パスではない）",
            ),
        ];

        let mut violations = Vec::new();
        for source in guard_scan::skill_sources() {
            if FACADE_EXEMPT
                .iter()
                .any(|(name, _)| *name == source.name.as_str())
            {
                continue;
            }
            for (number, line) in source.body().lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("std::fs::write(") {
                    violations.push(format!("{}:{}", source.path.display(), number + 1));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "許可済みパスへの書き込みは `permissions::fs::write()` を使ってください\n（親フォルダ自動作成とエラーメッセージが統一されます）:\n{}",
            violations.join("\n")
        );
    }
}
