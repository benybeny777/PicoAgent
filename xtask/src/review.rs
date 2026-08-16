// review.rs - 「main へ入った変更は書いた側の相手がレビューする」クロスチェック運用の管理。
//
// レビューは指示があるときだけ行うと抜ける。抜けたことに気づく手段が無いと、
// 実際に「起動を止めない」修正が会話履歴を消す状態のまま数日残った（`ac04622`）。
// レビュー済みかどうかの正本は `docs/REVIEW-LOG.md` に書いてある記録そのものとし、
// ここではその記録と `git log` を突き合わせて、まだ誰も見ていないコミットを出す。
//
// **求めるのは書いた本人以外のレビュー**。自分の書いた変更を自分で見ても、同じ前提の
// まま同じ見落としが残る（それを防ぐためのクロスチェックなので、そこを数えない）。
// 作成者が分からないコミットは、どちらが書いたか決められないので両方を求める。
//
// 記録側の書き方（どちらも1行、見出しの直下）:
//   対象コミット: d371577, ac04622 ｜ レビュアー: claude
//   作成コミット: e9bbd7f ｜ 作成者: claude
//
// 作成者は、squash merge で GitHub アカウントへ付け替わると git からは分からなくなるため
// 台帳へ書く。git の author が Claude のコミットだけは自動で判別する。
//
// 起点（ここより前は個別記録を要求しない）:
//   レビュー必須の起点: ac04622

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::util::{log, repo_root};

/// レビューを担当する作業エージェント。書いた本人以外がそろって「レビュー済み」とする。
pub const REVIEWERS: [&str; 2] = ["codex", "claude"];

const LOG_PATH: &str = "docs/REVIEW-LOG.md";
const BASELINE_MARKER: &str = "レビュー必須の起点:";
const TARGET_MARKER: &str = "対象コミット:";
const REVIEWER_MARKER: &str = "レビュアー:";
const AUTHOR_TARGET_MARKER: &str = "作成コミット:";
const AUTHOR_MARKER: &str = "作成者:";
/// Claude Code が作ったコミットの git author（squash 前のコミットで残る）。
const CLAUDE_AUTHOR_MAIL: &str = "noreply@anthropic.com";

/// レビュー対象のコミット1件。
struct Commit {
    sha: String,
    date: String,
    subject: String,
    /// git の author から分かる作成者（判別できるのは Claude だけ）。
    author: Option<String>,
}

impl Commit {
    /// squash merge の件名末尾に残る `(#92)` から PR 番号を取る。
    /// PR 経由が必須なので、レビュー相手へ渡す入口はコミットではなく PR の差分になる。
    fn pull_request(&self) -> Option<u32> {
        let tail = self.subject.rsplit_once("(#")?.1;
        tail.strip_suffix(')')?.parse().ok()
    }
}

/// `origin` の URL から `owner/repo` を取る（PR の URL を組み立てるため）。
fn repo_slug() -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo_root())
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let slug = url
        .rsplit_once("github.com")
        .map(|(_, rest)| rest.trim_start_matches([':', '/']).to_string())?;
    Some(slug.trim_end_matches(".git").to_string())
}

/// レビュー記録から読み取った「どのコミットを誰が見たか」。
pub struct Coverage {
    /// この起点より後のコミットが個別記録の対象。
    pub baseline: String,
    /// コミット（短縮 SHA）→ レビューした人の一覧。
    pub reviewed: BTreeMap<String, Vec<String>>,
    /// コミット（短縮 SHA）→ 書いた人（台帳に記録があるぶん）。
    pub authors: BTreeMap<String, String>,
}

impl Coverage {
    /// 台帳に記録された作成者。
    fn author(&self, commit: &str) -> Option<&str> {
        self.authors
            .iter()
            .find(|(sha, _)| same_commit(sha, commit))
            .map(|(_, author)| author.as_str())
    }

