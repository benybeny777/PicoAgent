// memory.rs - 暗黙的な長期メモリ。
//
// 直近履歴とは別に、ユーザーの好み・自己情報・継続指示だけを保守的に
// app_data_dir/memory.json へ保存し、system 文脈へ短く差し込む。

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::config::Config;

const MAX_ITEMS_DEFAULT: usize = 80;
const MAX_CONTEXT_ITEMS: usize = 24;
const MAX_ITEM_CHARS: usize = 140;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemoryItem {
    pub category: String,
    pub text: String,
    pub updated_at: String,
    pub hits: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MemoryStore {
    pub items: Vec<MemoryItem>,
}

pub fn path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("memory.json")
}

pub fn load(app: &AppHandle) -> MemoryStore {
    let target = path(app);
    let Ok(text) = std::fs::read_to_string(&target) else {
        return MemoryStore::default();
    };
    match serde_json::from_str(&text) {
        Ok(store) => store,
        // フォールバック許可: 破損時も会話継続を優先し、空メモリで起動する。ただし
        // そのまま次の保存で上書きすると記憶が恒久的に消えるので、退避してから続ける。
        Err(error) => {
            let saved = crate::store::quarantine_corrupt(&target);
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "長期メモリを読めないため空で起動します（{error}）{}",
                    saved
                        .map(|path| format!("。破損ファイルは {} へ退避しました", path.display()))
                        .unwrap_or_default()
                ),
            );
            MemoryStore::default()
        }
    }
}

pub fn save(app: &AppHandle, store: &MemoryStore) {
    let target = path(app);
    let result = (|| -> anyhow::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::store::write_json_atomic(&target, store)?;
        Ok(())
    })();
    // フォールバック許可: メモリ保存失敗は会話を止めず、ログのみに留める。
    if let Err(error) = result {
        crate::log_paths::append_app_log(app, &format!("長期メモリの保存に失敗しました: {error}"));
    }
}

pub fn clear(app: &AppHandle) {
    let _ = std::fs::remove_file(path(app));
}

pub fn context(app: &AppHandle, cfg: &Config) -> String {
    if !cfg.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
        return String::new();
    }
    context_from_store(&load(app))
}

