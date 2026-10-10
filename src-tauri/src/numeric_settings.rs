// 整数数量設定の正本。保存・読取・画面の制約を同じ宣言から生成する。
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, Debug)]
pub struct IntSetting {
    pub key: &'static str,
    pub label: &'static str,
    pub default: Option<i64>,
    pub min: i64,
    pub max: i64,
    pub unit: &'static str,
}

macro_rules! declare_int_settings {
    ($($name:ident: ($label:literal, $default:expr, $min:expr, $max:expr, $unit:literal)),* $(,)?) => {
        $(pub const $name: IntSetting = IntSetting {
            key: concat!("PICOAGENT_", stringify!($name)), label: $label,
            default: $default, min: $min, max: $max, unit: $unit,
        };)*
        pub const INT_SETTINGS: &[IntSetting] = &[$($name),*];
    };
}

declare_int_settings! {
    AGENT_CLI_TIMEOUT_SECS: ("1回の応答を待つ秒数", Some(180), 10, 600, "秒"),
    API_PORT: ("ローカルAPIのポート", Some(11500), 1, 65535, ""),
    BLUESKY_TIMEOUT_SECS: ("Bluesky API の待ち時間", Some(20), 3, 60, "秒"),
    BOOT_TIMEOUT_MS: ("起動進捗通知の待ち時間", Some(120000), 3000, 120000, "ms"),
    BROWSER_MAX_CHARS: ("ブラウザ本文の最大文字数", None, 0, i64::MAX, "文字"),
    BROWSER_PORT: ("ブラウザCDPのポート", Some(11437), 1, 65535, ""),
    BROWSER_TIMEOUT_SECS: ("1操作の待ち時間", Some(30), 5, 180, "秒"),
    BUBBLE_FONT_PX: ("ペット・ミニのフキダシ文字サイズ(px)", Some(13), 10, 28, "px"),
    CHATTER_MAX_SECS: ("自発雑談の最長間隔", Some(240), 30, 3600, "秒"),
    CHATTER_MIN_SECS: ("自発雑談の最短間隔", Some(90), 30, 3600, "秒"),
    CHATTER_TOPIC_TIMEOUT_SECS: ("自発雑談の話題取得待ち時間", Some(8), 3, 30, "秒"),
    CLIPBOARD_MAX_CHARS: ("クリップボード取得の最大文字数", Some(3000), 500, 8000, "文字"),
    CODE_RUNNER_TIMEOUT_SECS: ("code_runner の既定タイムアウト秒", Some(60), 1, 300, "秒"),
    COMMAND_TIMEOUT_SECS: ("コマンド実行の既定タイムアウト秒", Some(30), 1, 120, "秒"),
    CURRENCY_TIMEOUT_SECS: ("為替API待ち時間", Some(15), 3, 60, "秒"),
    DESKTOP_MAX_OPS_PER_MIN: ("1分あたりの操作回数の上限", Some(60), 1, 600, "回"),
    DISCORD_TIMEOUT_SECS: ("Discord Webhook の待ち時間", Some(15), 3, 60, "秒"),
    DOCUMENT_CONVERT_TIMEOUT_SECS: ("文書変換タイムアウト秒", Some(120), 5, 300, "秒"),
    GAME_MAX_OPS: ("利用者の1依頼あたりの操作試行上限", Some(1), 1, 100, "回"),
    GITHUB_TIMEOUT_SECS: ("GitHub API の待ち時間", Some(20), 3, 60, "秒"),
    GMAIL_CLEANUP_MAX_RESULTS: ("Gmail の検索条件ベース整理/掃除で一度に扱う既定上限", Some(50), 0, i64::MAX, "件"),
    GMAIL_TIMEOUT_SECS: ("Gmail API の待ち時間", Some(20), 3, 60, "秒"),
    GOOGLE_CALENDAR_TIMEOUT_SECS: ("Google Calendar API の待ち時間", Some(20), 3, 60, "秒"),
    GOOGLE_DOCS_TIMEOUT_SECS: ("Google Docs API の待ち時間", Some(20), 3, 60, "秒"),
    GOOGLE_DRIVE_TIMEOUT_SECS: ("Google Drive API の待ち時間", Some(20), 3, 60, "秒"),
    GOOGLE_SHEETS_TIMEOUT_SECS: ("Google Sheets API の待ち時間", Some(20), 3, 60, "秒"),
    GOOGLE_TASKS_TIMEOUT_SECS: ("Google Tasks API の待ち時間", Some(15), 3, 60, "秒"),
    HTTP_METADATA_TIMEOUT_SECS: ("モデル一覧取得など短い問い合わせの待ち時間", Some(15), 3, 60, "秒"),
    HTTP_TIMEOUT_SECS: ("汎用HTTPの待ち時間", Some(20), 3, 60, "秒"),
    IMAGE_TOOLS_TIMEOUT_SECS: ("画像操作タイムアウト秒", Some(120), 5, 300, "秒"),
    JIRA_TIMEOUT_SECS: ("Jira API の待ち時間", Some(20), 3, 60, "秒"),
    LINEAR_TIMEOUT_SECS: ("Linear API の待ち時間", Some(20), 3, 60, "秒"),
    LINKEDIN_TIMEOUT_SECS: ("LinkedIn API の待ち時間", Some(20), 3, 60, "秒"),
    LOCAL_CTX: ("LLMのコンテキスト長", Some(32768), 32768, i64::MAX, "トークン"),
    LOCAL_NGL: ("GPUへオフロードするレイヤ数", Some(999), 0, i64::MAX, "レイヤ"),
    LOCAL_PORT: ("同梱LLMエンジンのポート", Some(11435), 1, 65535, ""),
    MARKET_TIMEOUT_SECS: ("市場指数API待ち時間", Some(15), 3, 60, "秒"),
    MASTODON_TIMEOUT_SECS: ("Mastodon API の待ち時間", Some(20), 3, 60, "秒"),
    MEMORY_MAX_ITEMS: ("長期メモリに保持する最大件数", Some(80), 10, 200, "件"),
    MISSKEY_TIMEOUT_SECS: ("Misskey API の待ち時間", Some(20), 3, 60, "秒"),
    MS_GRAPH_TIMEOUT_SECS: ("Microsoft Graph API の待ち時間", Some(20), 3, 60, "秒"),
    MS_OFFICE_TIMEOUT_SECS: ("Microsoft Office 操作タイムアウト秒", Some(120), 5, 300, "秒"),
    MULTI_AGENT_MAX_CONCURRENCY: ("外部AI候補の最大同時数", Some(3), 1, 8, "候補"),
    NAP_MINUTES: ("居眠りまでの無操作時間", Some(15), 0, i64::MAX, "分"),
    NEWS_SCRAPE_MAX_ITEMS: ("Google News / Yahoo!ニュースから各サイト取得する最大件数", Some(8), 1, 12, "件"),
    NEWS_SCRAPE_TIMEOUT_SECS: ("ニュースHTML取得待ち時間", Some(15), 3, 60, "秒"),
    NOTION_TIMEOUT_SECS: ("Notion API の待ち時間", Some(20), 3, 60, "秒"),
    PDF_TIMEOUT_SECS: ("PDF操作タイムアウト秒", Some(120), 5, 300, "秒"),
    REDDIT_TIMEOUT_SECS: ("Reddit API の待ち時間", Some(20), 3, 60, "秒"),
    RESEARCH_TIMEOUT_SECS: ("調査レポートの各Web検索待ち時間", Some(15), 3, 60, "秒"),
    RSS_TIMEOUT_SECS: ("RSS取得待ち時間", Some(15), 3, 60, "秒"),
    SLACK_TIMEOUT_SECS: ("Slack API の待ち時間", Some(15), 3, 60, "秒"),
    STREAM_CHAT_FONT_PX: ("配信チャットの履歴・入力文字サイズ(px)", Some(22), 18, 48, "px"),
    SUBAGENT_MAX_DEPTH: ("サブエージェントの最大ネスト深さ", Some(2), 1, i64::MAX, "段"),
    SUPER_AGENT_MAX_STEPS: ("スーパーエージェントモードのトップレベル最大ループ数", Some(12), 1, 30, "回"),
    TODOIST_TIMEOUT_SECS: ("Todoist API の待ち時間", Some(15), 3, 60, "秒"),
    TOOL_CONTEXT_TURNS: ("ツール定義の並び順へ使う発話件数", Some(5), 0, i64::MAX, "件"),
    TOOL_OUTPUT_MAX_CHARS: ("ツール出力の最大文字数", None, 1000, 60000, "文字"),
    TRANSCRIBE_TIMEOUT_SECS: ("ローカル文字起こしタイムアウト秒", Some(1800), 10, 7200, "秒"),
    TRELLO_TIMEOUT_SECS: ("Trello API の待ち時間", Some(15), 3, 60, "秒"),
    TTS_PORT: ("同梱音声エンジンのポート", Some(11436), 1, 65535, ""),
    TTS_VOLUME: ("キャラボイスとゲーム効果音の共通音量", Some(35), 0, 100, "%"),
    WEATHER_TIMEOUT_SECS: ("天気API待ち時間", Some(15), 3, 60, "秒"),
    WEB_FETCH_TIMEOUT_SECS: ("URL取得の待ち時間", Some(20), 3, 60, "秒"),
    WEB_SEARCH_MAX_RESULTS: ("Web検索で返す件数", Some(5), 1, 8, "件"),
    WEB_SEARCH_TIMEOUT_SECS: ("Web検索の待ち時間", Some(15), 3, 60, "秒"),
    X_TIMEOUT_SECS: ("X API の待ち時間", Some(20), 3, 60, "秒"),
}

