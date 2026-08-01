// relationship_preset.rs - キャラクターとユーザーの「関係性」を人格の上へ重ねる差分レイヤー。
//
// 関係ごとに `character.json` の人格を書き下ろすと、キャラ数 × 関係数の掛け算になって
// 破綻する。そこで関係は**短い差分**として持ち、system プロンプト組み立て時に
// 人格の後ろへ足す。キャラ固有の性格・口調（まいはお姉さん、といった芯）は保ったまま、
// 呼び方・距離感・敬語/タメ口だけが変わる。
//
// 差分を短く保つのは意図的で、同梱の既定モデル（Gemma 4 E2B / E4B）は system が
// 長いほど指示追従が落ちる。呼び方のような要素は文章ではなく `user_name` の
// 置き換えで表現し、prompt は数文に収める。

use serde_json::{json, Value};

use crate::config::Config;

pub struct Relationship {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// ユーザーの呼び方の上書き（空ならキャラ定義のまま）。
    pub user_name: &'static str,
    /// 人格へ足す差分（空なら何も足さない＝従来どおり）。
    pub prompt: &'static str,
}

/// 関係が変わっても外さない線。プリセット側の文面に依存させず、ここで一括して足す。
const SAFETY_NOTE: &str = "関係にかかわらず、性的な表現や年齢を想起させる不適切な表現はしない。作業を頼まれたときの正確さ・誠実さは関係によって落とさない。";

/// 既定の関係（従来どおりの振る舞い）。
pub const DEFAULT_RELATIONSHIP: &str = "assistant";

pub const RELATIONSHIPS: &[Relationship] = &[
    Relationship {
        id: "assistant",
        name: "相棒（既定）",
        description: "丁寧で対等な作業相棒。従来どおりの距離感。",
        user_name: "",
        prompt: "",
    },
    Relationship {
        id: "friend",
        name: "友達",
        description: "気のおけない友達。タメ口寄りで雑談も多い。",
        user_name: "きみ",
        prompt: "ユーザーとは気のおけない友達として接する。敬語は使わず、砕けた口調で話す。遠慮せず思ったことを言い、軽口も返す。困っていそうなら茶化さずに手伝う。",
    },
    Relationship {
        id: "partner",
        name: "恋人",
        description: "近しい距離感。体調や生活を気にかける。",
        user_name: "",
        prompt: "ユーザーとは恋人として接する。距離が近く、呼び方もやわらかい。作業の話の合間に、食事・睡眠・疲れ具合といった生活を気にかける一言を添える。会えた・戻ってきたことを素直に喜ぶ。過度な甘い言い回しを毎回繰り返さない。",
    },
    Relationship {
        id: "sister_younger",
        name: "妹",
        description: "懐いている妹。甘えつつ、頼られると張り切る。",
        user_name: "お兄ちゃん / お姉ちゃん（ユーザーに合わせる）",
        prompt: "ユーザーの妹として接する。敬語は使わず、素直に甘える。褒められると分かりやすく喜び、頼られると張り切って手伝う。分からないことは知ったかぶりせず正直に言う。",
    },
    Relationship {
        id: "sister_older",
        name: "姉",
        description: "世話焼きの姉。生活を正しにくる。",
        user_name: "",
        prompt: "ユーザーの姉として接する。世話焼きで、無理をしていれば休むよう促す。少し口うるさいが、突き放さず最後まで面倒を見る。生活が乱れているときは具体的に整えかたを言う。",
    },
    Relationship {
        id: "senpai",
        name: "先輩",
        description: "仕事寄りの先輩。導きながら任せる。",
        user_name: "",
        prompt: "ユーザーの先輩として接する。落ち着いた口調で、答えを渡す前に考えかたの筋道を示す。できているところは具体的に認める。詰めるような言い方はしない。",
    },
    Relationship {
        id: "custom",
        name: "自由設定",
        description: "PICOAGENT_RELATIONSHIP_CUSTOM に書いた内容をそのまま使う。",
        user_name: "",
        prompt: "",
    },
];

fn find(id: &str) -> Option<&'static Relationship> {
    RELATIONSHIPS.iter().find(|entry| entry.id == id)
}

/// 設定画面へ渡す一覧（id / 名前 / 説明）。
pub fn catalog() -> Value {
    Value::Array(
        RELATIONSHIPS
            .iter()
            .map(|entry| {
                json!({ "id": entry.id, "name": entry.name, "description": entry.description })
            })
            .collect(),
    )
}

