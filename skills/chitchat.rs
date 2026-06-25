// chitchat.rs - アイドル時にキャラクターが自発的にひとこと雑談するスキル。
//
// このスキルはモデルが会話中に呼ぶ通常ツールではなく（agent_callable=false）、
// フロントのアイドル検知タイマーが `pet_idle_chatter` 経由で `dispatch` する内部スキル。
// run() は「独り言を1つ言う」ための指示文（プロンプト）を返し、実際の生成は
// commands.rs 側が選択中キャラの人格（system プロンプト）込みでプロバイダに依頼する。
// ON/OFF と発話間隔は config_schema で設定画面に並ぶ。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

// 自発雑談で使う「独り言を1つ」生成させる指示文。
// 重要: 挨拶・名乗り・自己紹介・特技説明（「むぎだよ」「観察しながら…」等の決まり文句）は禁止。
// 待機中にふと漏らす独り言だけを求める。
const CHATTER_INSTRUCTION: &str = "いまは誰も話しかけていない待機中に、ふと漏らす自発的な独り言です。選択中キャラクター本人の口調で、いま思いついたような短い独り言を1つだけ言ってください。厳守: 挨拶（こんにちは等）・名乗り・自己紹介・自分の特技や役割の説明は絶対にしない（「むぎだよ」「観察しながら手伝う」のような決まり文句は禁止）。相手に話しかけず、質問・指示・催促もしない。30文字程度までの1文。Web検索や事実確認はしない。URL・箇条書き・絵文字の多用はしない。前回までと違う話題にし、同じ言い回しを繰り返さない。";

// 毎回ちがう切り口にするための話題ヒント（小型モデルが定型文へ寄らないよう揺らぎを与える）。
const TOPIC_HINTS: &[&str] = &[
    "いまの気分",
    "ふと思い出した小さなこと",
    "いまの時間帯や天気の感じ",
    "ちょっとした気づき",
    "好きなものや興味",
    "ひと休みのおすすめ",
    "目に入ったものへの感想",
    "なんでもない軽いつぶやき",
    "作業のはかどり具合への感想",
    "これからやってみたい小さなこと",
];

// 時刻をもとに話題ヒントを1つ選ぶ（依存を増やさず擬似ランダムに回す）。
fn pick_topic_hint() -> &'static str {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0) as usize;
    TOPIC_HINTS[nanos % TOPIC_HINTS.len()]
}

pub struct ChitchatTool;

#[async_trait]
impl Tool for ChitchatTool {
    fn name(&self) -> &'static str {
        "chitchat"
    }

    fn label(&self) -> &'static str {
        "雑談（自発）"
    }

    fn description(&self) -> String {
        "アイドル（待機）中にキャラクターが自発的にひとこと雑談する機能。設定で ON/OFF と発話間隔を変えられる。通常の会話でモデルが呼ぶツールではなく、アプリの待機タイマーが使う。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    // 会話中にモデルへ提示しない（アプリが直接 dispatch する内部スキル）。
    fn agent_callable(&self) -> bool {
        false
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_CHATTER_ENABLED",
                "アイドル時に自発的な雑談をする",
                "checkbox",
                "",
            ),
            Field::new(
                "PICOAGENT_CHATTER_MIN_SECS",
                "自発雑談の最短間隔（秒、30〜3600）",
                "text",
                "90",
            ),
            Field::new(
                "PICOAGENT_CHATTER_MAX_SECS",
                "自発雑談の最長間隔（秒、30〜3600）",
                "text",
                "240",
            ),
        ]
    }

    async fn run(&self, _cfg: &Config, _args: Value) -> String {
        format!(
            "{CHATTER_INSTRUCTION}\n今回の話題のヒント: {}（このヒントは出力に書かず、内容の参考にだけ使う）。",
            pick_topic_hint()
        )
    }
}

inventory::submit! { ToolReg { make: || Box::new(ChitchatTool) } }
