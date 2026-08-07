// history.rs - 会話履歴の永続化。
//
// 直近の会話（system 以外）をキャラクター別に app_data_dir/history-<id>.json へ
// JSON 配列で保存し、起動時・キャラクター切替時に末尾の一定数を文脈へ復元する。
// これにより再起動やキャラクター切替をまたいで「そのキャラとの直近の会話を踏まえて
// 話す」ことができ、別キャラの一人称・自己紹介が混ざらない。設定ではなく対話データ
// なので config.json とは分ける。

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::message::Message;

// ファイルに残す最大メッセージ数（system 除く）。
const MAX_PERSIST: usize = 60;
// 起動時に文脈へ戻す最大メッセージ数（小型モデルのコンテキスト超過を避ける）。
const MAX_CONTEXT_LOAD: usize = 20;

// 2026-08-07以前は短い挨拶の末尾へこの内部ガイドを追記して保存していた。
// 小型モデルがガイド自体を発話したため、読み込み時に原文へ戻す。
const LEGACY_GREETING_GUIDANCE: &str = "\n\nこれは短い挨拶です。表記の正誤や語法の説明に広げず、選択中キャラクター本人として短く挨拶を返してください。自分をアプリ、AI、デスクトップペットとして説明しないでください。";

fn data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
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

pub fn path(app: &AppHandle, character_id: &str) -> PathBuf {
    data_dir(app).join(format!("history-{}.json", sanitize(character_id)))
}

/// 旧仕様の単一ファイル history.json を、指定キャラクターのファイルへ一度だけ移行する。
/// 呼び出し側は既定キャラ（mugi）を渡す。旧履歴はキャラ別管理が無かった時代のもので
/// 当時の唯一/既定キャラのものだから。後から追加したキャラへ割り当てると人格が混ざる。
pub fn migrate_legacy(app: &AppHandle, character_id: &str) {
    let legacy = data_dir(app).join("history.json");
    let target = path(app, character_id);
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
        let Some(original) = message.content.strip_suffix(LEGACY_GREETING_GUIDANCE) else {
            continue;
        };
        message.content = original.to_string();
        cleaned += 1;
    }
    cleaned
}

fn clean_history_file(target: &std::path::Path) -> anyhow::Result<usize> {
    let lock_name = format!("history:{}", target.display());
    let _guard = crate::store::lock_state(&lock_name);
    let text = std::fs::read_to_string(target)?;
    let mut messages: Vec<Message> = serde_json::from_str(&text)?;
    let cleaned = clean_legacy_greeting_guidance(&mut messages);
    if cleaned > 0 {
        crate::store::write_json_atomic(target, &messages)?;
    }
    Ok(cleaned)
}

/// 全キャラクターの保存済み履歴から、旧版がユーザー発話へ混ぜた内部ガイドを除去する。
/// 選択中キャラだけでなく、後から切り替えるキャラの文脈も起動時にまとめて直す。
pub fn migrate_saved_greeting_guidance(app: &AppHandle) {
    let dir = data_dir(app);
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
pub fn load_recent(app: &AppHandle, character_id: &str) -> Vec<Message> {
    let target = path(app, character_id);
    let Ok(text) = std::fs::read_to_string(&target) else {
        return Vec::new();
    };
    // フォールバック許可: 履歴ファイルが壊れていても会話継続を優先し、空履歴で起動する。
    // ただし次の保存で上書きして会話を失わないよう、破損ファイルは退避してから続ける。
    let all: Vec<Message> = match serde_json::from_str(&text) {
        Ok(all) => all,
        Err(error) => {
            let saved = crate::store::quarantine_corrupt(&target);
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "会話履歴を読めないため空で起動します（{error}）{}",
                    saved
                        .map(|path| format!("。破損ファイルは {} へ退避しました", path.display()))
                        .unwrap_or_default()
                ),
            );
            Vec::new()
        }
    };
    let start = all.len().saturating_sub(MAX_CONTEXT_LOAD);
    all[start..].to_vec()
}

/// 指定キャラクターの会話（system 以外）を末尾 MAX_PERSIST 件だけ保存する。
pub fn save(app: &AppHandle, character_id: &str, messages: &[Message]) {
    let convo: Vec<&Message> = messages.iter().filter(|m| m.role != "system").collect();
    let start = convo.len().saturating_sub(MAX_PERSIST);
    let recent = &convo[start..];
    let target = path(app, character_id);
    let result = (|| -> anyhow::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::store::write_json_atomic(&target, &recent)?;
        Ok(())
    })();
    // フォールバック許可: 履歴保存の失敗は会話継続を優先し、ログのみに留める。
    if let Err(error) = result {
        crate::log_paths::append_app_log(app, &format!("会話履歴の保存に失敗しました: {error}"));
    }
}

/// 指定キャラクターの履歴ファイルを削除する（会話の完全リセット用）。
pub fn clear(app: &AppHandle, character_id: &str) {
    let _ = std::fs::remove_file(path(app, character_id));
}

#[cfg(test)]
mod tests {
    use super::{clean_legacy_greeting_guidance, LEGACY_GREETING_GUIDANCE};
    use crate::message::Message;

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
}