    /// そのコミットのレビューがまだ足りない人。
    /// `author` は git から分かった作成者（台帳の記録があればそちらを優先する）。
    pub fn missing(&self, commit: &str, author: Option<&str>) -> Vec<&'static str> {
        let author = self.author(commit).or(author);
        REVIEWERS
            .iter()
            // 書いた本人は数えない。同じ前提のまま読み直しても見落としは同じ場所に残る。
            .filter(|reviewer| author != Some(**reviewer))
            .filter(|reviewer| {
                !self.reviewed.iter().any(|(sha, reviewers)| {
                    same_commit(sha, commit) && reviewers.iter().any(|done| done == *reviewer)
                })
            })
            .copied()
            .collect()
    }
}

/// 短縮 SHA どうしの比較（記録側と `git log` 側で桁数が違ってよい）。
fn same_commit(left: &str, right: &str) -> bool {
    let (left, right) = (left.to_lowercase(), right.to_lowercase());
    !left.is_empty() && (left.starts_with(&right) || right.starts_with(&left))
}

/// 台帳の行だけを拾う。**行頭が印であること**を要求する（この文書自身の説明文で
/// 印を引用しても、記録として数えないため）。先頭の箇条書き `- ` は許す。
fn entry<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let line = line.trim_start().trim_start_matches("- ").trim_start();
    line.starts_with(marker).then(|| after(line, marker))?
}

fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = line.split_once(marker)?.1;
    // 同じ行に別の項目が続く（`… ｜ レビュアー: …`）ので、区切りまでを見る。
    Some(rest.split(['｜', '|']).next().unwrap_or(rest).trim())
}

/// 16進以外は区切りとして扱い、短縮 SHA らしい語だけを拾う
/// （`d371577, \`91ea586\`` も `ac04622（注記）` も同じように読める）。
fn shas(raw: &str) -> Vec<String> {
    raw.split(|c: char| !c.is_ascii_hexdigit())
        .map(str::to_lowercase)
        .filter(|token| (7..=40).contains(&token.len()))
        .collect()
}

/// レビュアー名は先頭の英字の並びだけを見る（`claude（指摘0件）` のような注記を許す）。
fn reviewer_name(raw: &str) -> String {
    raw.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_lowercase()
}

/// レビュー記録の本文を読み取る。
pub fn coverage(text: &str) -> Result<Coverage> {
    let mut baseline = String::new();
    let mut reviewed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut authors: BTreeMap<String, String> = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        if baseline.is_empty() {
            if let Some(raw) = entry(line, BASELINE_MARKER) {
                baseline = shas(raw).first().cloned().unwrap_or_default();
            }
        }
        if let Some(targets) = entry(line, AUTHOR_TARGET_MARKER) {
            let author = after(line, AUTHOR_MARKER)
                .map(reviewer_name)
                .unwrap_or_default();
            if !REVIEWERS.contains(&author.as_str()) {
                bail!(
                    "{LOG_PATH}:{} の `{AUTHOR_MARKER}` は {} のいずれかにしてください",
                    number + 1,
                    REVIEWERS.join(" / ")
                );
            }
            let targets = shas(targets);
            if targets.is_empty() {
                bail!(
                    "{LOG_PATH}:{} の `{AUTHOR_TARGET_MARKER}` にコミットがありません",
                    number + 1
                );
            }
            for target in targets {
                authors.insert(target, author.clone());
            }
            continue;
        }
        let Some(targets) = entry(line, TARGET_MARKER) else {
            continue;
        };
        let Some(reviewer) = after(line, REVIEWER_MARKER).map(reviewer_name) else {
            bail!(
                "{LOG_PATH}:{} に `{REVIEWER_MARKER}` がありません（例: 対象コミット: d371577 ｜ レビュアー: claude）",
                number + 1
            );
        };
        if !REVIEWERS.contains(&reviewer.as_str()) {
            bail!(
                "{LOG_PATH}:{} のレビュアー `{reviewer}` は未知です（{}）",
                number + 1,
                REVIEWERS.join(" / ")
            );
        }
        let targets = shas(targets);
        if targets.is_empty() {
            bail!(
                "{LOG_PATH}:{} の `{TARGET_MARKER}` にコミットがありません",
                number + 1
            );
        }
        for target in targets {
            reviewed.entry(target).or_default().push(reviewer.clone());
        }
    }
    if baseline.is_empty() {
        bail!("{LOG_PATH} に `{BASELINE_MARKER} <コミット>` がありません");
    }
    Ok(Coverage {
        baseline,
        reviewed,
        authors,
    })
}

