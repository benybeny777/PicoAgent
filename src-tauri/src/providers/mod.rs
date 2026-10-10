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

// コネクタ登録の対象ではない共通部品は、自動走査の外へ置く。
#[path = "common/compatible.rs"]
pub(crate) mod compatible;
#[path = "common/http_error.rs"]
pub(crate) mod http_error;
pub use http_error::ProviderHttpError;

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn config_schema(&self) -> Vec<Field> {
        Vec::new()
    }
    fn default_model(&self, cfg: &Config) -> String;
    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>>;
    async fn is_up(&self, cfg: &Config) -> Result<bool, String>;
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

    /// オプション付き chat（JSON モードなど）。
    /// 非対応プロバイダはデフォルト実装（通常の `chat()` へ委譲、`json_mode` 無視）を使う。
    /// OpenAI 互換プロバイダはオーバーライドして `response_format: json_object` を付与する。
    async fn chat_with_options(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
        json_mode: bool,
    ) -> anyhow::Result<Message> {
        let _ = json_mode;
        self.chat(cfg, model, messages, tools).await
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

#[cfg(test)]
mod guard {
    #[test]
    fn 互換四社は共通通信を使い個別の再送や通信を重複させない() {
        let sources = crate::guard_scan::all_sources();
        for id in ["mistral", "deepseek", "xai", "gemini"] {
            let path = format!("src-tauri/src/providers/{id}.rs");
            let source = sources
                .iter()
                .find(|source| source.path.to_string_lossy().replace('\\', "/") == path)
                .expect("プロバイダのソースが走査対象にない");
            let body = source.body();
            assert!(
                body.contains("CompatProvider") && body.contains("inventory::submit!"),
                "{path}: 共通通信と自動登録を維持する"
            );
            for duplicate in [
                ".send()",
                ".bearer_auth(",
                "async fn chat",
                "async fn list_models",
                "impl Provider for",
            ] {
                assert!(
                    !body.contains(duplicate),
                    "{path}: 通信責務を共通層へ集める: {duplicate}"
                );
            }
        }
    }

    #[test]
    fn 再試行はエラー本文の状態らしい数字を判定しない() {
        let sources = crate::guard_scan::all_sources();
        let source = sources
            .iter()
            .find(|source| {
                source.path.to_string_lossy().replace('\\', "/") == "src-tauri/src/agent.rs"
            })
            .expect("agentが走査対象にない");
        let retry = source
            .body()
            .split("async fn chat_with_retry(")
            .nth(1)
            .expect("再試行処理がない")
            .split("const CANCEL_MESSAGE")
            .next()
            .unwrap();
        for status in ["400", "401", "403", "404", "422"] {
            assert!(
                !retry.contains(&format!(".contains(\"{status}\")")),
                "エラー本文の {status} をHTTP状態として判定している"
            );
        }
        assert!(source
            .body()
            .contains("downcast_ref::<crate::providers::ProviderHttpError>"));
    }
}
