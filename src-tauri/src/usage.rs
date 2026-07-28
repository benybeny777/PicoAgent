// usage.rs - トークン使用量とコストの追跡（90日間保持、月次集計）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRecord {
    pub provider_id: String,
    pub model: String,
    pub usage: TokenUsage,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageStore {
    pub records: Vec<UsageRecord>,
}

impl UsageStore {
    /// 90日より古いレコードを削除する。
    pub fn prune(&mut self) {
        let cutoff = Utc::now() - chrono::Duration::days(90);
        self.records.retain(|r| {
            r.recorded_at
                .parse::<DateTime<Utc>>()
                .map(|t| t > cutoff)
                .unwrap_or(false)
        });
    }

    /// 今月のプロバイダ/モデル別トークン合計。
    pub fn monthly_summary(&self) -> Vec<serde_json::Value> {
        use std::collections::HashMap;
        let now = Utc::now();
        let month_prefix = format!("{}-{:02}", now.format("%Y"), now.format("%m"));
        let mut totals: HashMap<String, (u32, u32)> = HashMap::new();
        for r in &self.records {
            if r.recorded_at.starts_with(&month_prefix) {
                let key = format!("{}/{}", r.provider_id, r.model);
                let entry = totals.entry(key).or_default();
                entry.0 += r.usage.prompt_tokens;
                entry.1 += r.usage.completion_tokens;
            }
        }
        let mut result: Vec<serde_json::Value> = totals
            .into_iter()
            .map(|(key, (prompt, completion))| {
                serde_json::json!({
                    "key": key,
                    "promptTokens": prompt,
                    "completionTokens": completion,
                    "totalTokens": prompt + completion,
                })
            })
            .collect();
        result.sort_by(|a, b| {
            b["totalTokens"]
                .as_u64()
                .unwrap_or(0)
                .cmp(&a["totalTokens"].as_u64().unwrap_or(0))
        });
        result
    }

    /// 累計トークン数（全期間）。
    pub fn total_tokens(&self) -> u64 {
        self.records
            .iter()
            .map(|r| r.usage.total_tokens as u64)
            .sum()
    }
}

fn path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("usage.json")
}

pub fn load(app: &AppHandle) -> UsageStore {
    let Ok(text) = std::fs::read_to_string(path(app)) else {
        return UsageStore::default();
    };
    // フォールバック許可: 使用量ファイル破損時は空ストアで継続する。
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(app: &AppHandle, store: &UsageStore) {
    let target = path(app);
    let result = (|| -> anyhow::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::store::write_json_atomic(&target, store)?;
        Ok(())
    })();
    // フォールバック許可: 使用量保存失敗は会話を止めずログだけ残す。
    if let Err(e) = result {
        crate::log_paths::append_app_log(app, &format!("トークン使用量の保存に失敗しました: {e}"));
    }
}

/// 1回の chat 呼び出しのトークン使用量を記録する。total_tokens が 0 の場合は無視する。
pub fn record(app: &AppHandle, provider_id: &str, model: &str, usage: TokenUsage) {
    if usage.total_tokens == 0 && usage.prompt_tokens == 0 {
        return;
    }
    let mut store = load(app);
    store.records.push(UsageRecord {
        provider_id: provider_id.to_string(),
        model: model.to_string(),
        usage,
        recorded_at: Utc::now().to_rfc3339(),
    });
    store.prune();
    save(app, &store);
}

#[cfg(test)]
mod tests {
    use super::{TokenUsage, UsageRecord, UsageStore};

    #[test]
    fn 月次集計が正しく動作する() {
        let now = chrono::Utc::now().to_rfc3339();
        let store = UsageStore {
            records: vec![
                UsageRecord {
                    provider_id: "openai".to_string(),
                    model: "gpt-4".to_string(),
                    usage: TokenUsage {
                        prompt_tokens: 100,
                        completion_tokens: 50,
                        total_tokens: 150,
                    },
                    recorded_at: now.clone(),
                },
                UsageRecord {
                    provider_id: "openai".to_string(),
                    model: "gpt-4".to_string(),
                    usage: TokenUsage {
                        prompt_tokens: 200,
                        completion_tokens: 100,
                        total_tokens: 300,
                    },
                    recorded_at: now,
                },
            ],
        };
        let summary = store.monthly_summary();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0]["totalTokens"], 450);
    }
}
