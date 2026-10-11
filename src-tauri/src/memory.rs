// memory.rs - 暗黙的な長期メモリ。
//
// 直近履歴とは別に、ユーザーの好み・自己情報・継続指示だけを保守的に
// app_data_dir/memory.json へ保存し、system 文脈へ短く差し込む。

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::config::Config;

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
        // フォールバック許可: PR #463 採用案3。退避と通知に成功した領域だけ新規状態で続ける。（根拠: PR #463 採用案。退避・通知と原本保護）
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
    let unresolved = items
        .iter()
        .filter(|item| memory_subject(&item.text) == MemorySubject::Unresolved)
        .count();
    format_memory_context(items.iter(), unresolved, false)
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
    let items: Vec<&MemoryItem> = store
        .items
        .iter()
        .filter(|item| memory_subject(&item.text) != MemorySubject::Unresolved)
        .collect();
    let unresolved = store.items.len() - items.len();
    if items.is_empty() {
        return Ok(format_memory_context(items, unresolved, true));
    }
    let docs: Vec<String> = items.iter().map(|item| item.text.clone()).collect();
    let order = crate::memory_embedding::search(app, cfg, &docs, query, generation).await?;
    if generation != crate::memory_embedding::generation() {
        return Err(crate::memory_embedding::interrupted());
    }
    let ordered = order.into_iter().map(|index| items[index]);
    Ok(format_memory_context(ordered, unresolved, true))
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
    let limit = max_items(cfg)?;
    // 読む→直す→書き戻すの間に別の会話と競合すると、片方が覚えた内容が消える。
    let _guard = crate::store::lock_state("memory.json");
    let mut store = read().map_err(|error| error.to_string())?;
    let before = serde_json::to_string(&store).map_err(|error| error.to_string())?;
    merge_candidates(&mut store, candidates, limit);
    let after = serde_json::to_string(&store).map_err(|error| error.to_string())?;
    if before == after {
        return Ok(None);
    }
    write(&store).map_err(|error| error.to_string())?;
    Ok(Some(context_from_store(&store)))
}

