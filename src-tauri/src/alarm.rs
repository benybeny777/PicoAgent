// alarm.rs - 目覚まし時計の保存と照合の中核。
//
// アラーム一覧は設定キー `PICOAGENT_ALARMS` に JSON 文字列で保存する（設定の正本は
// config.json）。チャット（雑談ならぬ目覚ましスキル `skills/alarm`）と設定画面の両方から
// 編集でき、発火（声でやさしく起こす）はフロントの時刻ポーリングが `pet_alarm_wake` を呼ぶ。
// 設定として変えうる値なので config に置き、ここは読み書きと曜日・時刻の正規化だけを担う。

use serde::{Deserialize, Serialize};

use crate::config::Config;

pub const CONFIG_KEY: &str = "PICOAGENT_ALARMS";

fn default_true() -> bool {
    true
}

fn default_snooze_minutes() -> u32 {
    5
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alarm {
    pub id: String,
    /// "HH:MM"（24時間表記）。
    pub time: String,
    /// 曜日 mon..sun。空なら毎日。
    #[serde(default)]
    pub days: Vec<String>,
    /// 繰り返すか（false なら一度鳴ったら無効化する一度きり）。
    #[serde(default = "default_true")]
    pub repeat: bool,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 起床確認がないときに再度鳴らすまでの分数。
    #[serde(default = "default_snooze_minutes")]
    pub snooze_minutes: u32,
}

const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// 保存済みのアラーム一覧を読む。壊れていた場合は空一覧にせずエラーを返す
/// （空から始めて書き戻すと、登録済みのアラームがすべて消えるため）。
pub fn load(cfg: &Config) -> Result<Vec<Alarm>, String> {
    crate::store::load_list(cfg, CONFIG_KEY)
}

/// アラーム一覧を保存する（config に JSON 文字列で書く）。
pub fn save(cfg: &Config, alarms: &[Alarm]) -> anyhow::Result<()> {
    let mut patch = serde_json::Map::new();
    patch.insert(
        CONFIG_KEY.to_string(),
        serde_json::Value::String(serde_json::to_string(alarms)?),
    );
    cfg.set_all(patch)?;
    Ok(())
}

/// 1トークンを曜日コード集合へ。毎日/平日/週末や日本語・英語表記に対応。
fn day_codes(token: &str) -> Vec<&'static str> {
    match token.trim().to_lowercase().as_str() {
        "everyday" | "daily" | "all" | "毎日" | "まいにち" => WEEKDAYS.to_vec(),
        "weekday" | "weekdays" | "平日" => vec!["mon", "tue", "wed", "thu", "fri"],
        "weekend" | "weekends" | "週末" => vec!["sat", "sun"],
        "mon" | "monday" | "月" | "月曜" | "月曜日" => vec!["mon"],
        "tue" | "tuesday" | "火" | "火曜" | "火曜日" => vec!["tue"],
        "wed" | "wednesday" | "水" | "水曜" | "水曜日" => vec!["wed"],
        "thu" | "thursday" | "木" | "木曜" | "木曜日" => vec!["thu"],
        "fri" | "friday" | "金" | "金曜" | "金曜日" => vec!["fri"],
        "sat" | "saturday" | "土" | "土曜" | "土曜日" => vec!["sat"],
        "sun" | "sunday" | "日" | "日曜" | "日曜日" => vec!["sun"],
        _ => vec![],
    }
}

/// 曜日表現を mon..sun の正規形へ。空指定や「毎日」は空ベクタ（=毎日）。
///
/// **解釈できない曜日が混じっていたらエラーにする。** 以前は黙って捨てており、
/// 指定がすべて未知語（`"Mon."` や全角の揺れなど）だと結果が空になり、
/// 空＝毎日の規則に吸い込まれて**「月曜だけ」の依頼が毎日鳴る**状態になっていた。
/// 落とすなら気づける形で落とす。
pub fn normalize_days(input: &[String]) -> Result<Vec<String>, String> {
    let mut set: Vec<&'static str> = Vec::new();
    let mut everyday = false;
    let mut unknown: Vec<String> = Vec::new();
    for raw in input {
        if raw.trim().is_empty() {
            continue;
        }
        let codes = day_codes(raw);
        if codes.is_empty() {
            unknown.push(raw.trim().to_string());
            continue;
        }
        if codes.len() == 7 {
            everyday = true;
        }
        for code in codes {
            if !set.contains(&code) {
                set.push(code);
            }
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "曜日を解釈できません: {}\n             「月」「月曜」「mon」「平日」「週末」「毎日」のように指定してください。",
            unknown.join("、")
        ));
    }
    if everyday {
        return Ok(Vec::new());
    }
    Ok(WEEKDAYS
        .iter()
        .filter(|d| set.contains(d))
        .map(|d| d.to_string())
        .collect())
}

/// "H:MM" / "HH:MM" を "HH:MM" へ正規化（範囲外は None）。
pub fn normalize_time(input: &str) -> Option<String> {
    let (h, m) = input.trim().split_once(':')?;
    let hour: u32 = h.trim().parse().ok()?;
    let minute: u32 = m.trim().parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{hour:02}:{minute:02}"))
}

