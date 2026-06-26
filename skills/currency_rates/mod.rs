// skills/currency_rates/mod.rs - Frankfurter API による為替レート取得・換算スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const FRANKFURTER_BASE: &str = "https://api.frankfurter.app";

pub struct CurrencyRatesTool;

#[async_trait]
impl Tool for CurrencyRatesTool {
    fn name(&self) -> &'static str {
        "currency_rates"
    }

    fn label(&self) -> &'static str {
        "為替"
    }

    fn description(&self) -> String {
        "Frankfurter API で為替レート取得・通貨換算を行う。旅行、買い物、請求額確認に使う。APIキー不要。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "base": { "type": "string", "description": "換算元通貨。例: USD, JPY, EUR。" },
                "symbols": { "type": "string", "description": "換算先通貨。複数はカンマ区切り。例: JPY,EUR。" },
                "amount": { "type": "number", "description": "換算金額。省略時1。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new("PICOAGENT_CURRENCY_DEFAULT_BASE", "為替の既定換算元", "text", "USD"),
            Field::new("PICOAGENT_CURRENCY_DEFAULT_SYMBOLS", "為替の既定換算先", "text", "JPY"),
            Field::new("PICOAGENT_CURRENCY_TIMEOUT_SECS", "為替API待ち時間（秒、3〜60）", "text", "15"),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let base = args
            .get("base")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or_else(|| cfg.get("PICOAGENT_CURRENCY_DEFAULT_BASE", "USD").to_ascii_uppercase());
        let symbols = args
            .get("symbols")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or_else(|| cfg.get("PICOAGENT_CURRENCY_DEFAULT_SYMBOLS", "JPY").to_ascii_uppercase());
        let amount = args.get("amount").and_then(Value::as_f64).unwrap_or(1.0);
        let timeout = cfg
            .get("PICOAGENT_CURRENCY_TIMEOUT_SECS", "15")
            .parse::<u64>()
            .ok()
            .filter(|value| (3..=60).contains(value))
            .unwrap_or(15);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("PicoAgent/1.0 (Currency tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: 為替APIの準備に失敗しました: {}", error),
        };
        let response = match client
            .get(format!("{}/latest", FRANKFURTER_BASE))
            .query(&[
                ("from", base.as_str()),
                ("to", symbols.as_str()),
                ("amount", &amount.to_string()),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return format!("エラー: 為替APIへの接続に失敗しました: {}", error),
        };
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            body
        } else {
            format!("エラー: 為替APIが HTTP {} を返しました。", status)
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(CurrencyRatesTool) } }
