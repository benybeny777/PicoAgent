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

/// 旧仕様の単一ファイル history.json を、現在のキャラクター別ファイルへ一度だけ移行する。
/// 旧履歴は最後に使っていたキャラクター（＝現在の選択キャラ）のものとして引き継ぐ。
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

/// 指定キャラクターの直近の会話（system 以外）を末尾 MAX_CONTEXT_LOAD 件まで読み込む。
pub fn load_recent(app: &AppHandle, character_id: &str) -> Vec<Message> {
    let Ok(text) = std::fs::read_to_string(path(app, character_id)) else {
        return Vec::new();
    };
    // フォールバック許可: 履歴ファイルが壊れていても会話継続を優先し、空履歴で起動する。
    let all: Vec<Message> = serde_json::from_str(&text).unwrap_or_default();
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
        std::fs::write(&target, serde_json::to_string_pretty(recent)?)?;
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
