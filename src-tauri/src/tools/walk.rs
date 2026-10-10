// 許可済みルート内のファイル走査と、省略原因の報告を一か所で持つ。
use crate::config::Config;
use crate::permissions::{self, SafePath};
use std::path::PathBuf;
use std::time::SystemTime;

pub const MAX_DEPTH: usize = 12;
pub const MAX_SCAN_FILES: usize = 20_000;
pub const SKIP_DIRS: [&str; 8] = [
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
];

pub struct File {
    pub relative: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
}
#[derive(Default)]
pub struct WalkResult {
    pub files: Vec<File>,
    pub skipped: Vec<(PathBuf, String)>,
    pub truncated: bool,
}
impl WalkResult {
    pub fn diagnostic(&self) -> String {
        let mut text = String::new();
        if self.truncated {
            text.push_str("\n走査は深さ・件数上限で打ち切りました。結果は全件ではありません。");
        }
        if !self.skipped.is_empty() {
            text.push_str(&format!("\n読めずに飛ばした: {}件", self.skipped.len()));
            for (path, reason) in self.skipped.iter().take(3) {
                text.push_str(&format!("\n例: {}（{}）", path.display(), reason));
            }
        }
        text
    }
}

pub fn files(cfg: &Config, root: &SafePath) -> WalkResult {
    files_with_limits(cfg, root, MAX_DEPTH, MAX_SCAN_FILES)
}

fn files_with_limits(
    cfg: &Config,
    root: &SafePath,
    max_depth: usize,
    max_files: usize,
) -> WalkResult {
    let mut walker = Walker {
        cfg,
        root,
        max_depth,
        max_files,
        scanned: 0,
        result: WalkResult::default(),
    };
    walker.scan(root, 0);
    walker.result
}

struct Walker<'a> {
    cfg: &'a Config,
    root: &'a SafePath,
    max_depth: usize,
    max_files: usize,
    scanned: usize,
    result: WalkResult,
}
impl Walker<'_> {
    fn scan(&mut self, dir: &SafePath, depth: usize) {
        if depth > self.max_depth || self.scanned >= self.max_files {
            self.result.truncated = true;
            return;
        }
        let entries = match permissions::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(reason) => {
                self.result
                    .skipped
                    .push((dir.as_path().to_path_buf(), reason));
                return;
            }
        };
        for entry in entries {
            if self.scanned >= self.max_files {
                self.result.truncated = true;
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    self.result
                        .skipped
                        .push((dir.as_path().to_path_buf(), error.to_string()));
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.result.skipped.push((path, error.to_string()));
                    continue;
                }
            };
            let linked = metadata.file_type().is_symlink();
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                linked || metadata.file_attributes() & 0x400 != 0
            };
            if linked {
                continue;
            }
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => {
                    self.result
                        .skipped
                        .push((path, "ファイル名をUTF-8として扱えません".into()));
                    continue;
                }
            };
            if metadata.is_dir() && (name.starts_with('.') || SKIP_DIRS.contains(&name.as_str())) {
                continue;
            }
            let child = match dir.join(self.cfg, &name) {
                Ok(child) => child,
                Err(reason) => {
                    self.result.skipped.push((path, reason));
                    continue;
                }
            };
            if metadata.is_dir() {
                self.scan(&child, depth + 1);
                continue;
            }
            self.scanned += 1;
            if !metadata.is_file() {
                continue;
            }
            let modified = match metadata.modified() {
                Ok(modified) => modified,
                Err(error) => {
                    self.result.skipped.push((path, error.to_string()));
                    continue;
                }
            };
            match path.strip_prefix(self.root.as_path()) {
                Ok(relative) => self.result.files.push(File {
                    relative: relative.to_path_buf(),
                    modified,
                    size: metadata.len(),
                }),
                Err(error) => self.result.skipped.push((path.clone(), error.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 読めないルートと拒否された子の原因を保持する() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &dir.path().to_string_lossy());
        let missing = SafePath::app_owned(dir.path().join("missing"));
        let result = files(&cfg, &missing);
        assert_eq!(result.skipped.len(), 1);
        assert!(result.diagnostic().contains("読めずに飛ばした: 1件"));
        let denied = dir.path().join("secret.txt");
        std::fs::write(&denied, "秘密").unwrap();
        cfg.set("PICOAGENT_DENIED_PATHS", &denied.to_string_lossy());
        let result = files(&cfg, &SafePath::app_owned(dir.path().to_path_buf()));
        assert!(result.files.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].0, denied);
    }
    #[test]
    fn 除外と深さ件数の上限を維持する() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &dir.path().to_string_lossy());
        for path in [
            "a.txt",
            "b.txt",
            "nested/deep/c.txt",
            "node_modules/d.txt",
            ".hidden/e.txt",
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        let root = SafePath::app_owned(dir.path().to_path_buf());
        let complete = files(&cfg, &root);
        assert_eq!(complete.files.len(), 3);
        assert!(!complete.truncated);
        assert!(complete.skipped.is_empty());
        assert!(files_with_limits(&cfg, &root, 0, 100).truncated);
        let limited = files_with_limits(&cfg, &root, 12, 1);
        assert_eq!(limited.files.len(), 1);
        assert!(limited.truncated);
    }
    #[cfg(unix)]
    #[test]
    fn 許可外へのリンクを辿らない() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "秘密").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        let cfg = Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &dir.path().to_string_lossy());
        let result = files(&cfg, &SafePath::app_owned(dir.path().to_path_buf()));
        assert!(result.files.is_empty());
        assert!(!result.truncated);
    }
}
