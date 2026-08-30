//! PR前のローカル検証を、途中の失敗で打ち切らずに一括実行する。

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Result};

use crate::agent_harness;
use crate::util::repo_root;

#[derive(Clone, Copy)]
struct Check {
    label: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

const CHECKS: &[Check] = &[
    Check {
        label: "本体の書式",
        program: "cargo",
        args: &["fmt", "--manifest-path", "src-tauri/Cargo.toml", "--check"],
    },
    Check {
        label: "xtaskの書式",
        program: "cargo",
        args: &["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"],
    },
    Check {
        label: "本体の静的解析",
        program: "cargo",
        args: &[
            "clippy",
            "--manifest-path",
            "src-tauri/Cargo.toml",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    },
    Check {
        label: "xtaskの静的解析",
        program: "cargo",
        args: &[
            "clippy",
            "--manifest-path",
            "xtask/Cargo.toml",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    },
    Check {
        label: "本体のlibテスト",
        program: "cargo",
        args: &["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib"],
    },
    Check {
        label: "xtaskのテスト",
        program: "cargo",
        args: &["test", "--manifest-path", "xtask/Cargo.toml"],
    },
];

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed(String),
    Skipped(String),
}

enum NodeAvailability {
    Available,
    Missing,
    Unusable(Option<i32>),
}

#[derive(Default)]
struct Report {
    passed: Vec<String>,
    failed: Vec<(String, String)>,
    skipped: Vec<(String, String)>,
}

impl Report {
    fn record(&mut self, label: impl Into<String>, outcome: Outcome) {
        let label = label.into();
        match outcome {
            Outcome::Passed => self.passed.push(label),
            Outcome::Failed(reason) => self.failed.push((label, reason)),
            Outcome::Skipped(reason) => self.skipped.push((label, reason)),
        }
    }

    fn print_and_finish(self) -> Result<()> {
        println!("\n[verify] 結果: 成功 {}件", self.passed.len());
        if !self.skipped.is_empty() {
            println!("[verify] スキップ {}件:", self.skipped.len());
            for (label, reason) in &self.skipped {
                println!("  - {label}: {reason}");
            }
        }
        if self.failed.is_empty() {
            println!("[verify] すべての必須検証に成功しました");
            return Ok(());
        }

        eprintln!("[verify] 失敗 {}件:", self.failed.len());
        for (label, reason) in &self.failed {
            eprintln!("  - {label}: {reason}");
        }
        bail!("必須検証が{}件失敗しました", self.failed.len())
    }
}

pub fn run() -> Result<()> {
    let root = repo_root();
    let mut report = execute_checks(CHECKS, |check| run_process(&root, check));
    check_skill_format(&root, &mut report);
    report.record(
        "共通スキル正本とClaude Code用自動認識ファイル",
        match agent_harness::check(&root) {
            Ok(()) => Outcome::Passed,
            Err(error) => Outcome::Failed(error.to_string()),
        },
    );
    check_javascript(&root, &mut report);
    report.record(
        "Git差分検査",
        run_process(
            &root,
            &Check {
                label: "Git差分検査",
                program: "git",
                args: &["diff", "--check"],
            },
        ),
    );
    report.print_and_finish()
}

fn execute_checks<F>(checks: &[Check], mut execute: F) -> Report
where
    F: FnMut(&Check) -> Outcome,
{
    let mut report = Report::default();
    for check in checks {
        report.record(check.label, execute(check));
    }
    report
}

fn run_process(root: &Path, check: &Check) -> Outcome {
    println!("\n[verify] 開始: {}", check.label);
    match Command::new(check.program)
        .args(check.args)
        .current_dir(root)
        .status()
    {
        Ok(status) if status.success() => Outcome::Passed,
        Ok(status) => Outcome::Failed(format!("終了コード {:?}", status.code())),
        Err(error) => Outcome::Failed(format!("起動できません: {error}")),
    }
}

/// スキルコネクタの書式を検査する。
///
/// `cargo fmt` は `mod` 宣言をたどってファイルを集めるが、`skills/<名前>/mod.rs` は
/// `build.rs` が生成した `#[path] pub mod` 経由でしか繋がっておらず、rustfmt からは
/// 見えない。そのため**本体の書式検査はスキルを1ファイルも見ていなかった**。
/// ここで追跡中のスキルソースを直接 rustfmt へ渡し、範囲の穴を塞ぐ。
fn check_skill_format(root: &Path, report: &mut Report) {
    const LABEL: &str = "スキルの書式";
    let files = match tracked_files(root, &["skills/*.rs"]) {
        Ok(files) => files,
        Err(reason) => {
            report.record(LABEL, Outcome::Failed(reason));
            return;
        }
    };
    if files.is_empty() {
        report.record(
            LABEL,
            Outcome::Failed("スキルのソースが1件も見つかりません".to_string()),
        );
        return;
    }
    println!("\n[verify] 開始: {LABEL}（{}件）", files.len());
    let outcome = match Command::new("rustfmt")
        .args(["--edition", "2021", "--check"])
        .args(&files)
        .current_dir(root)
        .status()
    {
        Ok(status) if status.success() => Outcome::Passed,
        Ok(status) => Outcome::Failed(format!(
            "終了コード {:?}（`rustfmt --edition 2021 skills/**/*.rs` で整形する）",
            status.code()
        )),
        Err(error) => Outcome::Failed(format!("rustfmtを起動できません: {error}")),
    };
    report.record(LABEL, outcome);
}

/// Git が追跡しているパスを取得する。空白や日本語を含むパスも壊さないよう NUL 区切りで読む。
fn tracked_files(root: &Path, patterns: &[&str]) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(["ls-files", "-z", "--"])
        .args(patterns)
        .current_dir(root)
        .output()
        .map_err(|error| format!("gitを起動できません: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(str::to_string)
                .map_err(|error| format!("UTF-8でない追跡パスがあります: {error}"))
        })
        .collect()
}

fn check_javascript(root: &Path, report: &mut Report) {
    match node_availability(root) {
        Ok(NodeAvailability::Available) => {}
        Ok(NodeAvailability::Missing) => {
            report.record(
                "JavaScript／MJS構文",
                Outcome::Skipped("nodeが見つからないため".to_string()),
            );
            return;
        }
        Ok(NodeAvailability::Unusable(code)) => {
            report.record(
                "JavaScript／MJS構文",
                Outcome::Failed(format!(
                    "node --versionが失敗しました（終了コード {code:?}）"
                )),
            );
            return;
        }
        Err(error) => {
            report.record(
                "JavaScript／MJS構文",
                Outcome::Failed(format!("nodeの確認に失敗しました: {error}")),
            );
            return;
        }
    }

    let files = match tracked_files(root, &["*.js", "*.mjs"]) {
        Ok(files) => files,
        Err(reason) => {
            report.record("JavaScript／MJS対象列挙", Outcome::Failed(reason));
            return;
        }
    };

    for path in files {
        let label = format!("JavaScript構文: {path}");
        println!("\n[verify] 開始: {label}");
        let outcome = match Command::new("node")
            .args(["--check", &path])
            .current_dir(root)
            .status()
        {
            Ok(status) if status.success() => Outcome::Passed,
            Ok(status) => Outcome::Failed(format!("終了コード {:?}", status.code())),
            Err(error) => Outcome::Failed(format!("nodeを起動できません: {error}")),
        };
        report.record(label, outcome);
    }
}

fn node_availability(root: &Path) -> io::Result<NodeAvailability> {
    match Command::new("node")
        .arg("--version")
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(NodeAvailability::Available),
        Ok(status) => Ok(NodeAvailability::Unusable(status.code())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(NodeAvailability::Missing),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{execute_checks, Check, Outcome, Report};

    const TEST_CHECKS: &[Check] = &[
        Check {
            label: "1",
            program: "unused",
            args: &[],
        },
        Check {
            label: "2",
            program: "unused",
            args: &[],
        },
        Check {
            label: "3",
            program: "unused",
            args: &[],
        },
    ];

    #[test]
    fn 途中の失敗後も残りの検証を実行する() {
        let mut executed = Vec::new();
        let report = execute_checks(TEST_CHECKS, |check| {
            executed.push(check.label);
            if check.label == "2" {
                Outcome::Failed("意図した失敗".to_string())
            } else {
                Outcome::Passed
            }
        });

        assert_eq!(executed, ["1", "2", "3"]);
        assert_eq!(report.passed, ["1", "3"]);
        assert_eq!(
            report.failed,
            [("2".to_string(), "意図した失敗".to_string())]
        );
    }

    #[test]
    fn スキップは必須検証の失敗に数えない() {
        let mut report = Report::default();
        report.record("JavaScript", Outcome::Skipped("nodeなし".to_string()));

        assert!(report.failed.is_empty());
        assert_eq!(
            report.skipped,
            [("JavaScript".to_string(), "nodeなし".to_string())]
        );
    }
}
