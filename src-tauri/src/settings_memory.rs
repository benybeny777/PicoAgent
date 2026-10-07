// settings_memory.rs - 設定画面から全キャラクター共通の長期メモリを管理する。

use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::commands::AppState;

fn command_error(app: &AppHandle, context: &str, error: impl std::fmt::Display) -> String {
    let message = format!("{context}: {error}");
    crate::log_paths::append_app_log(app, &message);
    message
}

#[tauri::command]
pub fn settings_memory_list(app: AppHandle) -> Result<Value, String> {
    let store = crate::memory::load(&app)
        .map_err(|error| command_error(&app, "長期メモリ一覧を読み込めません", error))?;
    Ok(json!({ "items": store.items, "search": crate::memory_embedding::status() }))
}

#[tauri::command]
pub async fn settings_memory_delete(
    category: String,
    text: String,
    updated_at: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    let store = crate::memory_embedding::reset_for_edit(|| {
        crate::memory::invalidate_cache(&app)?;
        crate::memory::delete_item(&app, &category, &text, &updated_at)
    })
    .await
    .map_err(|error| command_error(&app, "長期メモリを削除できません", error))?;

    let next_context = if state.config.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
        crate::memory::context_from_store(&store)
    } else {
        String::new()
    };
    *state.memory_context.lock().unwrap() = next_context;
    // 削除前のsystem文脈で進行中だった応答に、削除済みの記憶を再び書き戻させない。
    state.bump_conversation_epoch();
    state.reset_conversation();

    Ok(json!({ "items": store.items }))
}
