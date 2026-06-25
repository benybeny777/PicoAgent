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
const CHATTER_INSTRUCTION: &str = "いまは誰も話しかけていない待機中です。選択中キャラクター本人として、自発的なひとことを言ってください。条件: 30文字程度までの短い1文にする。Web検索や事実確認はしない。質問攻めや指示・催促はしない。記号・箇条書き・絵文字の多用やURLは入れない。毎回ちがう話題（今の気分、ちょっとした気づき、季節や時間、軽い感想など）にして、同じ言い回しを繰り返さない。自分をAI・アプリ・デスクトップペットとして説明しない。";

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
        CHATTER_INSTRUCTION.to_string()
    }
}

inventory::submit! { ToolReg { make: || Box::new(ChitchatTool) } }
