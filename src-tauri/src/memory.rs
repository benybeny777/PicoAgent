// memory.rs - 暗黙的な長期メモリ。
//
// 直近履歴とは別に、ユーザーの好み・自己情報・継続指示だけを保守的に
// app_data_dir/memory.json へ保存し、system 文脈へ短く差し込む。

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

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

pub fn path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(crate::app_paths::data_path(
        app,
        crate::app_paths::DataPath::Memory,
    ))
}

pub fn load(app: &AppHandle) -> anyhow::Result<MemoryStore> {
    let target = path(app)?;
    let text = match std::fs::read_to_string(&target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MemoryStore::default())
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "長期メモリを読み込めません: {} ({error})",
                target.display()
            ));
        }
    };
    match serde_json::from_str(&text) {
        Ok(store) => Ok(store),
        // フォールバック許可: PR #463 採用案3。退避と通知に成功した領域だけ新規状態で続ける。
        // そのまま次の保存で上書きすると記憶が恒久的に消えるので、退避してから続ける。
        Err(error) => {
            let saved = crate::store::quarantine_and_notify(
                app,
                &target,
                crate::store::QuarantineArea::Memory,
            )
            .ok_or_else(|| {
                anyhow::anyhow!("壊れた長期メモリを退避できません。上書きを中止しました: {error}")
            })?;
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "長期メモリを読めないため空で起動します（{error}）。破損ファイルは {} へ退避しました",
                    saved.display()
                ),
            );
            Ok(MemoryStore::default())
        }
    }
}

pub fn save_checked(app: &AppHandle, store: &MemoryStore) -> anyhow::Result<()> {
    let target = path(app)?;
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::store::write_json_atomic(&target, store)?;
    Ok(())
}

/// 設定画面で選ばれた長期メモリを1件だけ削除する。
///
/// 配列番号は会話中の更新で変わるため、画面へ返した3項目を同時に照合する。
/// 最終更新日時まで一致しなければ、古い画面から別の内容を誤削除せずエラーにする。
pub fn delete_item(
    app: &AppHandle,
    category: &str,
    text: &str,
    updated_at: &str,
) -> anyhow::Result<MemoryStore> {
    let _guard = crate::store::lock_state("memory.json");
    let mut store = load(app)?;
    if !remove_item(&mut store, category, text, updated_at) {
        anyhow::bail!("対象の長期メモリは見つからないか更新されています。画面を開き直してください");
    }
    save_checked(app, &store)?;
    Ok(store)
}

fn remove_item(store: &mut MemoryStore, category: &str, text: &str, updated_at: &str) -> bool {
    let Some(position) = store.items.iter().position(|item| {
        item.category == category && item.text == text && item.updated_at == updated_at
    }) else {
        return false;
    };
    store.items.remove(position);
    true
}

