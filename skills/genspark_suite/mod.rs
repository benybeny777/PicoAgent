// Genspark資料生成。モードごとの差を宣言し、素材処理と保存は共通経路を通す。
use super::{Tool, ToolReg};
use crate::{
    config::Config,
    text::{slug, SlugPolicy},
};
use async_trait::async_trait;
use chrono::Local;
use serde_json::{json, Value};
mod documents;
use documents::{Documents, Mode};

pub struct GensparkSuiteTool;
const FULL_KEYWORDS: &[&str] = &[
    "genspark全部",
    "genspark風",
    "deep research",
    "ai slides",
    "ai sheets",
    "ai docs",
    "sparkpage",
    "autopilot",
    "meeting notes",
    "multi-agent",
    "ai browser",
];
const LIGHT_KEYWORDS: &[&str] = &[
    "genspark",
    "資料一式",
    "ワークスペース",
    "スライドとサイト",
    "サイトとスライド",
    "ページと資料",
    "spark",
    "deck",
    "brief",
];
const STRING_KEYS: [&str; 5] = ["topic", "audience", "goal", "source_text", "output_dir"];
fn optional_string<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(format!("{key} は文字列で指定してください。")),
    }
}
fn string_arguments(args: &Value) -> Result<[Option<&str>; 5], String> {
    let mut values = [None; 5];
    for (index, key) in STRING_KEYS.iter().enumerate() {
        values[index] = optional_string(args, key)?;
    }
    Ok(values)
}
fn nonempty<'a>(value: Option<&'a str>, default: &'a str) -> &'a str {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
}
#[async_trait]
impl Tool for GensparkSuiteTool {
    fn name(&self) -> &'static str {
        "genspark_suite"
    }
    fn intent_keywords(&self) -> &'static [&'static str] {
        static KEYWORDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
        KEYWORDS.get_or_init(|| {
            FULL_KEYWORDS
                .iter()
                .chain(LIGHT_KEYWORDS)
                .copied()
                .chain(["全部やって"])
                .collect()
        })
    }
    fn intent_priority(&self) -> i32 {
        700
    }
    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        let lower = text.to_lowercase();
        let full = FULL_KEYWORDS.iter().any(|word| lower.contains(word))
            || (lower.contains("genspark") && text.contains("全部"));
        let light = LIGHT_KEYWORDS.iter().any(|word| lower.contains(word));
        if !full && !light {
            return None;
        }
        let mut args = json!({"topic":text.trim(),"audience":"一般読者","source_text":text.trim(),"mode":if full {"full"} else {"light"}});
        if !full {
            args["goal"] = json!("資料、スライド、表、共有ページ、実行計画をまとめて作る");
        }
        Some(crate::tools::IntentInvocation::direct(args))
    }
    fn label(&self) -> &'static str {
        "Genspark風ワークフロー"
    }
    fn description(&self) -> String {
        "fullは調査・スライド・表・文書・共有ページ・自動実行・索引・議事録・役割分担・ブラウザー計画の11ファイル、lightは概要・スライド・表・共有ページ・実行計画・プロンプト・チェックリスト・索引の8ファイルを同じフォルダに生成する。省略時はfull。".into()
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "topic":{"type":"string","description":"テーマまたは資料タイトル。"},
            "mode":{"type":"string","enum":["full","light"],"default":"full","description":"fullは11ファイル、lightは8ファイル。"},
            "goal":{"type":"string","description":"lightで生成する成果物の目的。"},
            "audience":{"type":"string","description":"想定読者。省略時は一般読者。"},
            "source_text":{"type":"string","description":"素材メモ、議事録、調査メモ。fullは32000文字、lightは24000文字以内。"},
            "output_dir":{"type":"string","description":"保存先フォルダ。共通許可ルート配下。"}},"required":["topic"]})
    }
    async fn run(&self, cfg: &Config, args: Value) -> String {
        let mode = match Mode::parse(&args) {
            Ok(mode) => mode,
            Err(reason) => return format!("エラー: {reason}"),
        };
        let [topic, audience, goal, _, output_dir] = match string_arguments(&args) {
            Ok(values) => values,
            Err(reason) => return format!("エラー: {reason}"),
        };
        let topic = nonempty(topic, "");
        if topic.is_empty() {
            return "エラー: topic が指定されていません。".into();
        }
        let audience = nonempty(audience, "一般読者");
        let goal = nonempty(goal, "調査・資料化・共有までを一気通貫で進める");
        let now = Local::now();
        let date = now.format("%Y-%m-%d").to_string();
        let documents = match Documents::new(mode, topic, goal, audience, &date, &args) {
            Ok(documents) => documents,
            Err(reason) => return format!("エラー: {reason}"),
        };
        // 不正モード・素材超過を全て検査し、生成も終えてからディレクトリを作る。
        let files = documents.files();
        let spec = mode.spec();
        let default_dir = format!(
            "{}/{}-{}",
            spec.prefix,
            now.format("%Y%m%d-%H%M%S"),
            slug(topic, spec.slug_fallback, 64, SlugPolicy::AsciiOutput)
        );
        let dir = match crate::permissions::resolve(cfg, nonempty(output_dir, &default_dir)) {
            Ok(dir) => dir,
            Err(reason) => return format!("エラー: {reason}"),
        };
        if let Err(error) = std::fs::create_dir_all(&dir) {
            return format!("エラー: 保存先フォルダを作成できません: {error}");
        }
        let mut saved = Vec::new();
        for (name, content) in files {
            let path = match dir.join(cfg, name) {
                Ok(path) => path,
                Err(reason) => return format!("エラー: {reason}"),
            };
            if let Err(reason) = crate::permissions::fs::write(&path, &content) {
                return format!("エラー: {reason}");
            }
            saved.push(format!("- {}", path.display()));
        }
        format!("{}:\n{}", spec.result, saved.join("\n"))
    }
}
inventory::submit! { ToolReg { make: || Box::new(GensparkSuiteTool) } }

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", dir.path().to_str().unwrap());
        (dir, cfg)
    }
    #[test]
    fn 全機能語を優先し旧資料依頼はlightへ割り当てる() {
        for (text, mode) in [
            ("Gensparkみたいに資料一式を作って", "light"),
            ("Genspark風の機能を全部やって", "full"),
            ("資料一式とAI Slidesを作って", "full"),
        ] {
            assert_eq!(
                GensparkSuiteTool.intent_invocation(text).unwrap().args["mode"],
                mode
            );
        }
        assert!(GensparkSuiteTool
            .intent_invocation("天気を教えて")
            .is_none());
    }
    #[tokio::test]
    async fn 両モードの本体が既存ファイルを保ち8件と11件を保存する() {
        let (dir, cfg) = fixture();
        for (mode, count, expected, absent) in [
            ("light", 8, "brief.md", "deep-research.md"),
            ("full", 11, "deep-research.md", "brief.md"),
        ] {
            let target = dir.path().join(mode);
            std::fs::create_dir(&target).unwrap();
            std::fs::write(target.join("利用者メモ.txt"), "保存済み").unwrap();
            let result=GensparkSuiteTool.run(&cfg,json!({"mode":mode,"topic":"資料{goal}","goal":"目的","audience":"<読者>","source_text":"- A\nB\"C","output_dir":mode})).await;
            assert!(!result.starts_with("エラー:"), "{result}");
            assert_eq!(std::fs::read_dir(&target).unwrap().count(), count + 1);
            assert!(target.join(expected).is_file());
            assert!(!target.join(absent).exists());
            assert_eq!(
                std::fs::read_to_string(target.join("利用者メモ.txt")).unwrap(),
                "保存済み"
            );
            let page = if mode == "light" {
                "page.html"
            } else {
                "sparkpage.html"
            };
            let html = std::fs::read_to_string(target.join(page)).unwrap();
            assert!(html.contains("&lt;読者&gt;"));
            assert!(html.contains("資料{goal}"));
        }
    }
    #[tokio::test]
    async fn 不正モードと型と素材超過は保存前に止まる() {
        let (dir, cfg) = fixture();
        for mode in ["full", "light"] {
            for key in STRING_KEYS {
                for invalid in [json!(null), json!(true), json!(3), json!([]), json!({})] {
                    let mut args = json!({"mode":mode,"topic":"資料","output_dir":"invalid-type","未知項目":{"保持":true}});
                    args[key] = invalid;
                    let result = GensparkSuiteTool.run(&cfg, args).await;
                    assert!(
                        result.contains(&format!("{key} は文字列")),
                        "{mode}/{key}: {result}"
                    );
                    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
                }
            }
        }
        for mode in [json!("unknown"), json!(null), json!(1)] {
            let result = GensparkSuiteTool
                .run(
                    &cfg,
                    json!({"mode":mode,"topic":"資料","output_dir":"invalid"}),
                )
                .await;
            assert!(result.contains("mode は full または light"));
            assert!(!dir.path().join("invalid").exists());
        }
        for (mode, limit) in [("light", 24_000), ("full", 32_000)] {
            let target = format!("over-{mode}");
            let result=GensparkSuiteTool.run(&cfg,json!({"mode":mode,"topic":"資料","source_text":"字".repeat(limit+1),"output_dir":target})).await;
            assert!(result.contains(&limit.to_string()), "{result}");
            assert!(!dir.path().join(target).exists());
        }
    }
    #[tokio::test]
    async fn 省略はfullで既定保存先はモードごとに維持する() {
        let (dir, cfg) = fixture();
        for mode in [None, Some("light")] {
            let mut args = json!({"topic":"日本語テーマ"});
            if let Some(mode) = mode {
                args["mode"] = json!(mode);
            }
            let result = GensparkSuiteTool.run(&cfg, args).await;
            assert!(!result.starts_with("エラー:"), "{result}");
            let prefix = if mode.is_some() {
                "spark-workspaces"
            } else {
                "genspark-suite"
            };
            let target = std::fs::read_dir(dir.path().join(prefix))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            assert_eq!(
                std::fs::read_dir(target).unwrap().count(),
                if mode.is_some() { 8 } else { 11 }
            );
        }
    }
}
