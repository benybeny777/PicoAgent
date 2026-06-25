// tools/mod.rs - エージェント機能（スキル）コネクタのトレイトとレジストリ。
//
// 各スキルは Tool を実装し、inventory::submit! で自身を登録する。
// 連結ファイルはプロジェクト直下の skills/ に置き、build.rs が skills/*.rs を
// 走査して `pub mod` 宣言を自動生成するため、.rs を1枚置くだけでスキルを追加できる
// （このファイルの編集は不要）。トレイト・レジストリ本体だけがここに残る。

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;
use crate::message::Field;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn label(&self) -> &'static str {
        self.name()
    }
    fn description(&self) -> String;
    fn parameters(&self) -> Value;
    fn config_schema(&self) -> Vec<Field> {
        Vec::new()
    }
    /// エージェントループでモデルに function として提示するか。
    /// アプリ側が直接 `dispatch` するだけの内部スキル（例: 自発雑談）は false にして、
    /// モデルが会話中に誤って呼ばないようにする（`dispatch` 自体は false でも実行できる）。
    fn agent_callable(&self) -> bool {
        true
    }
    async fn run(&self, cfg: &Config, args: Value) -> String;
}

pub struct ToolReg {
    pub make: fn() -> Box<dyn Tool>,
}
inventory::collect!(ToolReg);

// build.rs が生成した `pub mod time; pub mod read_file; ...` を取り込む。
include!(concat!(env!("OUT_DIR"), "/tool_mods.rs"));

pub fn all() -> Vec<Box<dyn Tool>> {
    inventory::iter::<ToolReg>
        .into_iter()
        .map(|r| (r.make)())
        .collect()
}

/// プロバイダに渡す function スキーマ配列（OpenAI/Ollama 共通形式）を生成。
pub fn build_schemas() -> Vec<Value> {
    all()
        .iter()
        .filter(|t| t.agent_callable())
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters(),
                }
            })
        })
        .collect()
}

/// モデルが要求したツール呼び出しを実行する。失敗は例外にせず文字列で返す。
pub async fn dispatch(cfg: &Config, name: &str, args: Value) -> String {
    for t in all() {
        if t.name() == name {
            return t.run(cfg, args).await;
        }
    }
    format!("エラー: 未登録のツール \"{}\" が呼び出されました。", name)
}