impl IntSetting {
    pub fn default_text(&self) -> String {
        self.default
            .map_or_else(|| "auto".to_string(), |value| value.to_string())
    }

    fn invalid(&self) -> String {
        format!(
            "{}（{}）は {}〜{}{} の整数{}で指定してください。",
            self.key,
            self.label,
            self.min,
            self.max,
            self.unit,
            if self.default.is_none() {
                " または auto"
            } else {
                ""
            }
        )
    }

    /// 空欄は宣言の既定値だけへ戻す。不正値を既定値に降格させない。
    pub fn parse(&self, raw: &str) -> Result<Option<i64>, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(self.default);
        }
        if raw.eq_ignore_ascii_case("auto") {
            return if self.default.is_none() {
                Ok(None)
            } else {
                Err(self.invalid())
            };
        }
        let normalized: String = raw
            .chars()
            .map(|character| match character {
                '０'..='９' => char::from(b'0' + (character as u32 - '０' as u32) as u8),
                other => other,
            })
            .collect();
        let value = normalized.parse::<i64>().map_err(|_| self.invalid())?;
        if !(self.min..=self.max).contains(&value) {
            return Err(self.invalid());
        }
        Ok(Some(value))
    }
}

pub fn find(key: &str) -> Option<&'static IntSetting> {
    INT_SETTINGS.iter().find(|setting| setting.key == key)
}

