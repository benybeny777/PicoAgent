// history.rs - 会話履歴の永続化。
//
// 直近の会話（system 以外）をキャラクター別に app_data_dir/history-<id>.json へ
// JSON 配列で保存し、起動時・キャラクター切替時に末尾の一定数を文脈へ復元する。
// これにより再起動やキャラクター切替をまたいで「そのキャラとの直近の会話を踏まえて
// 話す」ことができ、別キャラの一人称・自己紹介が混ざらない。設定ではなく対話データ
// なので config.json とは分ける。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;

use crate::message::Message;

// ファイルに残す最大メッセージ数（system 除く）。
const MAX_PERSIST: usize = 60;
// 起動時に文脈へ戻す最大メッセージ数（小型モデルのコンテキスト超過を避ける）。
const MAX_CONTEXT_LOAD: usize = 20;

// 2026-08-07以前は短い挨拶の末尾へこの内部ガイドを追記して保存していた。
// 小型モデルがガイド自体を発話したため、読み込み時に原文へ戻す。
/// 旧版（#215 以前）が雑談・相談の発話へ付けていた内部ガイド。利用者の吹き出しにそのまま出ていた。
const LEGACY_CHAT_GUIDANCE: &str = "\n\nこれは雑談や相談です。Web検索は不要です。選択中キャラクター本人として、自然に短めに答えてください。最新情報・事実確認が必要なときだけ、その旨を伝えてください。";
const LEGACY_GREETING_GUIDANCE: &str = "\n\nこれは短い挨拶です。表記の正誤や語法の説明に広げず、選択中キャラクター本人として短く挨拶を返してください。自分をアプリ、AI、デスクトップペットとして説明しないでください。";

fn data_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(crate::app_paths::data_dir(app))
}

/// キャラクターIDをファイル名に使える形へ正規化する（想定外の文字は落とす）。
fn sanitize(character_id: &str) -> String {
    let s: String = character_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if s.is_empty() {
        "default".to_string()
    } else {
        s
    }
}

pub fn path(app: &AppHandle, character_id: &str) -> anyhow::Result<PathBuf> {
    Ok(crate::app_paths::data_path(
        app,
        crate::app_paths::DataPath::History(&sanitize(character_id)),
    ))
}

/// 読み込みに失敗した履歴ファイル（＝中身が分からない原本）を持つキャラクターID。
/// 履歴は「全部読んで全部書き戻す」更新なので、読めないまま空で続けて保存すると、
/// 読めなかっただけの原本が短い新規履歴で丸ごと消える。読めたことを確認できるまで
/// 保存を止め、原本をそのまま残す。
fn unreadable() -> &'static Mutex<HashSet<String>> {
    static UNREADABLE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    UNREADABLE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 履歴を読めなかったキャラクターの保存を止める。
fn block_saving(character_id: &str) {
    unreadable().lock().unwrap().insert(sanitize(character_id));
}

/// 原本の中身が分かった（読めた・退避した・利用者が消した）ので保存を再開する。
///
/// 保存停止はキャラごとだが、画面の警告は1つ。止めているキャラが残っている間は解除を
/// 知らせない（別キャラの保存成功で「解除」と出すと、止まったままのキャラを見誤る）。
/// 最後の1体が解除されたときだけ `true` を返す。
fn allow_saving(character_id: &str) -> bool {
    release(&mut unreadable().lock().unwrap(), character_id)
}

fn release(blocked: &mut HashSet<String>, character_id: &str) -> bool {
    blocked.remove(&sanitize(character_id)) && blocked.is_empty()
}

fn resume_saving(app: &AppHandle, character_id: &str) {
    if allow_saving(character_id) {
        BLOCKED.report(
            app,
            "history-save-blocked",
            "会話履歴の保存",
            Ok(()),
            "会話履歴の保存停止が解除されました",
        );
    }
}

/// 保存を止めているか。止めている間の会話はどこにも残らないため、
/// `pet_init` がこの状態を画面へ返して利用者へ知らせる（ログだけでは気づけない）。
pub fn saving_blocked(character_id: &str) -> bool {
    unreadable()
        .lock()
        .unwrap()
        .contains(&sanitize(character_id))
}

