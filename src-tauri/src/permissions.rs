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

/// 入力パスを主ルート基準で正規化し、許可ルート内かつ保護パスでないことを検証して返す。
/// 違反時はモデルへ返す日本語エラー文字列を Err で返す。
pub fn resolve(cfg: &Config, input: &str) -> Result<PathBuf, String> {
    let target = normalize(&primary_root(cfg), input);
    if is_protected(cfg, &target) {
        return Err(format!(
            "許可されていない保護パスです（C:\\Windows・Program Files や拒否設定の配下は操作できません）: {}",
            target.display()
        ));
    }
    let within = allowed_roots(cfg).into_iter().any(|root| {
        let root = normalize(&root, ".");
        target == root || target.starts_with(&root)
    });
    if within {
        Ok(target)
    } else {
        Err(format!(
            "許可ルートの外は操作できません（設定の「許可」タブで許可ルートを追加してください）: {}",
            target.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{is_system_protected, normalize};
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

    #[cfg(windows)]
    #[test]
    fn windowsとprogram_filesは常に保護() {
        assert!(is_system_protected(&PathBuf::from(r"C:\Windows\System32")));
        assert!(is_system_protected(&PathBuf::from(r"C:\Program Files\App")));
        assert!(is_system_protected(&PathBuf::from(
            r"C:\Program Files (x86)\App"
        )));
        assert!(!is_system_protected(&PathBuf::from(r"C:\Users\me\doc.txt")));
    }
}
