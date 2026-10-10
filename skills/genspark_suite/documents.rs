// documents.rs - 素材の整形と成果物生成の正本。構成の違いは宣言で表す。
use crate::text::html_escape;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Full,
    Light,
}
pub(super) struct ModeSpec {
    pub limit: usize,
    pub prefix: &'static str,
    pub slug_fallback: &'static str,
    points: usize,
    empty_chars: usize,
    strip_bullets: bool,
    pub result: &'static str,
}
impl Mode {
    pub fn parse(args: &Value) -> Result<Self, String> {
        match args.get("mode") {
            None => Ok(Self::Full),
            Some(Value::String(mode)) if mode == "full" => Ok(Self::Full),
            Some(Value::String(mode)) if mode == "light" => Ok(Self::Light),
            _ => Err("mode は full または light で指定してください。".into()),
        }
    }
    pub fn spec(self) -> ModeSpec {
        match self {
            Self::Full => ModeSpec {
                limit: 32_000,
                prefix: "genspark-suite",
                slug_fallback: "genspark-suite",
                points: 12,
                empty_chars: 140,
                strip_bullets: true,
                result: "Genspark風ワークフロー成果物を生成しました",
            },
            Self::Light => ModeSpec {
                limit: 24_000,
                prefix: "spark-workspaces",
                slug_fallback: "spark-workspace",
                points: 10,
                empty_chars: 120,
                strip_bullets: false,
                result: "資料ワークスペースを作成しました",
            },
        }
    }
}
pub(super) struct Documents<'a> {
    mode: Mode,
    topic: &'a str,
    goal: &'a str,
    audience: &'a str,
    date: &'a str,
    points: Vec<String>,
}
impl<'a> Documents<'a> {
    pub fn new(
        mode: Mode,
        topic: &'a str,
        goal: &'a str,
        audience: &'a str,
        date: &'a str,
        args: &Value,
    ) -> Result<Self, String> {
        let spec = mode.spec();
        let source = super::optional_string(args, "source_text")?
            .unwrap_or(topic)
            .trim();
        // 省略時の素材はテーマ。full の空素材だけは既存の宣言に従ってテーマを使う。
        let source = if mode == Mode::Full && source.is_empty() {
            topic
        } else {
            source
        };
        if source.chars().count() > spec.limit {
            return Err(format!(
                "source_text は {} 文字以内で指定してください。",
                spec.limit
            ));
        }
        let mut points: Vec<String> = source
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(spec.points)
            .map(|line| {
                if spec.strip_bullets {
                    line.trim_start_matches(['-', '*', '・', ' ']).to_owned()
                } else {
                    line.to_owned()
                }
            })
            .collect();
        if points.is_empty() {
            points.push(source.chars().take(spec.empty_chars).collect());
        }
        Ok(Self {
            mode,
            topic,
            goal,
            audience,
            date,
            points,
        })
    }
    fn expand(&self, template: &str) -> String {
        // 置換後の利用者文字列を再置換しない。
        let mut out = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            let Some(end) = tail.find('}') else {
                out.push_str(tail);
                return out;
            };
            let token = &tail[..=end];
            out.push_str(match token {
                "{topic}" | "{project}" => self.topic,
                "{goal}" => self.goal,
                "{audience}" => self.audience,
                "{date}" => self.date,
                _ => token,
            });
            rest = &tail[end + 1..];
        }
        out.push_str(rest);
        out
    }
    fn markdown(&self, template: &str, take: usize) -> String {
        let (before, after) = template.split_once("{points}").unwrap_or((template, ""));
        let mut out = self.expand(before);
        if template.contains("{points}") {
            for point in self.points.iter().take(take) {
                out.push_str(&format!("- {point}\n"));
            }
        }
        out.push_str(&self.expand(after));
        out
    }
    fn deck(&self) -> String {
        let (heading,intro,take,table,confirmation,actions)=match self.mode {
            Mode::Full => ("AI Slides", "## 1. タイトル\n\n- 結論を一言で示す\n\n## 2. 背景\n\n- なぜ今このテーマか\n\n",4,"比較表\n\n| 観点 | 内容 | 補足 |","根拠確認","- 判断者を決める\n- 不足データを集める\n- 期限を決める\n"),
            Mode::Light => ("スライド構成案","## 1. タイトル\n\n- {topic}\n- {goal}\n\n## 2. 背景\n\n- 対象: {audience}\n- この資料で判断したいことを明示する。\n\n",3,"比較・論点\n\n| 観点 | 内容 | 次の確認 |","根拠を確認","- 担当者を決める。\n- 期限を決める。\n- 不足データを追加する。\n"),
        };
        let mut out = format!("# {heading}: {}\n\n{}", self.topic, self.expand(intro));
        out.push_str(&self.markdown("## 3. 主要メッセージ\n\n{points}", take));
        out.push_str(&format!("\n## 4. {table}\n|---|---|---|\n"));
        for point in self.points.iter().take(5) {
            out.push_str(&format!(
                "| 論点 | {} | {confirmation} |\n",
                point.replace('|', "｜")
            ));
        }
        out.push_str(&format!("\n## 5. 次のアクション\n\n{actions}"));
        out
    }
    fn csv(&self) -> String {
        let (header, fixed) = match self.mode {
            Mode::Full => (
                "category,item,score,status,next_action\n",
                vec!["論点", "3", "未確認", "根拠と評価軸を追加"],
            ),
            Mode::Light => (
                "type,item,status,next_action\n",
                vec!["論点", "未確認", "根拠を追加確認"],
            ),
        };
        let mut out = header.to_owned();
        for point in &self.points {
            let mut cells = fixed.clone();
            cells.insert(1, point);
            out.push_str(
                &cells
                    .into_iter()
                    .map(|cell| format!("\"{}\"", cell.replace('"', "\"\"").replace('\n', " ")))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push('\n');
        }
        out
    }
    fn html(&self) -> String {
        let light = self.mode == Mode::Light;
        let (
            color,
            border,
            width,
            header_pad,
            title_margin,
            card_pad,
            line_height,
            meta_css,
            section_css,
            take,
        ) = if light {
            (
                "#202427",
                "#d8dee2",
                1040,
                24,
                12,
                15,
                "1.55",
                "display:flex;gap:14px;flex-wrap:wrap;color:#59636a;font-size:13px;",
                "section { margin-top:18px; }",
                8,
            )
        } else {
            (
                "#202426",
                "#dbe1e4",
                1080,
                26,
                10,
                16,
                "1.6",
                "color:#5c666d;",
                "",
                9,
            )
        };
        let title = html_escape(self.topic);
        let audience = html_escape(self.audience);
        let meta = if light {
            format!(
                "<span>目的: {}</span><span>読者: {audience}</span>",
                html_escape(self.goal)
            )
        } else {
            format!("想定読者: {audience}")
        };
        let cards = self
            .points
            .iter()
            .take(take)
            .map(|point| format!("<article>{}</article>", html_escape(point)))
            .collect::<String>();
        let body = if light {
            format!("<section><h2>主要ポイント</h2><div class=\"grid\">{cards}</div></section><section><h2>次のアクション</h2><div class=\"grid\"><article>根拠確認</article><article>資料調整</article><article>共有・レビュー</article></div></section>")
        } else {
            format!("<section class=\"grid\">{cards}</section>")
        };
        format!(
            r#"<!doctype html>
<html lang="ja"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title>
<style>
body {{ margin:0;font-family:"Yu Gothic UI",Meiryo,sans-serif;color:{color};background:#f5f7f8; }}
header {{ padding:36px 40px {header_pad}px;background:#fff;border-bottom:1px solid {border}; }}
main {{ max-width:{width}px;margin:0 auto;padding:28px 24px 44px; }}
h1 {{ margin:0 0 {title_margin}px;font-size:32px;line-height:1.25; }}
.meta {{ {meta_css} }} {section_css}
.grid {{ display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:12px; }}
article {{ background:#fff;border:1px solid {border};border-radius:8px;padding:{card_pad}px;line-height:{line_height}; }}
</style></head><body><header><h1>{title}</h1><div class="meta">{meta}</div></header><main>{body}</main></body></html>"#
        )
    }
    pub fn files(&self) -> Vec<(&'static str, String)> {
        use Content::*;
        let specs: &[(&str, Content)] = match self.mode {
            Mode::Full => FULL,
            Mode::Light => LIGHT,
        };
        specs
            .iter()
            .map(|(name, content)| {
                (
                    *name,
                    match content {
                        Markdown(template, take) => self.markdown(template, *take),
                        Deck => self.deck(),
                        Csv => self.csv(),
                        Html => self.html(),
                    },
                )
            })
            .collect()
    }
}
enum Content {
    Markdown(&'static str, usize),
    Deck,
    Csv,
    Html,
}
const DEEP_RESEARCH_MD: &str = "# Deep Research計画: {topic}\n\n## 調査フェーズ\n\n1. 問いを分解する\n2. 公式情報と一次情報を優先して集める\n3. 数値・日付・固有名詞を照合する\n4. 反対意見と制約を探す\n5. 出典付きで結論をまとめる\n\n## 初期論点\n\n{points}\n## 出典評価軸\n\n| 軸 | 見ること |\n|---|---|\n| 一次性 | 公式・原典・当事者情報か |\n| 新しさ | 更新日・対象期間が明確か |\n| 利害 | 広告・比較サイト・PRではないか |\n| 再現性 | 同じ主張を別ソースで確認できるか |\n";
const DOCS_MD: &str = "# AI Docs: {topic}\n\n## 読者\n\n{audience}\n\n## 概要\n\n{points}\n## 本文構成\n\n1. 背景\n2. 課題\n3. 選択肢\n4. 推奨案\n5. リスク\n6. 次のアクション\n";
const MEETING_NOTES_MD: &str = "# Meeting Notes: {topic}\n\n## 要約\n\n{points}\n## 決定事項\n\n- 未確定\n\n## TODO\n\n- [ ] 担当者と期限を決める\n- [ ] 根拠資料を確認する\n\n## 共有メール下書き\n\n本日の要点と次のアクションを共有します。詳細は添付の資料一式を確認してください。\n";
const BRIEF: &str = "# 1ページ概要: {project}\n\n## 目的\n\n{goal}\n\n## 想定読者\n\n{audience}\n\n## 要点\n\n{points}\n## 推奨する次の一手\n\n- 重要な主張を一次情報で確認する。\n- 共有相手に合わせてスライド枚数と表現を調整する。\n- `page.html` を開き、全体の流れを確認する。\n";
const AUTOPILOT_MD: &str = "# Autopilot計画: {topic}\n\n1. 調査を開始する\n2. ソースを評価する\n3. 比較表を更新する\n4. 資料とページを生成する\n5. 未確認事項をTODO化する\n6. 完了時にユーザーへ報告する\n\n## バックグラウンド化候補\n\n- `task_scheduler` で定期調査\n- `research_report` で再調査\n- `genspark_suite` の light モードで成果物更新\n";
const DRIVE_INDEX_MD: &str = "# AI Drive索引: {topic}\n\n| 種類 | ファイル | 用途 |\n|---|---|---|\n| Research | `deep-research.md` | 調査設計 |\n| Slides | `slides.md` | 発表構成 |\n| Sheets | `sheets.csv` | 比較表・採点表 |\n| Docs | `docs.md` | 文書ドラフト |\n| Page | `sparkpage.html` | 共有ページ |\n| Meeting | `meeting-notes.md` | 議事録化 |\n| Agent | `multi-agent.md` | 役割分担 |\n| Browser | `browser-plan.md` | Web操作計画 |\n";
const MULTI_AGENT_MD: &str = "# Multi-Agent設計: {topic}\n\n| 役割 | 仕事 | 成果物 |\n|---|---|---|\n| Researcher | 情報収集・出典評価 | deep-research.md |\n| Analyst | 比較表・スコアリング | sheets.csv |\n| Writer | 文書化 | docs.md |\n| Designer | ページ・スライド化 | sparkpage.html / slides.md |\n| Reviewer | リスク・未確認事項確認 | checklist.md |\n";
const BROWSER_PLAN_MD: &str = "# AI Browser計画: {topic}\n\n1. 公式サイト・一次情報を開く\n2. 価格・仕様・更新日を抽出する\n3. 比較表へ転記する\n4. 重要ページを保存する\n5. フォーム送信など副作用のある操作はユーザー確認後に実行する\n";
const CHECKLIST_MD: &str = "- [ ] 出典の一次性を確認した\n- [ ] 最新日付を確認した\n- [ ] 数値と固有名詞を照合した\n- [ ] スライドの結論が先に来ている\n- [ ] 表の評価軸が揃っている\n- [ ] HTMLページを開いて確認した\n";
const ARTIFACT_INDEX: &str = "# {topic}\n\n- 目的: {goal}\n- 想定読者: {audience}\n- 生成日: {date}\n\n## 成果物\n\n- `brief.md`: 1ページ概要\n- `deck.md`: スライド構成案\n- `sheet.csv`: 表計算用の論点表\n- `page.html`: 共有用の単体HTMLページ\n- `agent-plan.md`: 自律実行ステップ\n- `prompts.md`: 再生成・深掘り用プロンプト集\n- `checklist.md`: 公開前チェックリスト\n";
const AGENT_PLAN: &str = "# エージェント実行計画: {topic}\n\n1. 目的を確認する: {goal}\n2. 入力素材から主要論点を抽出する。\n3. 足りない根拠を調査する。\n4. `brief.md` と `deck.md` を更新する。\n5. `sheet.csv` に比較軸を追加する。\n6. `page.html` を共有前に確認する。\n7. 公開前チェックリストを消化する。\n";
const PROMPTS: &str = "# 再生成・深掘りプロンプト集: {topic}\n\n- この資料を経営層向けに短くしてください。目的: {goal}\n- このスライド構成を10枚のピッチデックにしてください。\n- `sheet.csv` の論点に評価軸とスコアを追加してください。\n- `page.html` をより営業資料らしい文体にしてください。\n- リスクと未確認事項だけを洗い出してください。\n";
const CHECKLIST: &str = "- [ ] 数値・日付・固有名詞を確認した\n- [ ] 共有相手に不要な内部情報を削った\n- [ ] スライドの結論が先に来ている\n- [ ] 表の評価軸が揃っている\n- [ ] HTMLページをブラウザで開いて確認した\n";
const FULL: &[(&str, Content)] = &[
    ("README.md", Content::Markdown(DRIVE_INDEX_MD, 0)),
    ("deep-research.md", Content::Markdown(DEEP_RESEARCH_MD, 8)),
    ("slides.md", Content::Deck),
    ("sheets.csv", Content::Csv),
    ("docs.md", Content::Markdown(DOCS_MD, 6)),
    ("sparkpage.html", Content::Html),
    ("autopilot.md", Content::Markdown(AUTOPILOT_MD, 0)),
    ("meeting-notes.md", Content::Markdown(MEETING_NOTES_MD, 5)),
    ("multi-agent.md", Content::Markdown(MULTI_AGENT_MD, 0)),
    ("browser-plan.md", Content::Markdown(BROWSER_PLAN_MD, 0)),
    ("checklist.md", Content::Markdown(CHECKLIST_MD, 0)),
];
const LIGHT: &[(&str, Content)] = &[
    ("README.md", Content::Markdown(ARTIFACT_INDEX, 0)),
    ("brief.md", Content::Markdown(BRIEF, 5)),
    ("deck.md", Content::Deck),
    ("sheet.csv", Content::Csv),
    ("page.html", Content::Html),
    ("agent-plan.md", Content::Markdown(AGENT_PLAN, 0)),
    ("prompts.md", Content::Markdown(PROMPTS, 0)),
    ("checklist.md", Content::Markdown(CHECKLIST, 0)),
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 素材整形はモードごとの論点数と空文字の扱いを保つ() {
        assert!(Documents::new(
            Mode::Full,
            "テーマ",
            "目的",
            "読者",
            "日付",
            &json!({"source_text":false})
        )
        .err()
        .unwrap()
        .contains("source_text は文字列"));
        let source = (0..14)
            .map(|n| format!("- 論点{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        for (mode, count, first) in [(Mode::Full, 12, "論点0"), (Mode::Light, 10, "- 論点0")] {
            let documents = Documents::new(
                mode,
                "テーマ",
                "目的",
                "読者",
                "2026-10-11",
                &json!({"source_text":source}),
            )
            .unwrap();
            assert_eq!(documents.points.len(), count);
            assert_eq!(documents.points[0], first);
            let csv = documents.csv();
            assert_eq!(csv.lines().count(), count + 1);
            let at_limit = json!({"source_text":"字".repeat(mode.spec().limit)});
            assert!(Documents::new(mode, "テーマ", "目的", "読者", "日付", &at_limit).is_ok());
        }
        for (mode, point) in [(Mode::Full, "テーマ"), (Mode::Light, "")] {
            let documents = Documents::new(
                mode,
                "テーマ",
                "目的",
                "読者",
                "日付",
                &json!({"source_text":"  "}),
            )
            .unwrap();
            assert_eq!(documents.points, [point]);
        }
    }

    #[test]
    fn 共有テンプレートは利用者内の置換語と各形式の構造を保つ() {
        for mode in [Mode::Full, Mode::Light] {
            let documents = Documents::new(
                mode,
                "資料{goal}",
                "目的",
                "読者{topic}",
                "2026-10-11",
                &json!({"source_text":"A\"B|<C>\n二番"}),
            )
            .unwrap();
            let files = documents.files();
            let csv = files
                .iter()
                .find(|(name, _)| name.ends_with(".csv"))
                .unwrap()
                .1
                .as_str();
            assert!(csv.contains("\"A\"\"B|<C>\""));
            let deck = documents.deck();
            assert!(deck.contains("A\"B｜<C>"));
            assert!(deck.contains("資料{goal}"));
            let html = documents.html();
            assert!(html.contains("A&quot;B|&lt;C&gt;"));
            assert!(html.contains("読者{topic}"));
            if mode == Mode::Light {
                assert!(html.contains("次のアクション"));
                assert!(files[0].1.contains("生成日: 2026-10-11"));
            } else {
                assert!(!html.contains("次のアクション"));
            }
        }
    }
}