/// 数える対象の枝。既定は「`main` へ入った変更」なので `origin/main` を見る。
/// squash merge の後は作業ブランチ側と `main` 側で SHA が変わるため、HEAD を見ると
/// マージ済みの作業コミットが未レビューとして残り続ける。
fn target_ref(args: &[String]) -> String {
    if let Some(index) = args.iter().position(|arg| arg == "--base") {
        if let Some(base) = args.get(index + 1) {
            return base.clone();
        }
    }
    for candidate in ["origin/main", "main", "HEAD"] {
        let resolved = Command::new("git")
            .current_dir(repo_root())
            .args(["rev-parse", "--verify", "--quiet", candidate])
            .output();
        if resolved.is_ok_and(|output| output.status.success()) {
            return candidate.to_string();
        }
    }
    "HEAD".to_string()
}

/// 起点より後のコミット（新しい順）。日付は「いつから待たせているか」を出すため。
fn commits(baseline: &str, target: &str) -> Result<Vec<Commit>> {
    let output = Command::new("git")
        .current_dir(repo_root())
        .args([
            "log",
            "--date=short",
            "--format=%h\t%ad\t%ae\t%s",
            &format!("{baseline}..{target}"),
        ])
        .output()
        .context("git log を実行できません")?;
    if !output.status.success() {
        bail!(
            "git log に失敗しました（起点 {baseline} はこの履歴にありますか）: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let sha = columns.next()?.trim().to_string();
            let date = columns.next()?.trim().to_string();
            let mail = columns.next()?.trim().to_lowercase();
            Some(Commit {
                sha,
                date,
                subject: columns.next()?.trim().to_string(),
                // squash merge で GitHub アカウントへ付け替わると判別できない。
                // 分からない場合は台帳の `作成者:` に任せる。
                author: mail
                    .ends_with(CLAUDE_AUTHOR_MAIL)
                    .then(|| "claude".to_string()),
            })
        })
        .collect())
}

