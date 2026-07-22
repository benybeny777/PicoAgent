// spark_workspace/mod.rs - 資料・ページ・表・実行計画をまとめて作るワークスペース生成スキル。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Local;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;

const MAX_TEXT_CHARS: usize = 24_000;

pub struct SparkWorkspaceTool;

fn workspace_root(cfg: &Config) -> PathBuf {
    crate::permissions::primary_root(cfg)
}

fn normalize(base: &Path, input: &str) -> PathBuf {
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

fn bounded_path(base: &Path, path: &str) -> Result<PathBuf, String> {
    let target = normalize(base, path);
    if target == base || target.starts_with(base) {
        Ok(target)
    } else {
        Err(format!(
            "許可ルート（{}）の外には保存できません: {}",
            base.display(),
            target.display()
        ))
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "spark-workspace".to_string()
    } else {
        out.chars().take(64).collect()
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn clean_text(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.chars().count() > MAX_TEXT_CHARS {
        return Err(format!(
            "入力は {} 文字以内で指定してください。",
            MAX_TEXT_CHARS
        ));
    }
    Ok(trimmed.to_string())
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|error| format!("{} の保存に失敗しました: {}", path.display(), error))
}

fn bullets(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(10)
        .map(ToString::to_string)
        .collect();
    if out.is_empty() {
        out.push(text.chars().take(120).collect());
    }
    out
}

fn artifact_index(project: &str, goal: &str, audience: &str) -> String {
    format!(
        "# {project}\n\n- 目的: {goal}\n- 想定読者: {audience}\n- 生成日: {}\n\n## 成果物\n\n- `brief.md`: 1ページ概要\n- `deck.md`: スライド構成案\n- `sheet.csv`: 表計算用の論点表\n- `page.html`: 共有用の単体HTMLページ\n- `agent-plan.md`: 自律実行ステップ\n- `prompts.md`: 再生成・深掘り用プロンプト集\n- `checklist.md`: 公開前チェックリスト\n",
        Local::now().format("%Y-%m-%d")
    )
}

fn brief(project: &str, goal: &str, audience: &str, source: &str) -> String {
    let mut out = format!("# 1ページ概要: {project}\n\n");
    out.push_str(&format!(
        "## 目的\n\n{goal}\n\n## 想定読者\n\n{audience}\n\n"
    ));
    out.push_str("## 要点\n\n");
    for item in bullets(source).iter().take(5) {
        out.push_str(&format!("- {item}\n"));
    }
    out.push_str("\n## 推奨する次の一手\n\n- 重要な主張を一次情報で確認する。\n- 共有相手に合わせてスライド枚数と表現を調整する。\n- `page.html` を開き、全体の流れを確認する。\n");
    out
}

fn deck(project: &str, goal: &str, audience: &str, source: &str) -> String {
    let points = bullets(source);
    let mut out = format!("# スライド構成案: {project}\n\n");
    out.push_str("## 1. タイトル\n\n");
    out.push_str(&format!("- {project}\n- {goal}\n\n"));
    out.push_str("## 2. 背景\n\n");
    out.push_str(&format!(
        "- 対象: {audience}\n- この資料で判断したいことを明示する。\n\n"
    ));
    out.push_str("## 3. 主要メッセージ\n\n");
    for point in points.iter().take(3) {
        out.push_str(&format!("- {point}\n"));
    }
    out.push_str("\n## 4. 比較・論点\n\n| 観点 | 内容 | 次の確認 |\n|---|---|---|\n");
    for point in points.iter().take(5) {
        out.push_str(&format!(
            "| 論点 | {} | 根拠を確認 |\n",
            point.replace('|', "｜")
        ));
    }
    out.push_str("\n## 5. 次のアクション\n\n- 担当者を決める。\n- 期限を決める。\n- 不足データを追加する。\n");
    out
}

fn sheet_csv(source: &str) -> String {
    let mut out = String::from("type,item,status,next_action\n");
    for point in bullets(source).iter().take(12) {
        out.push_str(&format!(
            "\"論点\",\"{}\",\"未確認\",\"根拠を追加確認\"\n",
            point.replace('"', "\"\"")
        ));
    }
    out
}

fn page_html(project: &str, goal: &str, audience: &str, source: &str) -> String {
    let mut point_cards = String::new();
    for point in bullets(source).iter().take(8) {
        point_cards.push_str(&format!("<article>{}</article>", html_escape(point)));
    }
    format!(
        r#"<!doctype html>
<html lang="ja">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    body {{ margin: 0; font-family: "Yu Gothic UI", Meiryo, sans-serif; color: #202427; background: #f5f7f8; }}
    header {{ padding: 36px 40px 24px; background: #ffffff; border-bottom: 1px solid #d8dee2; }}
    main {{ max-width: 1040px; margin: 0 auto; padding: 28px 24px 44px; }}
    h1 {{ margin: 0 0 12px; font-size: 32px; line-height: 1.25; }}
    .meta {{ display: flex; gap: 14px; flex-wrap: wrap; color: #59636a; font-size: 13px; }}
    section {{ margin-top: 18px; }}
    .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 12px; }}
    article {{ background: #fff; border: 1px solid #d8dee2; border-radius: 8px; padding: 15px; line-height: 1.55; }}
  </style>
</head>
<body>
  <header>
    <h1>{title}</h1>
    <div class="meta"><span>目的: {goal}</span><span>読者: {audience}</span></div>
  </header>
  <main>
    <section><h2>主要ポイント</h2><div class="grid">{point_cards}</div></section>
    <section><h2>次のアクション</h2><div class="grid"><article>根拠確認</article><article>資料調整</article><article>共有・レビュー</article></div></section>
  </main>
</body>
</html>"#,
        title = html_escape(project),
        goal = html_escape(goal),
        audience = html_escape(audience),
        point_cards = point_cards
    )
}

fn agent_plan(project: &str, goal: &str) -> String {
    format!(
        "# エージェント実行計画: {project}\n\n1. 目的を確認する: {goal}\n2. 入力素材から主要論点を抽出する。\n3. 足りない根拠を調査する。\n4. `brief.md` と `deck.md` を更新する。\n5. `sheet.csv` に比較軸を追加する。\n6. `page.html` を共有前に確認する。\n7. 公開前チェックリストを消化する。\n"
    )
}

fn prompts(project: &str, goal: &str) -> String {
    format!(
        "# 再生成・深掘りプロンプト集: {project}\n\n- この資料を経営層向けに短くしてください。目的: {goal}\n- このスライド構成を10枚のピッチデックにしてください。\n- `sheet.csv` の論点に評価軸とスコアを追加してください。\n- `page.html` をより営業資料らしい文体にしてください。\n- リスクと未確認事項だけを洗い出してください。\n"
    )
}

fn checklist() -> String {
    "- [ ] 数値・日付・固有名詞を確認した\n- [ ] 共有相手に不要な内部情報を削った\n- [ ] スライドの結論が先に来ている\n- [ ] 表の評価軸が揃っている\n- [ ] HTMLページをブラウザで開いて確認した\n".to_string()
}

#[async_trait]
impl Tool for SparkWorkspaceTool {
    fn name(&self) -> &'static str {
        "spark_workspace"
    }

    fn label(&self) -> &'static str {
        "資料ワークスペース生成"
    }

    fn description(&self) -> String {
        "ひとつの依頼から brief.md、deck.md、sheet.csv、page.html、agent-plan.md、prompts.md、checklist.md を含む資料ワークスペースを作る。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "プロジェクト名または資料タイトル。" },
                "goal": { "type": "string", "description": "成果物の目的。" },
                "audience": { "type": "string", "description": "想定読者。省略時は一般読者。" },
                "source_text": { "type": "string", "description": "素材メモ、調査メモ、箇条書き。" },
                "output_dir": { "type": "string", "description": "保存先フォルダ。共通許可ルート配下。" }
            },
            "required": ["project"]
        })
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let project = match args
            .get("project")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(project) => project,
            None => return "エラー: project が指定されていません。".to_string(),
        };
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("調査・資料化・共有までを一気通貫で進める");
        let audience = args
            .get("audience")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("一般読者");
        let source = match clean_text(
            args.get("source_text")
                .and_then(Value::as_str)
                .unwrap_or(project),
        ) {
            Ok(source) => source,
            Err(error) => return format!("エラー: {error}"),
        };
        let base = workspace_root(cfg);
        let default_dir = format!(
            "spark-workspaces/{}-{}",
            Local::now().format("%Y%m%d-%H%M%S"),
            slug(project)
        );
        let dir_text = args
            .get("output_dir")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&default_dir);
        let dir = match bounded_path(&base, dir_text) {
            Ok(dir) => dir,
            Err(error) => return format!("エラー: {error}"),
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            return format!("エラー: 保存先フォルダを作成できません: {error}");
        }
        let files = [
            ("README.md", artifact_index(project, goal, audience)),
            ("brief.md", brief(project, goal, audience, &source)),
            ("deck.md", deck(project, goal, audience, &source)),
            ("sheet.csv", sheet_csv(&source)),
            ("page.html", page_html(project, goal, audience, &source)),
            ("agent-plan.md", agent_plan(project, goal)),
            ("prompts.md", prompts(project, goal)),
            ("checklist.md", checklist()),
        ];
        let mut saved = Vec::new();
        for (name, content) in files {
            let path = dir.join(name);
            if let Err(error) = write_file(&path, &content) {
                return format!("エラー: {error}");
            }
            saved.push(path);
        }
        let list = saved
            .iter()
            .map(|path| format!("- {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n");
        format!("資料ワークスペースを作成しました:\n{list}")
    }
}

inventory::submit! { ToolReg { make: || Box::new(SparkWorkspaceTool) } }

#[cfg(test)]
mod tests {
    use super::{html_escape, sheet_csv, slug};

    #[test]
    fn slugを生成する() {
        assert_eq!(slug("AI Workspace / Genspark"), "ai-workspace-genspark");
    }

    #[test]
    fn htmlをエスケープする() {
        assert_eq!(html_escape("<a&b>"), "&lt;a&amp;b&gt;");
    }

    #[test]
    fn sheet_csvを生成する() {
        let csv = sheet_csv("A\nB");
        assert!(csv.contains("\"A\""));
        assert!(csv.contains("\"B\""));
    }
}