pub fn number_settings_schema() -> Map<String, Value> {
    INT_SETTINGS
        .iter()
        .map(|setting| {
            (
                setting.key.to_string(),
                json!({
                    "range": [setting.min.to_string(), setting.max.to_string()],
                    "unit": setting.unit, "allow_auto": setting.default.is_none(),
                    "default": setting.default_text(), "label": setting.label,
                }),
            )
        })
        .collect()
}

/// 元の値を残すため、不正項目を全件返し、呼出側が保存全体を拒否する。
pub fn validate_int_patch(mut patch: Map<String, Value>) -> (Map<String, Value>, Vec<Value>) {
    let mut invalid = Vec::new();
    for (key, value) in &mut patch {
        let Some(setting) = find(key) else { continue };
        if value.is_null() {
            continue;
        }
        let raw = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        if raw.trim().is_empty() {
            *value = Value::String(String::new());
            continue;
        }
        match setting.parse(&raw) {
            Ok(value_number) => {
                *value = Value::String(
                    value_number.map_or_else(|| "auto".to_string(), |number| number.to_string()),
                )
            }
            Err(message) => invalid.push(json!({"key":key, "message":message})),
        }
    }
    (patch, invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 全宣言は一意で既定値と両端が範囲内に収まる() {
        let mut keys = std::collections::HashSet::new();
        for setting in INT_SETTINGS {
            assert!(keys.insert(setting.key), "{}", setting.key);
            assert!(setting.min <= setting.max, "{}", setting.key);
            assert_eq!(
                setting.parse(&setting.default_text()).unwrap(),
                setting.default
            );
            for edge in [setting.min, setting.max] {
                assert_eq!(setting.parse(&edge.to_string()).unwrap(), Some(edge));
            }
        }
        assert_eq!(INT_SETTINGS.len(), 67);
    }

    #[test]
    fn 全角整数を正規化し小数型範囲とoverflowを拒否する() {
        assert_eq!(WEB_SEARCH_MAX_RESULTS.parse(" ３ ").unwrap(), Some(3));
        for bad in [
            "0",
            "9",
            "3.0",
            "１．５",
            "abc",
            "true",
            "9223372036854775808",
            "auto",
        ] {
            assert!(WEB_SEARCH_MAX_RESULTS
                .parse(bad)
                .unwrap_err()
                .contains(WEB_SEARCH_MAX_RESULTS.key));
        }
        assert_eq!(BROWSER_MAX_CHARS.parse("AuTo").unwrap(), None);
        assert_eq!(BROWSER_MAX_CHARS.parse("0").unwrap(), Some(0));
        assert_eq!(GMAIL_CLEANUP_MAX_RESULTS.parse("0").unwrap(), Some(0));
    }

    #[test]
    fn 保存検査は全不正項目を返し未指定と他種設定を保持する() {
        let patch = json!({
            "PICOAGENT_WEB_SEARCH_MAX_RESULTS":"３",
            "PICOAGENT_HTTP_TIMEOUT_SECS":"abc",
            "PICOAGENT_RSS_TIMEOUT_SECS":true,
            "PICOAGENT_CLIPBOARD_MAX_CHARS":{},
            "PICOAGENT_BROWSER_MAX_CHARS":"AUTO",
            "PICOAGENT_NAP_MINUTES":null,
            "PICOAGENT_TTS_VOLUME":" ",
            "PICOAGENT_CHARACTER":"miko",
        })
        .as_object()
        .unwrap()
        .clone();
        let (normalized, invalid) = validate_int_patch(patch);
        assert_eq!(invalid.len(), 3);
        assert_eq!(normalized["PICOAGENT_WEB_SEARCH_MAX_RESULTS"], "3");
        assert_eq!(normalized["PICOAGENT_BROWSER_MAX_CHARS"], "auto");
        assert!(normalized["PICOAGENT_NAP_MINUTES"].is_null());
        assert_eq!(normalized["PICOAGENT_TTS_VOLUME"], "");
        assert_eq!(normalized["PICOAGENT_CHARACTER"], "miko");
        for error in invalid {
            assert!(error["message"]
                .as_str()
                .unwrap()
                .contains(error["key"].as_str().unwrap()));
        }
    }

    #[test]
    fn 画面の範囲は整数精度を失わない文字列で返す() {
        let schema = number_settings_schema();
        assert_eq!(
            schema[BROWSER_MAX_CHARS.key]["range"][1],
            i64::MAX.to_string()
        );
        assert_eq!(schema[BROWSER_MAX_CHARS.key]["default"], "auto");
        assert_eq!(schema[HTTP_TIMEOUT_SECS.key]["range"], json!(["3", "60"]));
    }
    #[test]
    fn 固定接続のポートは動的割当のゼロを拒否する() {
        for setting in [&API_PORT, &BROWSER_PORT, &LOCAL_PORT, &TTS_PORT] {
            assert!(setting.parse("0").is_err(), "{}", setting.key);
            assert_eq!(setting.parse("65535").unwrap(), Some(65535));
            assert!(setting.parse("65536").is_err());
        }
    }
}
