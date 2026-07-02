// genspark_suite/mod.rs - Genspark風の調査・資料・表・ページ・計画を一括生成する統合スキル。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Local;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;

const MAX_INPUT_CHARS: usize = 32_000;

pub struct GensparkSuiteTool;

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
        "genspark-suite".to_string()
    } else {
        out.chars().take(64).collect()
    }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn clean_input(args: &Value, key: &str, fallback: &str) -> Result<String, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    if text.chars().count() > MAX_INPUT_CHARS {
        return Err(format!(
            "{key} は {MAX_INPUT_CHARS} 文字以内で指定してください。"
        ));
    }
    Ok(text.to_string())
}

fn points(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(12)
        .map(|line| line.trim_start_matches(['-', '*', '・', ' ']).to_string())
        .collect();
    if out.is_empty() {
        out.push(text.chars().take(140).collect());
    }
    out
}

fn csv_cell(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\"").replace('\n', " "))
}

fn write(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|error| format!("{} の保存に失敗しました: {}", path.display(), error))
}

fn deep_research_md(topic: &str, source: &str) -> String {
    let mut out = format!("# Deep Research計画: {topic}\n\n");
    out.push_str("## 調査フェーズ\n\n1. 問いを分解する\n2. 公式情報と一次情報を優先して集める\n3. 数値・日付・固有名詞を照合する\n4. 反対意見と制約を探す\n5. 出典付きで結論をまとめる\n\n## 初期論点\n\n");
    for point in points(source).iter().take(8) {
        out.push_str(&format!("- {point}\n"));
    }
    out.push_str("\n## 出典評価軸\n\n| 軸 | 見ること |\n|---|---|\n| 一次性 | 公式・原典・当事者情報か |\n| 新しさ | 更新日・対象期間が明確か |\n| 利害 | 広告・比較サイト・PRではないか |\n| 再現性 | 同じ主張を別ソースで確認できるか |\n");
    out
}

fn slides_md(topic: &str, source: &str) -> String {
    let pts = points(source);
    let mut out = format!("# AI Slides: {topic}\n\n");
    out.push_str("## 1. タイトル\n\n- 結論を一言で示す\n\n## 2. 背景\n\n- なぜ今このテーマか\n\n## 3. 主要メッセージ\n\n");
    for point in pts.iter().take(4) {
        out.push_str(&format!("- {point}\n"));
    }
    out.push_str("\n## 4. 比較表\n\n| 観点 | 内容 | 補足 |\n|---|---|---|\n");
    for point in pts.iter().take(5) {
        out.push_str(&format!(
            "| 論点 | {} | 根拠確認 |\n",
            point.replace('|', "｜")
        ));
    }
    out.push_str(
        "\n## 5. 次のアクション\n\n- 判断者を決める\n- 不足データを集める\n- 期限を決める\n",
    );
    out
}

fn sheets_csv(source: &str) -> String {
    let mut out = String::from("category,item,score,status,next_action\n");
    for point in points(source).iter().take(20) {
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_cell("論点"),
            csv_cell(point),
            csv_cell("3"),
            csv_cell("未確認"),
            csv_cell("根拠と評価軸を追加")
        ));
    }
    out
}

fn docs_md(topic: &str, audience: &str, source: &str) -> String {
    let mut out = format!("# AI Docs: {topic}\n\n## 読者\n\n{audience}\n\n## 概要\n\n");
    for point in points(source).iter().take(6) {
        out.push_str(&format!("- {point}\n"));
    }
    out.push_str(
        "\n## 本文構成\n\n1. 背景\n2. 課題\n3. 選択肢\n4. 推奨案\n5. リスク\n6. 次のアクション\n",
    );
    out
}