pub fn clear(app: &AppHandle) -> anyhow::Result<()> {
    let _guard = crate::store::lock_state("memory.json");
    let target = path(app)?;
    invalidate_cache(app)?;
    match std::fs::remove_file(&target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn invalidate_cache(app: &AppHandle) -> anyhow::Result<()> {
    let vectors = crate::app_paths::data_path(app, crate::app_paths::DataPath::MemoryVectors);
    for target in [vectors.clone(), vectors.with_extension("corrupt")] {
        match std::fs::remove_file(target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn context(app: &AppHandle, cfg: &Config) -> String {
    if !cfg.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
        return String::new();
    }
    match load(app) {
        Ok(store) => context_from_store(&store),
        Err(error) => {
            crate::log_paths::append_app_log(app, &error.to_string());
            String::new()
        }
    }
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

/// 意味の近い記憶をEmbeddingGemma 2で順位付けする。原本はRust側で保持する。
pub async fn context_for_query(
    app: &AppHandle,
    cfg: &Config,
    query: &str,
) -> anyhow::Result<String> {
    if !cfg.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
        return Ok(String::new());
    }
    let generation = crate::memory_embedding::generation();
    let store = load(app)?;
    if store.items.is_empty() {
        return Ok(String::new());
    }
    let docs: Vec<String> = store.items.iter().map(|item| item.text.clone()).collect();
    let order = crate::memory_embedding::search(app, cfg, &docs, query, generation).await?;
    if generation != crate::memory_embedding::generation() {
        return Err(crate::memory_embedding::interrupted());
    }
    let lines: Vec<String> = order
        .into_iter()
        .take(MAX_CONTEXT_ITEMS)
        .map(|index| {
            format!(
                "- [{}] {}",
                store.items[index].category, store.items[index].text
            )
        })
        .collect();
    Ok(format!(
        "ユーザーについて会話から暗黙的に覚えた長期メモリです（意味の近い順）。関連しない記憶を回答へ無理に使わないでください。古い内容と今回の発話が矛盾する場合は今回の発話を優先してください。\n{}",
        lines.join("\n")
    ))
}

pub fn update_from_turn(
    app: &AppHandle,
    cfg: &Config,
    user_text: &str,
) -> Result<Option<String>, String> {
    update_with(
        cfg,
        user_text,
        || load(app),
        |store| save_checked(app, store),
    )
}

fn update_with(
    cfg: &Config,
    user_text: &str,
    read: impl FnOnce() -> anyhow::Result<MemoryStore>,
    write: impl FnOnce(&MemoryStore) -> anyhow::Result<()>,
) -> Result<Option<String>, String> {
    if !cfg.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
        return Ok(None);
    }
    let candidates = extract_candidates(user_text);
    if candidates.is_empty() {
        return Ok(None);
    }
    // 読む→直す→書き戻すの間に別の会話と競合すると、片方が覚えた内容が消える。
    let _guard = crate::store::lock_state("memory.json");
    let mut store = read().map_err(|error| error.to_string())?;
    let before = serde_json::to_string(&store).map_err(|error| error.to_string())?;
    merge_candidates(&mut store, candidates, max_items(cfg));
    let after = serde_json::to_string(&store).map_err(|error| error.to_string())?;
    if before == after {
        return Ok(None);
    }
    write(&store).map_err(|error| error.to_string())?;
    Ok(Some(context_from_store(&store)))
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

/// 会話文を1文ずつに割る。`project_rules` の強調検出も同じ割り方を使う
/// （別に実装すると、片方だけが箇条書きや改行の扱いを取りこぼす）。
pub(crate) fn split_sentences(text: &str) -> Vec<String> {
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
    use super::{context_from_store, extract_candidates, remove_item, MemoryItem, MemoryStore};

    #[test]
    fn 読込失敗では保存せず保存失敗では覚えた文脈を返さない() {
        let cfg = crate::config::Config::new();
        let result = super::update_with(
            &cfg,
            "私はRustが好きです",
            || anyhow::bail!("読込不可"),
            |_| panic!("保存してはいけない"),
        );
        assert!(result.unwrap_err().contains("読込不可"));
        let directory = tempfile::tempdir().unwrap();
        // 書込先と同名のディレクトリは全OSで保存不能となり、権限変更より確実に再現できる。
        let target = directory.path().join("memory.json");
        std::fs::create_dir(&target).unwrap();
        let result = super::update_with(
            &cfg,
            "私はRustが好きです",
            || Ok(MemoryStore::default()),
            |store| crate::store::write_json_atomic(&target, store),
        );
        assert!(
            result.is_err(),
            "保存できない記憶の文脈を公開してはいけない"
        );
        let result = super::update_with(
            &cfg,
            "私はRustが好きです",
            || Ok(MemoryStore::default()),
            |_| Ok(()),
        );
        assert!(result.unwrap().unwrap().contains("私はRustが好きです"));
    }

    #[test]
    fn 候補なしの発話では読込も保存も試さない() {
        let cfg = crate::config::Config::new();
        let result = super::update_with(
            &cfg,
            "こんにちは",
            || panic!("読込を試してはいけない"),
            |_| panic!("保存を試してはいけない"),
        );
        assert!(matches!(result, Ok(None)));
    }

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

    #[test]
    fn 指定したメモリだけを削除して文脈から除外する() {
        let mut store = MemoryStore {
            items: vec![
                MemoryItem {
                    category: "好み".to_string(),
                    text: "私はRustをよく使う".to_string(),
                    updated_at: "2026-06-26T00:00:00Z".to_string(),
                    hits: 3,
                },
                MemoryItem {
                    category: "継続指示".to_string(),
                    text: "今後は結論から話してほしい".to_string(),
                    updated_at: "2026-06-27T00:00:00Z".to_string(),
                    hits: 1,
                },
            ],
        };

        assert!(remove_item(
            &mut store,
            "好み",
            "私はRustをよく使う",
            "2026-06-26T00:00:00Z"
        ));
        let context = context_from_store(&store);
        assert!(!context.contains("私はRustをよく使う"));
        assert!(context.contains("今後は結論から話してほしい"));
    }

    #[test]
    fn 更新後のメモリを古い日時で誤削除しない() {
        let mut store = MemoryStore {
            items: vec![MemoryItem {
                category: "好み".to_string(),
                text: "私はRustをよく使う".to_string(),
                updated_at: "2026-06-27T00:00:00Z".to_string(),
                hits: 2,
            }],
        };

        assert!(!remove_item(
            &mut store,
            "好み",
            "私はRustをよく使う",
            "2026-06-26T00:00:00Z"
        ));
        assert_eq!(store.items.len(), 1);
    }
}