/// 旧仕様の単一ファイル history.json を、指定キャラクターのファイルへ一度だけ移行する。
/// 呼び出し側は既定キャラ（mugi）を渡す。旧履歴はキャラ別管理が無かった時代のもので
/// 当時の唯一/既定キャラのものだから。後から追加したキャラへ割り当てると人格が混ざる。
pub fn migrate_legacy(app: &AppHandle, character_id: &str) {
    let (legacy, target) = match (data_dir(app), path(app, character_id)) {
        (Ok(dir), Ok(target)) => (dir.join("history.json"), target),
        (Err(error), _) | (_, Err(error)) => {
            crate::log_paths::append_app_log(app, &error.to_string());
            return;
        }
    };
    if legacy.exists() && !target.exists() {
        if let Err(error) = std::fs::rename(&legacy, &target) {
            crate::log_paths::append_app_log(
                app,
                &format!("旧会話履歴のキャラクター別移行に失敗しました: {error}"),
            );
        }
    }
}

fn clean_legacy_greeting_guidance(messages: &mut [Message]) -> usize {
    let mut cleaned = 0;
    for message in messages {
        if message.role != "user" {
            continue;
        }
        let Some(original) = [LEGACY_GREETING_GUIDANCE, LEGACY_CHAT_GUIDANCE]
            .iter()
            .find_map(|guidance| message.content.strip_suffix(guidance))
        else {
            continue;
        };
        message.content = original.to_string();
        cleaned += 1;
    }
    cleaned
}

fn remove_known_prompt_leaks(messages: &mut Vec<Message>) -> usize {
    let before = messages.len();
    messages.retain(|message| {
        if message.role != "assistant" {
            return true;
        }
        let normalized = message.content.replace(char::is_whitespace, "");
        // 履歴の自動削除は不可逆なので、公開時の広い検査条件を流用しない。
        // 利用者が実際に確認した「入力の分類 + 応答方針 + 人格維持」の組合せだけを除く。
        let known_leak = normalized.contains("ユーザーからの挨拶")
            && normalized.contains("態度で応答")
            && (normalized.contains("キャラクター設定") || normalized.contains("口調"))
            && (normalized.contains("維持") || normalized.contains("一貫して"));
        !known_leak
    });
    before - messages.len()
}

/// 履歴ファイルごとのロック名。移行・保存・削除が同じ名前を取ることで、
/// 「読む → 直す → 書き戻す」の途中へ別経路の書き込みが割り込んでも取りこぼさない
/// （名前がずれているとロックを取っていないのと同じになる）。
fn lock_name(target: &std::path::Path) -> String {
    format!("history:{}", target.display())
}

fn clean_history_file(target: &std::path::Path) -> anyhow::Result<usize> {
    let _guard = crate::store::lock_state(&lock_name(target));
    let text = std::fs::read_to_string(target)?;
    let mut messages: Vec<Message> = serde_json::from_str(&text)?;
    let cleaned =
        clean_legacy_greeting_guidance(&mut messages) + remove_known_prompt_leaks(&mut messages);
    if cleaned > 0 {
        crate::store::write_json_atomic(target, &messages)?;
    }
    Ok(cleaned)
}

/// 全キャラクターの保存済み履歴から、旧版がユーザー発話へ混ぜた内部ガイドを除去する。
/// 選択中キャラだけでなく、後から切り替えるキャラの文脈も起動時にまとめて直す。
pub fn migrate_saved_greeting_guidance(app: &AppHandle) {
    let dir = match data_dir(app) {
        Ok(dir) => dir,
        Err(error) => {
            crate::log_paths::append_app_log(app, &error.to_string());
            return;
        }
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) => {
            crate::log_paths::append_app_log(
                app,
                &format!("会話履歴の内部ガイド移行を開始できませんでした: {error}"),
            );
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                crate::log_paths::append_app_log(
                    app,
                    &format!("会話履歴の列挙中に項目を読めませんでした: {error}"),
                );
                continue;
            }
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("history-") || !name.ends_with(".json") {
            continue;
        }
        if let Err(error) = clean_history_file(&entry.path()) {
            // 破損履歴の退避は、実際に選択して load_recent() したときの既存経路へ任せる。
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "{} の内部ガイド移行に失敗しました: {error}",
                    entry.path().display()
                ),
            );
        }
    }
}