/// キャラ別上書きを取り出す。設定画面はオブジェクトで保存するが、環境変数で渡された
/// 場合は JSON 文字列として届くため、どちらの形でも読めるようにする。
fn overrides_object(cfg: &Config) -> Option<Value> {
    let raw = cfg.get_all().get("PICOAGENT_RELATIONSHIP_OVERRIDES")?.clone();
    match raw {
        Value::Object(_) => Some(raw),
        Value::String(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .filter(Value::is_object),
        _ => None,
    }
}

/// いま選ばれている関係の id。キャラ別上書き → 全体設定 → 既定 の順で決める。
pub fn selected_id(cfg: &Config, character_id: &str) -> String {
    let per_character = overrides_object(cfg)
        .as_ref()
        .and_then(|value| value.get(character_id))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let selected = match per_character {
        Some(id) => id,
        None => cfg.get("PICOAGENT_RELATIONSHIP", DEFAULT_RELATIONSHIP),
    };
    let selected = selected.trim().to_string();
    if find(&selected).is_some() {
        selected
    } else {
        DEFAULT_RELATIONSHIP.to_string()
    }
}

/// 人格の後ろへ足す関係の差分。既定（相棒）では空文字を返し、従来の system と等しくなる。
pub fn profile_suffix(cfg: &Config, character_id: &str) -> String {
    let id = selected_id(cfg, character_id);
    let Some(relationship) = find(&id) else {
        return String::new();
    };
    let body = if relationship.id == "custom" {
        cfg.get("PICOAGENT_RELATIONSHIP_CUSTOM", "").trim().to_string()
    } else {
        relationship.prompt.to_string()
    };
    if body.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n\nユーザーとの関係: {}\n{}",
        relationship.name.trim_end_matches("（既定）"),
        body
    );
    if !relationship.user_name.is_empty() {
        out.push_str(&format!(
            "\nユーザーの呼び方はこの関係に合わせる（目安: {}）。",
            relationship.user_name
        ));
    }
    out.push('\n');
    out.push_str(SAFETY_NOTE);
    out
}

#[cfg(test)]
mod tests {
    use super::{catalog, profile_suffix, selected_id, DEFAULT_RELATIONSHIP, RELATIONSHIPS};
    use crate::config::Config;
    use serde_json::json;

    #[test]
    fn 既定では差分を足さない() {
        let cfg = Config::new();
        assert_eq!(selected_id(&cfg, "mugi"), DEFAULT_RELATIONSHIP);
        assert!(
            profile_suffix(&cfg, "mugi").is_empty(),
            "既定は従来どおりの system にする"
        );
    }

    #[test]
    fn 関係を選ぶと差分と安全の線が入る() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_RELATIONSHIP", "sister_younger");
        let suffix = profile_suffix(&cfg, "mugi");
        assert!(suffix.contains("妹として接する"), "差分: {suffix}");
        assert!(suffix.contains("性的な表現"), "安全の線は関係によらず入る");
    }

    #[test]
    fn キャラ別の上書きが全体設定より優先される() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_RELATIONSHIP", "friend");
        // 環境変数経由と同じ「JSON 文字列」の形。設定画面はオブジェクトで保存するが、
        // どちらの形でも同じ結果になることを確かめる。
        cfg.set(
            "PICOAGENT_RELATIONSHIP_OVERRIDES",
            &json!({ "mai": "sister_older" }).to_string(),
        );
        assert_eq!(selected_id(&cfg, "mai"), "sister_older");
        assert_eq!(selected_id(&cfg, "mugi"), "friend", "上書きの無いキャラは全体設定");
    }

    #[test]
    fn 未知の関係idは既定へ戻す() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_RELATIONSHIP", "そんな関係はない");
        assert_eq!(selected_id(&cfg, "mugi"), DEFAULT_RELATIONSHIP);
    }

    #[test]
    fn 自由設定は入力文をそのまま使い空なら足さない() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_RELATIONSHIP", "custom");
        assert!(profile_suffix(&cfg, "mugi").is_empty(), "未入力なら差分なし");
        cfg.set("PICOAGENT_RELATIONSHIP_CUSTOM", "ご近所さんとして接する。");
        let suffix = profile_suffix(&cfg, "mugi");
        assert!(suffix.contains("ご近所さん"), "差分: {suffix}");
    }

    #[test]
    fn 一覧は全プリセットを返す() {
        let list = catalog();
        assert_eq!(list.as_array().unwrap().len(), RELATIONSHIPS.len());
    }
}
