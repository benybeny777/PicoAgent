// diagnostics.rs - 不具合の連絡へ添えてもらう診断情報を1つの文章にまとめる。
//
// 版・OS・選択中のAI・設定・直近のログを利用者が手で集めると、毎回「どの版ですか」
// 「ログを見せてください」の往復になる。ここで1ファイルへまとめる。
//
// **秘密値は絶対に本文へ入れない。** APIキー等は `secret::is_secret_key()` で判定し、
// 設定済みかどうかだけを出す（保護保存された文字列もそのままでは意味を持たないが、
// 送り先が増える経路には載せない）。

use serde_json::{Map, Value};

/// ログ末尾として載せる行数と、読み取るバイト数の上限。
pub const LOG_TAIL_LINES: usize = 200;
pub const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// 本文の組み立てに必要な、呼び出し側が集めた事実。
pub struct Report<'a> {
    pub generated_at: String,
    pub version: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub beta_build: bool,
    pub config: &'a Map<String, Value>,
    pub log_tail: &'a str,
}

fn display_value(key: &str, value: &Value) -> String {
    if crate::secret::is_secret_key(key) {
        // 値そのものは出さない。設定済みかどうかだけ分かれば、こちらの切り分けには足りる。
        let filled = match value {
            Value::String(text) => !text.trim().is_empty(),
            Value::Null => false,
            _ => true,
        };
        return if filled {
            "（設定済み）".to_string()
        } else {
            "（未設定）".to_string()
        };
    }
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn selected(config: &Map<String, Value>, key: &str, fallback: &str) -> String {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// 診断情報の本文を組み立てる。ファイル入出力は呼び出し側が行う。
pub fn build_report(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("# PicoAgent 診断情報\n\n");
    out.push_str(&format!("生成日時: {}\n", report.generated_at));
    out.push_str(&format!("版: {}\n", report.version));
    out.push_str(&format!("環境: {} / {}\n", report.os, report.arch));
    out.push_str(&format!(
        "ビルド: {}\n",
        if report.beta_build {
            "アルファ版（beta-full）"
        } else {
            "通常"
        }
    ));

    out.push_str("\n## 選択中\n");
    out.push_str(&format!(
        "AI: {}\n",
        selected(report.config, "PICOAGENT_PROVIDER", "local")
    ));
    out.push_str(&format!(
        "モデル: {}\n",
        selected(report.config, "PICOAGENT_MODEL", "（既定）")
    ));
    out.push_str(&format!(
        "表示形式: {}\n",
        selected(report.config, "PICOAGENT_LAYOUT", "pet")
    ));
    out.push_str(&format!(
        "キャラクター: {}\n",
        selected(report.config, "PICOAGENT_CHARACTER", "mugi")
    ));
    out.push_str(&format!(
        "読み上げ: {}\n",
        selected(report.config, "PICOAGENT_TTS_ENABLED", "false")
    ));

    out.push_str("\n## 設定（APIキー等の秘密値は値を伏せています）\n");
    let mut keys: Vec<&String> = report.config.keys().collect();
    keys.sort();
    for key in keys {
        let value = report.config.get(key).unwrap_or(&Value::Null);
        out.push_str(&format!("{key} = {}\n", display_value(key, value)));
    }

    out.push_str(&format!("\n## ログ（app.log の末尾{LOG_TAIL_LINES}行）\n"));
    if report.log_tail.trim().is_empty() {
        out.push_str("（まだ記録がありません）\n");
    } else {
        out.push_str(report.log_tail);
        if !report.log_tail.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_config() -> Map<String, Value> {
        json!({
            "PICOAGENT_PROVIDER": "openai",
            "PICOAGENT_MODEL": "gpt-5",
            "PICOAGENT_CHARACTER": "rin",
            "PICOAGENT_OPENAI_API_KEY": "sk-live-should-never-appear",
            "PICOAGENT_BLUESKY_APP_PASSWORD": "dpapi:v1:should-never-appear",
            "PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN": "",
            "PICOAGENT_TTS_ENABLED": true
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    fn sample_report<'a>(config: &'a Map<String, Value>, log: &'a str) -> Report<'a> {
        Report {
            generated_at: "2026-09-02 08:00:00".to_string(),
            version: "1.0.4",
            os: "windows",
            arch: "x86_64",
            beta_build: true,
            config,
            log_tail: log,
        }
    }

    #[test]
    fn 秘密値は本文へ出さない() {
        let config = sample_config();
        let text = build_report(&sample_report(&config, "[08:00:00] 起動しました\n"));
        assert!(!text.contains("sk-live-should-never-appear"));
        assert!(!text.contains("dpapi:v1:should-never-appear"));
        // 設定済みかどうかだけは分かるようにする（切り分けに要るため）。
        assert!(text.contains("PICOAGENT_OPENAI_API_KEY = （設定済み）"));
        assert!(text.contains("PICOAGENT_BLUESKY_APP_PASSWORD = （設定済み）"));
        assert!(text.contains("PICOAGENT_GOOGLE_SHEETS_ACCESS_TOKEN = （未設定）"));
    }

    #[test]
    fn 版と環境と選択中のプロバイダを載せる() {
        let config = sample_config();
        let text = build_report(&sample_report(&config, ""));
        assert!(text.contains("版: 1.0.4"));
        assert!(text.contains("環境: windows / x86_64"));
        assert!(text.contains("ビルド: アルファ版"));
        assert!(text.contains("AI: openai"));
        assert!(text.contains("モデル: gpt-5"));
        assert!(text.contains("キャラクター: rin"));
        // 秘密でない値はそのまま読める。
        assert!(text.contains("PICOAGENT_PROVIDER = openai"));
        // ログが無くても本文は完成する。
        assert!(text.contains("（まだ記録がありません）"));
    }

    #[test]
    fn 未設定の選択は既定を書く() {
        let config = Map::new();
        let text = build_report(&sample_report(&config, "行"));
        assert!(text.contains("AI: local"));
        assert!(text.contains("表示形式: pet"));
        assert!(text.contains("キャラクター: mugi"));
        // 末尾に改行が無いログでも行が繋がらない。
        assert!(text.ends_with("行\n"));
    }
}
