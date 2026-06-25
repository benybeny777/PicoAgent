// providers/mod.rs - プロバイダ（叩く AI）コネクタのトレイトとレジストリ。
//
// 各プロバイダは Provider を実装し、inventory::submit! で自身を登録する。
// build.rs が providers/*.rs を走査して `pub mod` 宣言を自動生成するため、
// .rs を1枚置くだけでコネクタを追加できる（このファイルの編集は不要）。

use async_trait::async_trait;
use serde_json::Value;

use crate::config::Config;
use crate::message::{Field, Message};

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn config_schema(&self) -> Vec<Field> {
        Vec::new()
    }
    fn default_model(&self, cfg: &Config) -> String;
    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>>;
    async fn is_up(&self, cfg: &Config) -> bool;
    async fn chat(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
    ) -> anyhow::Result<Message>;
}

/// inventory に登録するためのファクトリ。
pub struct ProviderReg {
    pub make: fn() -> Box<dyn Provider>,
}
inventory::collect!(ProviderReg);

// build.rs が生成した `pub mod ollama; pub mod openai; ...` を取り込む。
include!(concat!(env!("OUT_DIR"), "/provider_mods.rs"));

/// 登録済みプロバイダを全て生成して返す。
pub fn all() -> Vec<Box<dyn Provider>> {
    inventory::iter::<ProviderReg>
        .into_iter()
        .map(|r| (r.make)())
        .collect()
}

/// id からプロバイダを取得。
pub fn get(id: &str) -> Option<Box<dyn Provider>> {
    for r in inventory::iter::<ProviderReg> {
        let p = (r.make)();
        if p.id() == id {
            return Some(p);
        }
    }
    None
}