pub fn context_from_store(store: &MemoryStore) -> String {
    if store.items.is_empty() {
        return String::new();
    }
    let mut items = store.items.clone();
    items.sort_by(|a, b| {
        b.hits
            .cmp(&a.hits)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    let lines: Vec<String> = items
        .into_iter()
        .take(MAX_CONTEXT_ITEMS)
        .map(|item| format!("- [{}] {}", item.category, item.text))
        .collect();
    format!(
        "ユーザーについて会話から暗黙的に覚えた長期メモリです。回答の参考にしてください。古い内容と今回の発話が矛盾する場合は今回の発話を優先してください。\n{}",
        lines.join("\n")
    )
}

/// クエリテキストとのセマンティック類似度でメモリを絞り込んで返す。
/// バイグラム（2文字の連続ペア）のジャカード係数でスコアリングし、
/// 関連度が高い順に最大 MAX_CONTEXT_ITEMS 件を返す。
/// 関連するメモリが 0 件の場合は通常の `context_from_store` へフォールバックする。
pub fn context_for_query(app: &tauri::AppHandle, cfg: &Config, query: &str) -> String {
    if !cfg.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
        return String::new();
    }
    let store = load(app);
    context_from_store_for_query(&store, query)
}

pub fn context_from_store_for_query(store: &MemoryStore, query: &str) -> String {
    if store.items.is_empty() {
        return String::new();
    }
    let query_bg = char_bigrams(query);

    let mut scored: Vec<(f32, &MemoryItem)> = store
        .items
        .iter()
        .map(|item| {
            let item_bg = char_bigrams(&item.text);
            let intersection = query_bg.intersection(&item_bg).count();
            let union = query_bg.union(&item_bg).count();
            let jaccard = if union == 0 {
                0.0f32
            } else {
                intersection as f32 / union as f32
            };
            // ヒット数ボーナス（よく言及される内容を優先）。
            let bonus = (item.hits as f32 * 0.05).min(0.2);
            (jaccard + bonus, item)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let relevant: Vec<String> = scored
        .iter()
        .filter(|(score, _)| *score > 0.05) // 類似度閾値（低すぎると全件返す）
        .take(MAX_CONTEXT_ITEMS)
        .map(|(_, item)| format!("- [{}] {}", item.category, item.text))
        .collect();

    if relevant.is_empty() {
        // 関連メモリがなければ全件（ヒット順）へフォールバックする。
        return context_from_store(store);
    }

    format!(
        "ユーザーについて会話から暗黙的に覚えた長期メモリです（関連度順）。回答の参考にしてください。古い内容と今回の発話が矛盾する場合は今回の発話を優先してください。\n{}",
        relevant.join("\n")
    )
}

/// 文字列からバイグラム集合を生成する（ASCII 以外の意味のある文字のみ対象）。
fn char_bigrams(text: &str) -> std::collections::HashSet<(char, char)> {
    let chars: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace() && (c.is_alphanumeric() || !c.is_ascii()))
        .collect();
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

pub fn update_from_turn(app: &AppHandle, cfg: &Config, user_text: &str) -> Option<String> {
    if !cfg.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
        return None;
    }
    let candidates = extract_candidates(user_text);
    if candidates.is_empty() {
        return None;
    }
    let mut store = load(app);
    let before = serde_json::to_string(&store).unwrap_or_default();
    merge_candidates(&mut store, candidates, max_items(cfg));
    let after = serde_json::to_string(&store).unwrap_or_default();
    if before == after {
        return None;
    }
    save(app, &store);
    Some(context_from_store(&store))
}

fn max_items(cfg: &Config) -> usize {
    cfg.get("PICOAGENT_MEMORY_MAX_ITEMS", &MAX_ITEMS_DEFAULT.to_string())
        .parse::<usize>()
        .map(|value| value.clamp(10, 200))
        .unwrap_or(MAX_ITEMS_DEFAULT)
}

fn merge_candidates(store: &mut MemoryStore, candidates: Vec<(String, String)>, max_items: usize) {
    let now = Utc::now().to_rfc3339();
    for (category, text) in candidates {
        let key = normalize(&text);
        if key.is_empty() {
            continue;
        }
        if let Some(item) = store
            .items
            .iter_mut()
            .find(|item| normalize(&item.text) == key)
        {
            item.hits = item.hits.saturating_add(1);
            item.updated_at = now.clone();
            continue;
        }
        store.items.push(MemoryItem {
            category,
            text,
            updated_at: now.clone(),
            hits: 1,
        });
    }
    store.items.sort_by(|a, b| {
        b.hits
            .cmp(&a.hits)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    if store.items.len() > max_items {
        store.items.truncate(max_items);
    }
}

fn extract_candidates(text: &str) -> Vec<(String, String)> {
    split_sentences(text)
        .into_iter()
        .filter_map(|sentence| classify_sentence(&sentence).map(|category| (category, sentence)))
        .collect()
}

fn split_sentences(text: &str) -> Vec<String> {
    let normalized = text.replace('\r', "\n");
    let mut out = Vec::new();
    for chunk in normalized.split(['\n', '。', '！', '？', '!', '?']) {
        let sentence = chunk
            .trim()
            .trim_matches(|c: char| c.is_whitespace() || "、，,.：:「」『』（）()[]".contains(c));
        let count = sentence.chars().count();
        if (4..=MAX_ITEM_CHARS).contains(&count) {
            out.push(sentence.to_string());
        }
    }
    out
}

fn classify_sentence(sentence: &str) -> Option<String> {
    if sentence.contains("http://") || sentence.contains("https://") || sentence.contains("```") {
        return None;
    }
    let user_markers = [
        "私は",
        "僕は",
        "俺は",
        "自分は",
        "わたしは",
        "ぼくは",
        "名前は",
        "呼んで",
    ];
    let preference_markers = [
        "好き",
        "嫌い",
        "苦手",
        "得意",
        "好み",
        "普段",
        "よく使う",
        "いつも",
        "デフォルト",
    ];
    let instruction_markers = [
        "覚えて",
        "今後",
        "これから",
        "してほしい",
        "しないで",
        "読まないで",
        "読み上げないで",
        "参照して",
        "優先して",
    ];
    if contains_any(sentence, &instruction_markers) {
        return Some("継続指示".to_string());
    }
    if contains_any(sentence, &user_markers) && contains_any(sentence, &preference_markers) {
        return Some("好み".to_string());
    }
    if contains_any(sentence, &user_markers) {
        return Some("ユーザー情報".to_string());
    }
    if contains_any(sentence, &preference_markers) && sentence.contains("ほしい") {
        return Some("好み".to_string());
    }
    None
}

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !"、。，,.！!？?：:".contains(*c))
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{context_from_store, extract_candidates, MemoryItem, MemoryStore};

    #[test]
    fn 自己情報と継続指示だけを抽出する() {
        let items = extract_candidates(
            "今日は暑いです。私はRustをよく使う。参考の部分は読み上げないでほしい。",
        );
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|(_, text)| text == "私はRustをよく使う"));
        assert!(items.iter().any(|(category, _)| category == "継続指示"));
    }

    #[test]
    fn メモリ文脈をsystem向けに整形する() {
        let store = MemoryStore {
            items: vec![MemoryItem {
                category: "好み".to_string(),
                text: "私はRustをよく使う".to_string(),
                updated_at: "2026-06-26T00:00:00Z".to_string(),
                hits: 1,
            }],
        };
        let context = context_from_store(&store);
        assert!(context.contains("長期メモリ"));
        assert!(context.contains("[好み] 私はRustをよく使う"));
    }
}