fn sparkpage_html(topic: &str, audience: &str, source: &str) -> String {
    let mut cards = String::new();
    for point in points(source).iter().take(9) {
        cards.push_str(&format!("<article>{}</article>", html_escape(point)));
    }
    format!(
        r#"<!doctype html>
<html lang="ja">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    body {{ margin: 0; font-family: "Yu Gothic UI", Meiryo, sans-serif; color: #202426; background: #f5f7f8; }}
    header {{ padding: 36px 40px 26px; background: #fff; border-bottom: 1px solid #dbe1e4; }}
    main {{ max-width: 1080px; margin: 0 auto; padding: 28px 24px 44px; }}
    h1 {{ margin: 0 0 10px; font-size: 32px; line-height: 1.25; }}
    .meta {{ color: #5c666d; }}
    .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 12px; }}
    article {{ background: white; border: 1px solid #dbe1e4; border-radius: 8px; padding: 16px; line-height: 1.6; }}
  </style>
</head>
<body>
  <header><h1>{title}</h1><div class="meta">想定読者: {audience}</div></header>
  <main><section class="grid">{cards}</section></main>
</body>
</html>"#,
        title = html_escape(topic),
        audience = html_escape(audience),
        cards = cards
    )
}

fn autopilot_md(topic: &str) -> String {
    format!(
        "# Autopilot計画: {topic}\n\n1. 調査を開始する\n2. ソースを評価する\n3. 比較表を更新する\n4. 資料とページを生成する\n5. 未確認事項をTODO化する\n6. 完了時にユーザーへ報告する\n\n## バックグラウンド化候補\n\n- `task_scheduler` で定期調査\n- `research_report` で再調査\n- `spark_workspace` で成果物更新\n"
    )
}

fn drive_index_md(topic: &str) -> String {
    format!(
        "# AI Drive索引: {topic}\n\n| 種類 | ファイル | 用途 |\n|---|---|---|\n| Research | `deep-research.md` | 調査設計 |\n| Slides | `slides.md` | 発表構成 |\n| Sheets | `sheets.csv` | 比較表・採点表 |\n| Docs | `docs.md` | 文書ドラフト |\n| Page | `sparkpage.html` | 共有ページ |\n| Meeting | `meeting-notes.md` | 議事録化 |\n| Agent | `multi-agent.md` | 役割分担 |\n| Browser | `browser-plan.md` | Web操作計画 |\n"
    )
}

fn meeting_notes_md(topic: &str, source: &str) -> String {
    let mut out = format!("# Meeting Notes: {topic}\n\n## 要約\n\n");
    for point in points(source).iter().take(5) {
        out.push_str(&format!("- {point}\n"));
    }
    out.push_str("\n## 決定事項\n\n- 未確定\n\n## TODO\n\n- [ ] 担当者と期限を決める\n- [ ] 根拠資料を確認する\n\n## 共有メール下書き\n\n本日の要点と次のアクションを共有します。詳細は添付の資料一式を確認してください。\n");
    out
}

fn multi_agent_md(topic: &str) -> String {
    format!(
        "# Multi-Agent設計: {topic}\n\n| 役割 | 仕事 | 成果物 |\n|---|---|---|\n| Researcher | 情報収集・出典評価 | deep-research.md |\n| Analyst | 比較表・スコアリング | sheets.csv |\n| Writer | 文書化 | docs.md |\n| Designer | ページ・スライド化 | sparkpage.html / slides.md |\n| Reviewer | リスク・未確認事項確認 | checklist.md |\n"
    )
}

fn browser_plan_md(topic: &str) -> String {
    format!(
        "# AI Browser計画: {topic}\n\n1. 公式サイト・一次情報を開く\n2. 価格・仕様・更新日を抽出する\n3. 比較表へ転記する\n4. 重要ページを保存する\n5. フォーム送信など副作用のある操作はユーザー確認後に実行する\n"
    )
}

fn checklist_md() -> String {
    "- [ ] 出典の一次性を確認した\n- [ ] 最新日付を確認した\n- [ ] 数値と固有名詞を照合した\n- [ ] スライドの結論が先に来ている\n- [ ] 表の評価軸が揃っている\n- [ ] HTMLページを開いて確認した\n".to_string()
}

#[async_trait]
impl Tool for GensparkSuiteTool {
    fn name(&self) -> &'static str {
        "genspark_suite"
    }

    fn label(&self) -> &'static str {
        "Genspark風ワークフロー"
    }

    fn description(&self) -> String {
        "Deep Research、AI Slides、AI Sheets、AI Docs、Sparkpage、Autopilot、AI Drive、Meeting Notes、Multi-Agent、AI Browser計画の10系統を同じフォルダに生成する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string", "description": "テーマ。" },
                "audience": { "type": "string", "description": "想定読者。" },
                "source_text": { "type": "string", "description": "素材メモ、議事録、調査メモ。" },
                "output_dir": { "type": "string", "description": "保存先フォルダ。PICOAGENT_WORKSPACE_ROOT 配下。" }
            },
            "required": ["topic"]
        })
    }


    async fn run(&self, cfg: &Config, args: Value) -> String {
        let topic = match args
            .get("topic")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(topic) => topic,
            None => return "エラー: topic が指定されていません。".to_string(),
        };
        let audience = args
            .get("audience")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("一般読者");
        let source = match clean_input(&args, "source_text", topic) {
            Ok(source) => source,
            Err(error) => return format!("エラー: {error}"),
        };
        let base = workspace_root(cfg);
        let default_dir = format!(
            "genspark-suite/{}-{}",
            Local::now().format("%Y%m%d-%H%M%S"),
            slug(topic)
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
            ("README.md", drive_index_md(topic)),
            ("deep-research.md", deep_research_md(topic, &source)),
            ("slides.md", slides_md(topic, &source)),
            ("sheets.csv", sheets_csv(&source)),
            ("docs.md", docs_md(topic, audience, &source)),
            ("sparkpage.html", sparkpage_html(topic, audience, &source)),
            ("autopilot.md", autopilot_md(topic)),
            ("meeting-notes.md", meeting_notes_md(topic, &source)),
            ("multi-agent.md", multi_agent_md(topic)),
            ("browser-plan.md", browser_plan_md(topic)),
            ("checklist.md", checklist_md()),
        ];
        let mut saved = Vec::new();
        for (name, content) in files {
            let path = dir.join(name);
            if let Err(error) = write(&path, &content) {
                return format!("エラー: {error}");
            }
            saved.push(path);
        }
        let list = saved
            .iter()
            .map(|path| format!("- {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Genspark風ワークフロー成果物を生成しました:\n{list}")
    }
}

inventory::submit! { ToolReg { make: || Box::new(GensparkSuiteTool) } }

#[cfg(test)]
mod tests {
    use super::{html_escape, sheets_csv, slug};

    #[test]
    fn slugを生成する() {
        assert_eq!(slug("Genspark Suite / AI"), "genspark-suite-ai");
    }

    #[test]
    fn htmlをエスケープする() {
        assert_eq!(html_escape("<x&y>"), "&lt;x&amp;y&gt;");
    }

    #[test]
    fn sheets_csvを生成する() {
        let csv = sheets_csv("A\nB");
        assert!(csv.contains("\"A\""));
        assert!(csv.contains("\"B\""));
    }
}