/// 指定キャラクターの直近の会話（system 以外）を末尾 MAX_CONTEXT_LOAD 件まで読み込む。
pub fn load_recent(app: &AppHandle, character_id: &str) -> anyhow::Result<Vec<Message>> {
    let target = path(app, character_id)?;
    let text = match std::fs::read_to_string(&target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            resume_saving(app, character_id);
            return Ok(Vec::new());
        }
        Err(error) => {
            // 読めた保証が無い原本を、このあとの保存で上書きさせない。
            // 起動は続けられるが、保存を許すと「読めなかっただけ」の会話が消える。
            block_saving(character_id);
            return Err(anyhow::anyhow!(
                "会話履歴を読み込めません: {} ({error})",
                target.display()
            ));
        }
    };
    // フォールバック許可: PR #463 採用案3。退避と通知に成功した領域だけ新規状態で続ける。
    // ただし次の保存で上書きして会話を失わないよう、破損ファイルは退避してから続ける。
    let all: Vec<Message> = match serde_json::from_str(&text) {
        Ok(all) => all,
        Err(error) => {
            let saved = crate::store::quarantine_and_notify(
                app,
                &target,
                crate::store::QuarantineArea::History,
            )
            .ok_or_else(|| {
                block_saving(character_id);
                anyhow::anyhow!("壊れた会話履歴を退避できません。上書きを中止しました: {error}")
            })?;
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "会話履歴を読めないため空で起動します（{error}）。破損ファイルは {} へ退避しました",
                    saved.display()
                ),
            );
            Vec::new()
        }
    };
    // ここまで来たら原本の中身は分かっている（読めた／退避済み）ので保存を再開してよい。
    resume_saving(app, character_id);
    let start = all.len().saturating_sub(MAX_CONTEXT_LOAD);
    Ok(all[start..]
        .iter()
        .cloned()
        .map(strip_legacy_label)
        .collect())
}

/// 旧版が自発雑談へ付けていた内部向けの見出し。履歴として画面にそのまま出ていた。
const LEGACY_CHATTER_LABEL: &str = "直前の自発雑談: ";

/// 保存済みの旧データから、画面に出すべきでない内部向けの見出しを外す。
fn strip_legacy_label(mut message: Message) -> Message {
    if message.role == "assistant" {
        if let Some(rest) = message.content.strip_prefix(LEGACY_CHATTER_LABEL) {
            message.content = rest.to_string();
        }
    }
    message
}

/// 指定キャラクターの会話（system 以外）を末尾 MAX_PERSIST 件だけ保存する。
pub fn save(app: &AppHandle, character_id: &str, messages: &[Message]) {
    if saving_blocked(character_id) {
        crate::log_paths::append_app_log(
            app,
            &format!(
                "会話履歴を読めなかったため保存を止めています（原本を保護中）: {character_id}"
            ),
        );
        // 起動後のキャラクター切替で読めなくなった場合、起動時の案内には間に合わない。
        // 同じ印で知らせるので、起動時に出していれば二重には出ない。
        notify_blocked(app);
        return;
    }
    let convo: Vec<&Message> = messages.iter().filter(|m| m.role != "system").collect();
    let start = convo.len().saturating_sub(MAX_PERSIST);
    let recent = &convo[start..];
    let target = match path(app, character_id) {
        Ok(target) => target,
        Err(error) => {
            crate::log_paths::append_app_log(app, &error.to_string());
            return;
        }
    };
    let _guard = crate::store::lock_state(&lock_name(&target));
    let result = (|| -> anyhow::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::store::write_json_atomic(&target, &recent)?;
        Ok(())
    })();
    // フォールバック許可: PR #463 採用案。保存失敗を通知して会話を継続する。
    SAVE_FAILED.report(
        app,
        "history-save",
        "会話履歴の保存",
        result.map_err(|error| {
            format!("会話履歴を保存できません。再起動で今の会話が残りません: {error}")
        }),
        "会話履歴の保存が復旧しました",
    );
}

static SAVE_FAILED: crate::notice::FailureLatch = crate::notice::FailureLatch::new();
static BLOCKED: crate::notice::FailureLatch = crate::notice::FailureLatch::new();

fn notify_blocked(app: &AppHandle) {
    BLOCKED.report(
        app,
        "history-save-blocked",
        "会話履歴の保存",
        Err(
            "会話履歴を読めなかったため、この会話は保存されません（元の履歴は残しています）".into(),
        ),
        "会話履歴の保存停止が解除されました",
    );
}

