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
        // 削除と会話文脈の確定を同じ境界で行い、古いターンの再保存を割り込ませない。
        state.edit_memory_context(|| {
            crate::memory::invalidate_cache(&app)?;
            let store = crate::memory::delete_item(&app, &category, &text, &updated_at)?;
            let next_context = if state.config.get_bool("PICOAGENT_MEMORY_ENABLED", true) {
                crate::memory::context_from_store(&store)
            } else {
                String::new()
            };
            Ok((store, next_context))
        })
    })
    .await
    .map_err(|error| command_error(&app, "長期メモリを削除できません", error))?;

    Ok(json!({ "items": store.items }))
}