/// 未レビューのコミットを一覧表示する。`--strict` は1件でもあれば失敗させる。
pub fn status(args: &[String]) -> Result<()> {
    let strict = args.iter().any(|arg| arg == "--strict");
    let text = std::fs::read_to_string(repo_root().join(LOG_PATH))
        .with_context(|| format!("{LOG_PATH} を読めません"))?;
    let coverage = coverage(&text)?;
    let target = target_ref(args);
    let commits = commits(&coverage.baseline, &target)?;
    let mut pending = Vec::new();
    for commit in &commits {
        let missing = coverage.missing(&commit.sha, commit.author.as_deref());
        if !missing.is_empty() {
            pending.push((commit, missing));
        }
    }
    log(
        "review-status",
        format!(
            "{target}: 起点 {} 以降 {} 件中、未レビュー {} 件",
            coverage.baseline,
            commits.len(),
            pending.len()
        ),
    );
    let slug = repo_slug();
    for (commit, missing) in &pending {
        println!(
            "  {} {} {}（未: {}）",
            commit.date,
            commit.sha,
            commit.subject,
            missing.join(", ")
        );
        // レビューは PR の差分で行う運用なので、開く先をそのまま出す。
        if let (Some(number), Some(slug)) = (commit.pull_request(), slug.as_ref()) {
            println!("    https://github.com/{slug}/pull/{number}");
        }
    }
    if pending.is_empty() {
        log("review-status", "全コミットを Codex と Claude が確認済み");
    } else {
        log(
            "review-status",
            format!(
                "レビュー後は {LOG_PATH} へ `{TARGET_MARKER} <SHA> ｜ {REVIEWER_MARKER} <{}>` を残すこと",
                REVIEWERS.join("/")
            ),
        );
    }
    if strict && !pending.is_empty() {
        bail!("未レビューのコミットが {} 件あります", pending.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{coverage, same_commit};

    const SAMPLE: &str = "# レビュー記録\n\
        - 書き方の説明: `対象コミット: <SHA> ｜ レビュアー: codex` を見出しの直下へ置く\n\
        - レビュー必須の起点: ac04622（これより前は個別記録を求めない）\n\
        \n\
        ## 2026-08-16 — 例\n\
        作成コミット: 7654321 ｜ 作成者: claude\n\
        対象コミット: d371577, `91ea586` ｜ レビュアー: claude\n\
        対象コミット: d371577 ｜ レビュアー: codex（指摘0件）\n";

    #[test]
    fn 起点とレビュー済みコミットを読み取る() {
        let coverage = coverage(SAMPLE).expect("読み取れること");
        assert_eq!(coverage.baseline, "ac04622");
        assert!(
            coverage.missing("d371577", None).is_empty(),
            "両者が確認済み"
        );
        assert_eq!(coverage.missing("91ea586", None), vec!["codex"]);
        assert_eq!(coverage.missing("1234567", None), vec!["codex", "claude"]);
    }

    #[test]
    fn 書いた本人のレビューは求めない() {
        let coverage = coverage(SAMPLE).expect("読み取れること");
        // 台帳に作成者があるコミットは、相手のレビューだけが残件。
        assert_eq!(coverage.missing("7654321", None), vec!["codex"]);
        // 台帳に無い場合は git の author から渡された作成者を使う。
        assert_eq!(coverage.missing("1234567", Some("claude")), vec!["codex"]);
        assert_eq!(coverage.missing("1234567", Some("codex")), vec!["claude"]);
        // 作成者が分からなければ従来どおり両方を求める。
        assert_eq!(coverage.missing("1234567", None), vec!["codex", "claude"]);
    }

    #[test]
    fn 未知の作成者名は失敗させる() {
        assert!(
            coverage("レビュー必須の起点: ac04622\n作成コミット: 1234567 ｜ 作成者: 誰か\n")
                .is_err()
        );
        assert!(
            coverage("レビュー必須の起点: ac04622\n作成コミット: ｜ 作成者: claude\n").is_err()
        );
    }

    #[test]
    fn 説明文の中の印は記録として数えない() {
        // 「行頭が印」だけを記録とする。台帳の書き方を説明する行で数えると、
        // 何もレビューしていないのに全件レビュー済みに見える。
        let coverage = coverage(SAMPLE).expect("読み取れること");
        assert_eq!(
            coverage.reviewed.len(),
            2,
            "説明行の SHA まで拾っています: {:?}",
            coverage.reviewed
        );
    }

    #[test]
    fn squashコミットの件名からpr番号を取る() {
        let commit = |subject: &str| super::Commit {
            sha: "abc1234".into(),
            date: "2026-08-16".into(),
            subject: subject.into(),
            author: None,
        };
        assert_eq!(
            commit("docs: レビュー必須の起点を変更する (#92)").pull_request(),
            Some(92)
        );
        // PR を通さず直接コミットしたものは番号が付かない。
        assert_eq!(
            commit("docs: 起動ブロッカー調査結果を記録").pull_request(),
            None
        );
        assert_eq!(commit("fix: 課題(#123)への対応の話").pull_request(), None);
    }

    #[test]
    fn 桁数の違う短縮shaを同じコミットとして扱う() {
        assert!(same_commit("d371577", "d3715770c73"));
        assert!(!same_commit("d371577", "d371578"));
        assert!(!same_commit("", "d371577"));
    }

    #[test]
    fn 起点の無い記録とレビュアー名の誤りは失敗させる() {
        assert!(coverage("対象コミット: d371577 ｜ レビュアー: claude\n").is_err());
        assert!(coverage(
            "レビュー必須の起点: ac04622\n対象コミット: d371577 ｜ レビュアー: gpt\n"
        )
        .is_err());
        assert!(
            coverage("レビュー必須の起点: ac04622\n対象コミット: ｜ レビュアー: claude\n").is_err()
        );
    }
}