/// 指定キャラクターの履歴ファイルを削除する（会話の完全リセット用）。
pub fn clear(app: &AppHandle, character_id: &str) -> anyhow::Result<()> {
    let target = path(app, character_id)?;
    let _guard = crate::store::lock_state(&lock_name(&target));
    match std::fs::remove_file(target) {
        // 利用者自身が消したので、読めなかった原本を守る必要も無くなる。
        Ok(()) => {
            resume_saving(app, character_id);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            resume_saving(app, character_id);
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allow_saving, block_saving, clean_legacy_greeting_guidance, release,
        remove_known_prompt_leaks, saving_blocked, strip_legacy_label, LEGACY_CHAT_GUIDANCE,
        LEGACY_GREETING_GUIDANCE,
    };
    use crate::message::Message;

    #[test]
    fn 旧版の雑談ガイドは利用者の発言から外す() {
        let mut messages = vec![
            Message::new("user", format!("かわいいね{LEGACY_CHAT_GUIDANCE}")),
            Message::new("assistant", format!("ありがと{LEGACY_CHAT_GUIDANCE}")),
        ];
        assert_eq!(clean_legacy_greeting_guidance(&mut messages), 1);
        assert_eq!(messages[0].content, "かわいいね");
        assert!(
            messages[1].content.ends_with(LEGACY_CHAT_GUIDANCE),
            "キャラの発話は書き換えない"
        );
    }

    #[test]
    fn 旧版の自発雑談の内部見出しは読み込み時に外す() {
        let cleaned =
            strip_legacy_label(Message::new("assistant", "直前の自発雑談: 今日もがんばろ"));
        assert_eq!(cleaned.content, "今日もがんばろ");
        let user = strip_legacy_label(Message::new("user", "直前の自発雑談: 引用"));
        assert_eq!(
            user.content, "直前の自発雑談: 引用",
            "利用者の発言は書き換えない"
        );
    }

    #[test]
    fn 読めなかった履歴はキャラ単位で保存を止めて復旧で再開する() {
        block_saving("test-unreadable");
        assert!(saving_blocked("test-unreadable"));
        // ファイル名と同じ正規化を通すので、区切り文字違いでも同じ原本として扱う。
        assert!(saving_blocked("test-unreadable!"));
        assert!(!saving_blocked("test-readable"));
        allow_saving("test-unreadable");
        assert!(!saving_blocked("test-unreadable"));
    }

    #[test]
    fn 別キャラの保存停止が残る間は解除を知らせない() {
        // 共有の保存停止表は並列テストと共有するので、判定関数を手元の表で確かめる。
        let mut blocked = std::collections::HashSet::from(["a".to_string(), "b".to_string()]);
        assert!(
            !release(&mut blocked, "a"),
            "Bが止まったままなら解除と知らせない"
        );
        assert!(
            !release(&mut blocked, "a"),
            "止めていないキャラの解除では知らせない"
        );
        assert!(
            release(&mut blocked, "b"),
            "最後の1体の解除で復旧を知らせる"
        );
    }

    #[test]
    fn 履歴保存は保存禁止の確認を通す() {
        // 履歴は「全部読んで全部書き戻す」更新なので、読めないまま空で保存すると
        // 原本が丸ごと消える。確認が外れていないことをソースで固定する。
        let source = crate::guard_scan::all_sources()
            .into_iter()
            .find(|source| source.name == "history.rs")
            .expect("history.rs を走査できません");
        let save = source
            .body()
            .split_once("pub fn save(")
            .expect("save() がありません")
            .1;
        let body = save.split_once("\n}").expect("関数の終端がありません").0;
        assert!(
            body.contains("saving_blocked("),
            "読めなかった履歴の上書きを止めるため、save() の先頭で saving_blocked() を確認してください"
        );
    }

    #[test]
    fn 全キャラ共通の旧挨拶ガイドをユーザー原文へ戻す() {
        let mut messages = vec![
            Message::new("user", format!("hi{LEGACY_GREETING_GUIDANCE}")),
            Message::new("assistant", "こんにちは！"),
            Message::new("user", "通常の質問"),
        ];

        assert_eq!(clean_legacy_greeting_guidance(&mut messages), 1);
        assert_eq!(messages[0].content, "hi");
        assert_eq!(messages[1].content, "こんにちは！");
        assert_eq!(messages[2].content, "通常の質問");
        assert_eq!(clean_legacy_greeting_guidance(&mut messages), 0);
    }

    #[test]
    fn 確実な内部設定説明だけを旧履歴から除く() {
        let mut messages = vec![
            Message::new("user", "こんちわぁ"),
            Message::new("assistant", "こんにちは。ユーザーからの挨拶に対して、ツンデレな態度で応答する。一貫してキャラクター設定を維持する。"),
            Message::new("assistant", "こんにちは！今日はなにする？"),
        ];
        assert_eq!(remove_known_prompt_leaks(&mut messages), 1);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "こんにちは！今日はなにする？");

        let mut legitimate = vec![Message::new(
            "assistant",
            "システムプロンプトや内部指示の扱いについて説明します。",
        )];
        assert_eq!(remove_known_prompt_leaks(&mut legitimate), 0);
        assert_eq!(legitimate.len(), 1);
    }
}
