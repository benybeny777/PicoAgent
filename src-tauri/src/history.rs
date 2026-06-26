// history.rs - 会話履歴の永続化。
//
// 直近の会話（system 以外）を app_data_dir/history.json に JSON 配列で保存し、
// 起動時に末尾の一定数を文脈へ復元する。これにより再起動をまたいで「直近の会話を
// 踏まえて話す」ことができる。設定ではなく対話データなので config.json とは分ける。

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::message::Message;

// ファイルに残す最大メッセージ数（system 除く）。
const MAX_PERSIST: usize = 60;
// 起動時に文脈へ戻す最大メッセージ数（小型モデルのコンテキスト超過を避ける）。
const MAX_CONTEXT_LOAD: usize = 20;

pub fn path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("history.json")
}

/// 直近の会話（system 以外）を末尾 MAX_CONTEXT_LOAD 件まで読み込む。
pub fn load_recent(app: &AppHandle) -> Vec<Message> {
    let Ok(text) = std::fs::read_to_string(path(app)) else {
        return Vec::new();
    };
    // フォールバック許可: 履歴ファイルが壊れていても会話継続を優先し、空履歴で起動する。
    let all: Vec<Message> = serde_json::from_str(&text).unwrap_or_default();
    let start = all.len().saturating_sub(MAX_CONTEXT_LOAD);
    all[start..].to_vec()
}

/// 会話（system 以外）を末尾 MAX_PERSIST 件だけ保存する。
pub fn save(app: &AppHandle, messages: &[Message]) {
    let convo: Vec<&Message> = messages.iter().filter(|m| m.role != "system").collect();
    let start = convo.len().saturating_sub(MAX_PERSIST);
    let recent = &convo[start..];
    let target = path(app);
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

/// 履歴ファイルを削除する（会話の完全リセット用）。
pub fn clear(app: &AppHandle) {
    let _ = std::fs::remove_file(path(app));
}