/// 曜日コードを日本語表示へ（一覧・確認用）。空=毎日。
pub fn days_label(days: &[String]) -> String {
    if days.is_empty() {
        return "毎日".to_string();
    }
    let jp = |code: &str| match code {
        "mon" => "月",
        "tue" => "火",
        "wed" => "水",
        "thu" => "木",
        "fri" => "金",
        "sat" => "土",
        "sun" => "日",
        _ => "?",
    };
    let ordered: Vec<&str> = WEEKDAYS
        .iter()
        .filter(|d| days.iter().any(|x| x == *d))
        .map(|d| jp(d))
        .collect();
    ordered.join("・")
}

#[cfg(test)]
mod tests {
    use super::{days_label, load, normalize_days, normalize_time, Alarm, CONFIG_KEY};
    use crate::config::Config;

    #[test]
    fn 時刻を正規化する() {
        assert_eq!(normalize_time("7:5").as_deref(), Some("07:05"));
        assert_eq!(normalize_time("23:59").as_deref(), Some("23:59"));
        assert_eq!(normalize_time("24:00"), None);
        assert_eq!(normalize_time("おはよう"), None);
    }

    /// 解釈できない曜日を黙って捨てると、空＝毎日の規則に吸い込まれて
    /// 「月曜だけ」の依頼が毎日鳴る。落とすなら気づける形で落とす。
    #[test]
    fn 解釈できない曜日は毎日へ吸い込まれない() {
        let error = normalize_days(&["Mon.".to_string()]).expect_err("未知語を通した");
        assert!(error.contains("Mon."), "error: {error}");
        let error = normalize_days(&["月".to_string(), "げつよう".to_string()])
            .expect_err("一部でも未知語なら断る");
        assert!(error.contains("げつよう"), "error: {error}");
        // 指定なしは従来どおり毎日。
        assert!(normalize_days(&[]).unwrap().is_empty());
        assert!(normalize_days(&["".to_string()]).unwrap().is_empty());
    }

    #[test]
    fn 曜日を正規化する() {
        assert_eq!(
            normalize_days(&["平日".to_string()]).unwrap(),
            vec!["mon", "tue", "wed", "thu", "fri"]
        );
        assert!(normalize_days(&["毎日".to_string()]).unwrap().is_empty());
        assert_eq!(
            normalize_days(&["日".to_string(), "土".to_string()]).unwrap(),
            vec!["sat", "sun"]
        );
        assert_eq!(
            days_label(&["mon".to_string(), "fri".to_string()]),
            "月・金"
        );
        assert_eq!(days_label(&[]), "毎日");
    }

    #[test]
    fn 古い目覚まし_json_はスヌーズ五分で読む() {
        let raw =
            r#"[{"id":"a","time":"07:00","days":[],"repeat":true,"label":"朝","enabled":true}]"#;
        let alarms: Vec<Alarm> = serde_json::from_str(raw).unwrap();
        assert_eq!(alarms[0].snooze_minutes, 5);
    }

    /// 壊れた JSON を空一覧として返すと、呼び出し側がそのまま書き戻して
    /// 登録済みの目覚ましを全部消してしまう。読めなければエラーにする。
    #[test]
    fn 壊れた保存内容は空一覧にせずエラーにする() {
        let cfg = Config::new();
        // 末尾が切れた JSON（書き込み中断や手編集で起こりうる形）。
        cfg.set(CONFIG_KEY, r#"[{"id":"a1","time":"07:00""#);
        let error = load(&cfg).unwrap_err();
        assert!(error.contains("読み取れません"), "error: {error}");
        assert!(error.contains("中止"), "何をしたかを伝えること: {error}");

        // 未設定・空文字は「登録なし」として正常に空一覧。
        cfg.set(CONFIG_KEY, "");
        assert!(load(&cfg).unwrap().is_empty());
        cfg.set(CONFIG_KEY, "[]");
        assert!(load(&cfg).unwrap().is_empty());
    }
}

/// 設計ガード: 曜日・時刻の語彙をこのファイル以外へ写さないこと。
///
/// `scheduler.rs` が `day_codes` / `normalize_days` / `normalize_time` / `days_label`
/// を丸ごと写しており、`days_label` は既に書き方が分かれていた（出力は同じだが、
/// 片方だけ直せば静かにずれる状態）。曜日の解釈は目覚ましと予約実行の両方が
/// 依存するので、定義元を1つに保つ。
#[cfg(test)]
mod guard {
    #[test]
    fn 曜日語彙の写しを増やさない() {
        let violations: Vec<String> = crate::guard_scan::all_sources()
            .into_iter()
            .filter(|source| source.name != "alarm.rs")
            .filter(|source| source.body().contains("\"everyday\" | \"daily\" | \"all\""))
            .map(|source| source.path.display().to_string())
            .collect();
        assert!(
            violations.is_empty(),
            "曜日の解釈は alarm::day_codes（および normalize_days）を使ってください: {violations:?}"
        );
    }
}
