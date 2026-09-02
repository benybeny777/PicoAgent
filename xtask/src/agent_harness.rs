//! 作業エージェント用スキルの正本と各クライアント入口の同期を検査する。

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

pub fn check(root: &Path) -> Result<()> {
    let shared_root = root.join(".agents/skills");
    let claude_root = root.join(".claude/skills");
    let shared = skill_names(&shared_root)?;
    let claude = skill_names(&claude_root)?;
    let mut issues = Vec::new();

    for name in shared.difference(&claude) {
        issues.push(format!(".claude/skills/{name}/SKILL.md がありません"));
    }
    for name in claude.difference(&shared) {
        issues.push(format!(".agents/skills/{name}/SKILL.md がありません"));
    }

    for name in &shared {
        validate_shared_skill(&shared_root.join(name).join("SKILL.md"), name, &mut issues);
    }
    for name in &claude {
        validate_claude_entry(&claude_root.join(name).join("SKILL.md"), name, &mut issues);
    }

    if issues.is_empty() {
        Ok(())
    } else {
        bail!(issues.join("\n"))
    }
}

fn skill_names(root: &Path) -> Result<BTreeSet<String>> {
    let entries = fs::read_dir(root).with_context(|| format!("{} を読めません", root.display()))?;
    let mut names = BTreeSet::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("{} の列挙に失敗しました", root.display()))?;
        if entry
            .file_type()
            .with_context(|| format!("{} の種類を取得できません", entry.path().display()))?
            .is_dir()
        {
            names.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

fn validate_shared_skill(path: &Path, name: &str, issues: &mut Vec<String>) {
    let Some(text) = read_skill(path, issues) else {
        return;
    };
    validate_frontmatter(path, name, &text, issues);
}

fn validate_claude_entry(path: &Path, name: &str, issues: &mut Vec<String>) {
    let Some(text) = read_skill(path, issues) else {
        return;
    };
    let body_start = validate_frontmatter(path, name, &text, issues);
    let expected_link = format!(".agents/skills/{name}/SKILL.md");
    if !text.contains(&expected_link) {
        issues.push(format!(
            "{} が共通正本 `{expected_link}` を参照していません",
            path.display()
        ));
    }
    if !text.contains("手順を複製しない") {
        issues.push(format!(
            "{} に手順を複製しない指示がありません",
            path.display()
        ));
    }
    if let Some(body_start) = body_start {
        let body_lines = text[body_start..]
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        if body_lines > 2 {
            issues.push(format!(
                "{} の本文が{body_lines}行あります。Claude Code用自動認識ファイルへ手順を複製せず、見出しと参照指示だけにしてください",
                path.display()
            ));
        }
    }
}

fn read_skill(path: &Path, issues: &mut Vec<String>) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) => {
            issues.push(format!("{} を読めません: {error}", path.display()));
            None
        }
    }
}

fn validate_frontmatter(
    path: &Path,
    expected_name: &str,
    text: &str,
    issues: &mut Vec<String>,
) -> Option<usize> {
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        issues.push(format!("{} が空です", path.display()));
        return None;
    };
    offset += first.len();
    if first.trim_end() != "---" {
        issues.push(format!("{} にfrontmatterがありません", path.display()));
        return None;
    }

    let mut name = None;
    let mut has_description = false;
    for line in lines {
        offset += line.len();
        let trimmed = line.trim();
        if trimmed == "---" {
            if name.as_deref() != Some(expected_name) {
                issues.push(format!(
                    "{} のnameがフォルダ名 `{expected_name}` と一致しません",
                    path.display()
                ));
            }
            if !has_description {
                issues.push(format!("{} のdescriptionが空です", path.display()));
            }
            return Some(offset);
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            has_description = !value.trim().is_empty();
        }
    }

    issues.push(format!("{} のfrontmatterが閉じていません", path.display()));
    None
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::check;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "picoagent-agent-harness-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_skill(root: &std::path::Path, base: &str, name: &str, body: &str) {
        let dir = root.join(base).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn 共通正本と薄い自動認識ファイルの対応を受理する() {
        let root = temp_root("valid");
        write_skill(
            &root,
            ".agents/skills",
            "sample-skill",
            "---\nname: sample-skill\ndescription: 検証用。\n---\n\n# 手順\n",
        );
        write_skill(
            &root,
            ".claude/skills",
            "sample-skill",
            "---\nname: sample-skill\ndescription: 検証用。\n---\n\n# 共通スキル正本の案内\n\n`.agents/skills/sample-skill/SKILL.md`を読み、手順を複製しない。\n",
        );

        let result = check(&root);
        fs::remove_dir_all(&root).unwrap();
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn 名前ずれと自動認識ファイルへの手順複製を拒否する() {
        let root = temp_root("invalid");
        fs::create_dir_all(root.join(".claude/skills")).unwrap();
        write_skill(
            &root,
            ".agents/skills",
            "sample-skill",
            "---\nname: sample-skill\ndescription: 検証用。\n---\n",
        );
        write_skill(
            &root,
            ".claude/skills",
            "extra-skill",
            "# 手順\n\n独自の手順です。\n",
        );

        let error = check(&root).unwrap_err().to_string();
        fs::remove_dir_all(&root).unwrap();
        assert!(error.contains(".claude/skills/sample-skill/SKILL.md がありません"));
        assert!(error.contains(".agents/skills/extra-skill/SKILL.md がありません"));
        assert!(error.contains("frontmatterがありません"));
    }
}