fn max_items(cfg: &Config) -> Result<usize, String> {
    cfg.int(&crate::config::ints::MEMORY_MAX_ITEMS)
        .map(|value| value as usize)
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum MemorySubject {
    UserFact,
    Instruction,
    Unresolved,
}

const SENTENCE_SEPARATORS: [char; 7] = ['\n', '。', '！', '？', '!', '?', '\r'];

// 「私の」は所有者だけを示す。「私の兄」の述語を利用者自身の事実へ移さない。
fn user_owned_attribute(body: &str) -> bool {
    let Some((attribute, value)) = body.split_once('は') else {
        return false;
    };
    let direct = [
        "名前",
        "年齢",
        "職業",
        "出身",
        "住所",
        "誕生日",
        "好み",
        "趣味",
    ]
    .contains(&attribute);
    let qualified = [
        "好きな",
        "嫌いな",
        "苦手な",
        "得意な",
        "普段使う",
        "よく使う",
    ]
    .into_iter()
    .find_map(|prefix| attribute.strip_prefix(prefix));
    // 好みの参照先が人か物か不明な場合は、名詞＋ですだけで名前と属性を推定しない。
    // 飲み物等の直接の好み、または参照先の「名前」という明示同定だけを扱う。
    let explicit_reference = qualified.is_some_and(|reference| {
        [
            "飲み物",
            "食べ物",
            "料理",
            "色",
            "音楽",
            "本",
            "作品",
            "場所",
            "ゲーム",
            "言語",
            "ツール",
            "ソフト",
            "アプリ",
            "道具",
        ]
        .contains(&reference)
            || reference
                .strip_suffix("の名前")
                .is_some_and(|referent| !referent.is_empty() && !referent.contains('の'))
    });
    (direct || explicit_reference) && !value.trim().is_empty()
        // 属性の値が更に別人の好み等を述べるなら、確定した利用者の属性とは扱わない。
        && !contains_any(value, &["好き", "嫌い", "苦手", "得意", "住んで", "勤めて", "働いて"])
}

// 報告する人の一人称だけでは、報告内の事実の主体を確定できない。
fn reported_or_uncertain(text: &str) -> bool {
    // 並列助詞の「と」や名詞の一文字を、報告の文形・動詞として扱わない。
    let reporting_verb = |rest: &str| {
        [
            "聞く",
            "聞き",
            "聞い",
            "聞け",
            "聞か",
            "知る",
            "知り",
            "知っ",
            "知れ",
            "知ら",
            "思う",
            "思い",
            "思っ",
            "思え",
            "思わ",
            "考える",
            "考えた",
            "考えて",
            "考えます",
            "言う",
            "言い",
            "言っ",
            "言え",
            "言わ",
            "話す",
            "話し",
            "話せ",
            "話さ",
            "見る",
            "見た",
            "見て",
            "見ます",
            "読む",
            "読み",
            "読ん",
            "読め",
            "読ま",
            "理解する",
            "理解し",
            "認識する",
            "認識し",
            "記憶する",
            "記憶し",
            "覚える",
            "覚えた",
            "覚えて",
            "覚えます",
            "教わる",
            "教わり",
            "教わっ",
            "教えられ",
            "感じる",
            "感じた",
            "感じて",
        ]
        .into_iter()
        .any(|verb| rest.starts_with(verb))
    };
    ["だと", "ですと", "ことを", "のを"]
        .into_iter()
        .any(|link| text.split(link).skip(1).any(reporting_verb))
        // 「好きと聞いた」のような省略形は、直前が好み等の述語の場合だけ扱う。
        || text.match_indices('と').any(|(index, link)| {
            let (before, after) = text.split_at(index);
            ["好き", "嫌い", "得意", "苦手"]
                .into_iter()
                .any(|predicate| before.ends_with(predicate))
                && reporting_verb(&after[link.len()..])
        })
        || contains_any(
            text,
            &[
                "とのこと",
                "という",
                "らしい",
                "そうです",
                "そうだ",
                "ようです",
                "ようだ",
            ],
        )
}

// 保存時と旧記録の参照時で同じ主体判定を使い、発話者と事実の主体を混同しない。
fn memory_subject(text: &str) -> MemorySubject {
    let text = text.trim();
    if reported_or_uncertain(text) {
        return MemorySubject::Unresolved;
    }
    if text.contains([
        '「', '」', '『', '』', '"', '“', '”', '‘', '’', '\'', '`', '?', '？',
    ]) || contains_any(
        text,
        &[
            "あなた",
            "キャラクター",
            "友人",
            "友達",
            "家族",
            "同僚",
            "上司",
            "部下",
            "彼の",
            "彼は",
            "彼が",
            "彼女",
            "君は",
            "君の",
            "母は",
            "母の",
            "父は",
            "父の",
            "妻は",
            "夫は",
            "教えて",
            "知りたい",
            "どんな",
            "何が",
            "ですか",
            "ますか",
            "なのですか",
            "でしょうか",
        ],
    ) {
        return MemorySubject::Unresolved;
    }
    let subject = [
        "私は",
        "僕は",
        "俺は",
        "自分は",
        "わたしは",
        "ぼくは",
        "私の",
        "僕の",
        "俺の",
        "自分の",
        "わたしの",
        "ぼくの",
        "利用者は",
        "利用者の",
        "ユーザーは",
        "ユーザーの",
    ]
    .into_iter()
    .find_map(|prefix| text.strip_prefix(prefix).map(|body| (prefix, body)));
    if let Some((prefix, body)) = subject {
        if prefix.ends_with('の') && !user_owned_attribute(body) {
            return MemorySubject::Unresolved;
        }
    }
    let subject = subject.map(|(_, body)| body);
    if let Some(body) = subject {
        // 複数の節を丸ごと一人の事実へ認定しない。本文は変更せず保存候補から外す。
        if body.contains(['、', ',', ';', '；'])
            || contains_any(
                body,
                &["私は", "僕は", "俺は", "自分は", "わたしは", "ぼくは"],
            )
            || [
                "好きで",
                "嫌いで",
                "苦手で",
                "得意で",
                "ですが",
                "だけど",
                "一方",
            ]
            .into_iter()
            .any(|connector| {
                body.split_once(connector)
                    .is_some_and(|(_, tail)| tail.contains(['は', 'が']))
            })
        {
            return MemorySubject::Unresolved;
        }
    }
    // 「覚えて」が添えられても、他人の好みを純粋な継続指示へ格上げしない。
    if subject.is_none()
        && contains_any(text, &["好き", "嫌い", "苦手", "得意", "名前は", "住んで"])
    {
        return MemorySubject::Unresolved;
    }
    let instruction = contains_any(
        text,
        &[
            "覚えて",
            "今後",
            "これから",
            "してほしい",
            "しないで",
            "読まないで",
            "読み上げないで",
            "参照して",
            "優先して",
        ],
    ) && contains_any(
        text,
        &[
            "ほしい",
            "ください",
            "覚えて",
            "しないで",
            "読まないで",
            "読み上げないで",
            "参照して",
            "優先して",
        ],
    );
    if instruction {
        MemorySubject::Instruction
    } else if subject.is_some() {
        MemorySubject::UserFact
    } else {
        MemorySubject::Unresolved
    }
}

fn format_memory_context<'a>(
    items: impl IntoIterator<Item = &'a MemoryItem>,
    unresolved: usize,
    semantic: bool,
) -> String {
    let lines: Vec<String> = items
        .into_iter()
        .filter_map(|item| {
            let owner = match memory_subject(&item.text) {
                MemorySubject::UserFact => "利用者の情報",
                MemorySubject::Instruction => "利用者の継続指示",
                MemorySubject::Unresolved => return None,
            };
            Some(format!("- [{owner}] [{}] {}", item.category, item.text))
        })
        .take(MAX_CONTEXT_ITEMS)
        .collect();
    if lines.is_empty() && unresolved == 0 {
        return String::new();
    }
    let order = if semantic {
        "（意味の近い順）"
    } else {
        ""
    };
    let mut context = format!("長期メモリ{order}です。利用者の情報と継続指示を区別し、キャラクター自身の人格や好みへ移さないでください。関連しない記録を回答へ無理に使わず、今回の発話を優先してください。\n{}", lines.join("\n"));
    if unresolved > 0 {
        context.push_str(&format!("\n主体を確認できない旧記録{unresolved}件は原本を保持し、確定した利用者情報として参照していません。"));
    }
    context
}

