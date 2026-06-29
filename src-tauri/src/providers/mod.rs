// providers/mod.rs - プロバイダ（叩く AI）コネクタのトレイトとレジストリ。
//
// 各プロバイダは Provider を実装し、inventory::submit! で自身を登録する。
// build.rs が providers/*.rs を走査して `pub mod` 宣言を自動生成するため、
// .rs を1枚置くだけでコネクタを追加できる（このファイルの編集は不要）。

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

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

    /// ストリーミング対応 chat。テキストチャンクを `on_chunk` コールバックで逐次返す。
    ///
    /// 非対応プロバイダはデフォルト実装（`chat()` を呼んで全文を一括返却）を使う。
    /// ストリーミング対応プロバイダ（openai 等）はオーバーライドして SSE を処理する。
    /// `on_chunk` は tool_calls のみの応答では呼ばれないことがある（ツール実行時は
    /// ユーザーへ見せる文字がないため）。
    async fn chat_stream(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        on_chunk: Arc<dyn Fn(String) + Send + Sync>,
    ) -> anyhow::Result<Message> {
        let msg = self.chat(cfg, model, messages, tools).await?;
        if !msg.content.is_empty() {
            on_chunk(msg.content.clone());
        }
        Ok(msg)
    }
}

/// inventory に登録するためのファクトリ。
pub struct ProviderReg {
    pub make: fn() -> Box<dyn Provider>,
}
inventory::collect!(ProviderReg);

// build.rs が生成した `pub mod ollama; pub mod openai; ...` を取り込む。
include!(concat!(env!("OUT_DIR"), "/provider_mods.rs"));

static PROVIDERS: OnceLock<Vec<Box<dyn Provider>>> = OnceLock::new();
static PROVIDER_INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();

/// 登録済みプロバイダを全て生成して返す。
pub fn all() -> &'static [Box<dyn Provider>] {
    PROVIDERS
        .get_or_init(|| {
            let mut providers: Vec<Box<dyn Provider>> = inventory::iter::<ProviderReg>
                .into_iter()
                .map(|r| (r.make)())
                .collect();
            providers.sort_by_key(|provider| (provider.id() != "local", provider.label()));
            providers
        })
        .as_slice()
}

fn index() -> &'static HashMap<&'static str, usize> {
    PROVIDER_INDEX.get_or_init(|| {
        all()
            .iter()
            .enumerate()
            .map(|(idx, provider)| (provider.id(), idx))
            .collect()
    })
}

/// id からプロバイダを取得。
pub fn get(id: &str) -> Option<&'static dyn Provider> {
    index().get(id).map(|idx| all()[*idx].as_ref())
}
