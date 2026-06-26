// tools/mod.rs - エージェント機能（スキル）コネクタのトレイトとレジストリ。
//
// 各スキルは Tool を実装し、inventory::submit! で自身を登録する。
// 連結ファイルはプロジェクト直下の skills/<スキル名>/mod.rs に置き、
// build.rs が走査して `pub mod` 宣言を自動生成するため、ディレクトリを足すだけでスキルを追加できる
// （このファイルの編集は不要）。トレイト・レジストリ本体だけがここに残る。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

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

static TOOLS: OnceLock<Vec<Box<dyn Tool>>> = OnceLock::new();
static TOOL_INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
static TOOL_SCHEMAS: OnceLock<Vec<Value>> = OnceLock::new();

/// 登録済みスキルを返す。各スキルはプロセス内で1回だけ生成する。
pub fn all() -> &'static [Box<dyn Tool>] {
    TOOLS
        .get_or_init(|| {
            inventory::iter::<ToolReg>
                .into_iter()
                .map(|r| (r.make)())
                .collect()
        })
        .as_slice()
}

fn index() -> &'static HashMap<&'static str, usize> {
    TOOL_INDEX.get_or_init(|| {
        all()
            .iter()
            .enumerate()
            .map(|(idx, tool)| (tool.name(), idx))
            .collect()
    })
}

/// プロバイダに渡す function スキーマ配列（OpenAI/Ollama 共通形式）を生成。
pub fn build_schemas() -> &'static [Value] {
    TOOL_SCHEMAS
        .get_or_init(|| {
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
        })
        .as_slice()
}

/// モデルが要求したツール呼び出しを実行する。失敗は例外にせず文字列で返す。
pub async fn dispatch(cfg: &Config, name: &str, args: Value) -> String {
    if let Some(idx) = index().get(name) {
        return all()[*idx].run(cfg, args).await;
    }
    format!("エラー: 未登録のツール \"{}\" が呼び出されました。", name)
}