fn extract_candidates(text: &str) -> Vec<(String, String)> {
    // 引用内の句点で分割して「私は…」だけを取り出さない。共通の分割規則は維持する。
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut quotes = crate::text::QuoteState::default();
    for character in text.chars() {
        let quoted = quotes.enter(character);
        if SENTENCE_SEPARATORS.contains(&character) && !quoted {
            chunks.push(std::mem::take(&mut chunk));
            // 疑問符も主体判定まで保持する。
            if matches!(character, '?' | '？') {
                if let Some(last) = chunks.last_mut() {
                    last.push(character);
                }
            }
            continue;
        }
        chunk.push(character);
    }
    chunks.push(chunk);
    chunks
        .into_iter()
        .filter(|chunk| memory_subject(chunk) != MemorySubject::Unresolved)
        .flat_map(|chunk| split_sentences(&chunk))
        .filter_map(|sentence| classify_sentence(&sentence).map(|category| (category, sentence)))
        .collect()
}

/// 会話文を1文ずつに割る。`project_rules` の強調検出も同じ割り方を使う
/// （別に実装すると、片方だけが箇条書きや改行の扱いを取りこぼす）。
pub(crate) fn split_sentences(text: &str) -> Vec<String> {
    let normalized = text.replace('\r', "\n");
    let mut out = Vec::new();
    for chunk in normalized.split(SENTENCE_SEPARATORS) {
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
    let subject = memory_subject(sentence);
    if subject == MemorySubject::Unresolved {
        return None;
    }
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
    if subject == MemorySubject::Instruction {
        return Some("継続指示".to_string());
    }
    if contains_any(sentence, &preference_markers) {
        return Some("好み".to_string());
    }
    if subject == MemorySubject::UserFact {
        return Some("ユーザー情報".to_string());
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
    fn 不正な保持件数では記憶を読まず保存しない() {
        let cfg = crate::config::Config::new();
        for value in ["9", "201", "abc", "1.5", "-1"] {
            cfg.set("PICOAGENT_MEMORY_MAX_ITEMS", value);
            let result = super::update_with(
                &cfg,
                "私はRustが好きです",
                || panic!("不正設定で読込してはいけない"),
                |_| panic!("不正設定で保存してはいけない"),
            );
            assert!(result.unwrap_err().contains("PICOAGENT_MEMORY_MAX_ITEMS"));
        }
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
    fn 引用と他人への質問と複数主体を利用者の事実として抽出しない() {
        for text in [
            "友人が「私は緑茶が好き」と話した",
            "「私は緑茶が好き。私はコーヒーが苦手」と友人が言った",
            "あなたの名前はむぎ",
            "あなたは緑茶が好きなのですか？",
            "私は緑茶が好きですか？",
            "私の好きな飲み物を教えて",
            "私は緑茶が好きで、友人はコーヒーが好き",
            "私は緑茶が好きで、太郎はコーヒーが好き",
            "彼の名前は太郎で、覚えてほしい",
            "太郎は緑茶が好きなので今後覚えてほしい",
        ] {
            assert!(
                extract_candidates(text).is_empty(),
                "誤った主体を保存した: {text}"
            );
        }
        let items = extract_candidates("私は緑茶が好きです。友人が「私はコーヒーが好き」と言った。今後は結論から話してほしい。");
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            ("好み".to_string(), "私は緑茶が好きです".to_string())
        );
        assert_eq!(items[1].0, "継続指示");
    }

    #[test]
    fn 私の所有物と第三者の述語を区別し明示した自分の属性だけを覚える() {
        for text in [
            "私の兄はコーヒーが好き",
            "私の母が緑茶好き",
            "僕の同級生は緑茶好き",
            "私の好きな作家は太郎です",
            "私の好きな作家は東京出身です",
            "私の好きな歌手は花子です",
            "私の好きな先生の兄の名前は太郎です",
            "私の好きな作家はコーヒーが好き",
            "私の好きな作家はコーヒー好き",
            "私の好きな作家の妹は緑茶好き",
            "私の趣味の先生は緑茶が好き",
            "利用者の父はコーヒーが好き",
        ] {
            assert!(
                extract_candidates(text).is_empty(),
                "所有名詞の第三者を利用者とした: {text}"
            );
            assert!(
                super::memory_subject(text) == super::MemorySubject::Unresolved,
                "旧記録でも同じ主体を拒否する: {text}"
            );
        }
        for text in [
            "私の好きな飲み物は緑茶です",
            "僕の名前は太郎です",
            "私の好きな作家の名前は太郎です",
            "私の好きな歌手の名前は花子です",
            "私の趣味は読書です",
        ] {
            assert_eq!(
                extract_candidates(text).len(),
                1,
                "明示属性を失った: {text}"
            );
        }
    }

    #[test]
    fn コード引用内の自己紹介を保存せず引用外の独立した自己情報だけを保存する() {
        for text in [
            "`私は緑茶が好き。私はコーヒーが苦手`",
            "``私は緑茶が好き。`私は紅茶が好き``",
            "```text\n私は緑茶が好き。私はコーヒーが苦手\n```",
        ] {
            assert!(
                extract_candidates(text).is_empty(),
                "コード資料を本人の情報とした: {text}"
            );
        }
        let items = extract_candidates("```text\n私は緑茶が好き\n```。私はRustが好きです。");
        assert_eq!(
            items,
            vec![("好み".to_string(), "私はRustが好きです".to_string())]
        );
    }

    #[test]
    fn 自己主語の報告や伝聞を本人の好みへ移さず直接の自己申告を保持する() {
        for text in [
            "私は兄がコーヒー好きだと聞いた",
            "私は兄がコーヒー好きと聞きました",
            "私は兄が緑茶好きだと記憶しています",
            "私は兄が緑茶好きなのを覚えている",
            "私はあおいが料理得意とのことを記憶した",
            "私はあおいが緑茶嫌いと知りました",
            "私はあおいが料理得意と聞きました",
            "私はあおいが運動苦手と聞いています",
            "私はあおいが緑茶好きだと知っています",
            "僕はあおいが緑茶を好きだと聞きました",
            "私はあおいがコーヒーが好きだと知りました",
            "私はあおいが緑茶好きらしいと思います",
            "私はあおいが緑茶好きとのことを聞いています",
            "私はあおいが緑茶好きなことを知っている",
            "私はあおいが緑茶好きという話を覚えてほしい",
        ] {
            assert!(
                extract_candidates(text).is_empty(),
                "報告の主体と事実の主体を混同した: {text}"
            );
            assert!(
                super::memory_subject(text) == super::MemorySubject::Unresolved,
                "旧記録でも伝聞を確定しない: {text}"
            );
        }
        for text in [
            "私は緑茶が好きです",
            "私は散歩と思索が好きです",
            "私は人と話すのが好きです",
            "私は人の名前を覚えるのが得意です",
        ] {
            assert_eq!(
                extract_candidates(text),
                vec![("好み".to_string(), text.to_string())],
                "直接の自己申告を伝聞へ読み替えない: {text}"
            );
        }
    }

    #[test]
    fn 旧記憶は原本を保持し確認できた主体と指示だけを区別して参考へ渡す() {
        let texts = [
            ("好み", "私は緑茶が好き"),
            ("継続指示", "今後は結論から話してほしい"),
            ("ユーザー情報", "あなたの名前はむぎ"),
            ("好み", "私は緑茶が好きで、太郎はコーヒーが好き"),
            ("好み", "「私は紅茶が好き」と友人が言った"),
            ("好み", "緑茶が好き"),
        ];
        let store = MemoryStore {
            items: texts
                .into_iter()
                .map(|(category, text)| MemoryItem {
                    category: category.into(),
                    text: text.into(),
                    updated_at: "2026-10-11T00:00:00Z".into(),
                    hits: 1,
                })
                .collect(),
        };
        let before = serde_json::to_string(&store).unwrap();
        let context = context_from_store(&store);
        assert!(context.contains("[利用者の情報] [好み] 私は緑茶が好き"));
        assert!(context.contains("[利用者の継続指示] [継続指示] 今後は結論から話してほしい"));
        for excluded in ["あなたの名前はむぎ", "太郎", "紅茶", "- [好み] 緑茶が好き"]
        {
            assert!(
                !context.contains(excluded),
                "未確認の主体を資料へ出した: {excluded}"
            );
        }
        assert_eq!(serde_json::to_string(&store).unwrap(), before);
    }

    #[test]
    fn 通常と意味検索の資料は同じ主体整形を使い不明主体で保存を始めない() {
        let store = MemoryStore {
            items: vec![MemoryItem {
                category: "好み".into(),
                text: "私は緑茶が好き".into(),
                updated_at: "2026-10-11T00:00:00Z".into(),
                hits: 1,
            }],
        };
        let normal = context_from_store(&store);
        let semantic = super::format_memory_context(store.items.iter(), 0, true);
        assert_eq!(
            normal.lines().skip(1).collect::<Vec<_>>(),
            semantic.lines().skip(1).collect::<Vec<_>>()
        );
        let cfg = crate::config::Config::new();
        for text in [
            "あなたの名前はむぎ",
            "友人が「私は緑茶が好き」と言った",
            "私は緑茶が好きで、太郎はコーヒーが好き",
        ] {
            assert!(matches!(
                super::update_with(
                    &cfg,
                    text,
                    || panic!("未確定の主体で原本を読まない"),
                    |_| panic!("未確定の主体を保存しない")
                ),
                Ok(None)
            ));
        }
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
