// emotion.rs - 会話内容に基づくキャラクター感情状態管理（ルールベース）。
//
// ユーザー発話とアシスタント返答から感情の種類とエネルギーレベルを推定し、
// AppState に保持する。フロントへ `pet:emotion` イベントで通知する。

use serde::Serialize;

/// キャラクターの現在の感情状態。
#[derive(Clone, Debug, Serialize)]
pub struct EmotionState {
    /// 感情の種類: "happy" | "sad" | "excited" | "calm" | "curious" | "surprised"
    pub mood: String,
    /// エネルギーレベル（0.0 = 低活性・落ち込み、1.0 = 高活性・興奮）
    pub energy: f32,
}

impl Default for EmotionState {
    fn default() -> Self {
        EmotionState {
            mood: "calm".to_string(),
            energy: 0.5,
        }
    }
}

/// ユーザー発話とアシスタント返答から感情状態を更新する（ルールベース）。
///
/// 現在の感情へのゆるやかな回帰（慣性）を持ち、急激な感情変化を避ける。
pub fn update(current: &EmotionState, user_text: &str, assistant_text: &str) -> EmotionState {
    let combined = format!("{} {}", user_text, assistant_text).to_lowercase();

    // 喜び・承認系
    let joy = contains_any(
        &combined,
        &[
            "ありがとう", "嬉しい", "うれしい", "よかった", "助かった",
            "すごい", "すばらしい", "さすが", "やった", "できた", "やったー",
            "わーい", "好き", "かわいい", "ありがと",
        ],
    );

    // 悲しみ・困惑系
    let sad = contains_any(
        &combined,
        &[
            "悲しい", "かなしい", "つらい", "しんどい", "落ち込", "疲れた",
            "失敗", "ごめん", "すまない", "残念", "だめだった", "うまくいかない",
            "エラー", "動かない", "おかしい",
        ],
    );

    // 興奮・驚き系
    let excited = contains_any(
        &combined,
        &[
            "！！", "やった", "びっくり", "驚", "おどろ", "信じられない",
            "まじか", "えー", "すごすぎ", "最高", "やばい",
        ],
    );

    // 好奇心・探求系
    let curious = contains_any(
        &combined,
        &[
            "教えて", "知りたい", "気になる", "なぜ", "どうして",
            "どうやって", "なんで", "不思議", "詳しく", "調べて",
        ],
    );

    // 怒り・苛立ち系
    let angry = contains_any(
        &combined,
        &["むかつく", "怒", "いらいら", "うるさい", "やめて", "消えて"],
    );

    // 感情とエネルギーを決定（優先順位付き）。
    let (next_mood, target_energy): (&str, f32) = if excited {
        ("excited", 0.9)
    } else if angry {
        ("surprised", 0.75)
    } else if joy {
        ("happy", 0.72)
    } else if sad {
        ("sad", 0.25)
    } else if curious {
        ("curious", 0.62)
    } else {
        // 変化なし → 現状から calm へ緩やかに収束する。
        ("calm", 0.5)
    };

    // 慣性: 急変を防ぐため現在値との加重平均を取る（感情の自然な遷移）。
    let inertia = 0.35_f32;
    let new_energy = current.energy * inertia + target_energy * (1.0 - inertia);

    EmotionState {
        mood: next_mood.to_string(),
        energy: new_energy.clamp(0.0, 1.0),
    }
}

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| text.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::{update, EmotionState};

    #[test]
    fn お礼を言われたら喜ぶ() {
        let s = update(&EmotionState::default(), "ありがとう", "どういたしまして");
        assert_eq!(s.mood, "happy");
        assert!(s.energy > 0.5);
    }

    #[test]
    fn エラー報告には悲しむ() {
        let s = update(&EmotionState::default(), "エラーが出た", "調べてみますね");
        assert_eq!(s.mood, "sad");
        assert!(s.energy < 0.5);
    }

    #[test]
    fn 質問されたら好奇心を示す() {
        let s = update(&EmotionState::default(), "なぜですか", "詳しく説明します");
        assert_eq!(s.mood, "curious");
    }
}
