// commands.rs - Tauri command（フロントの window.coda が叩く入口）と共有状態。

use std::sync::Mutex;

use serde_json::{json, Map, Value};
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::agent;
use crate::config::Config;
use crate::message::Message;
use crate::providers;
use crate::tools;

const LEGACY_DEFAULT_SYSTEM_PROMPT: &str = "あなたは小さなデスクトップペットです。日本語で、親しみやすく簡潔に話します。選択中キャラクターの人格設定を守り、ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";
pub const DEFAULT_SYSTEM_PROMPT: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";

fn effective_system_prompt(config: &Config) -> String {
    let prompt = config.get("CODA_SYSTEM_PROMPT", DEFAULT_SYSTEM_PROMPT);
    if prompt == LEGACY_DEFAULT_SYSTEM_PROMPT {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        prompt
    }
}

pub struct AppState {
    pub config: Config,
    pub provider_id: Mutex<String>,
    pub model: Mutex<String>,
    pub messages: Mutex<Vec<Message>>,
    pub character_profile: Mutex<String>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            config: Config::new(),
            provider_id: Mutex::new("local".to_string()),
            model: Mutex::new(String::new()),
            messages: Mutex::new(Vec::new()),
            character_profile: Mutex::new(String::new()),
        }
    }

    /// 会話履歴を初期化（system プロンプトは設定から読む）。
    pub fn reset_conversation(&self) {
        let mut prompt = effective_system_prompt(&self.config);
        let character_profile = self.character_profile.lock().unwrap().clone();
        if !character_profile.is_empty() {
            prompt.push_str("\n\n選択中キャラクターの人格設定:\n");
            prompt.push_str(&character_profile);
        }
        *self.messages.lock().unwrap() = vec![Message::new("system", prompt)];
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// 選択中プロバイダの is_up。
async fn current_up(state: &AppState) -> bool {
    let id = state.provider_id.lock().unwrap().clone();
    match providers::get(&id) {
        Some(p) => p.is_up(&state.config).await,
        None => false,
    }
}

#[tauri::command]
pub async fn pet_init(state: State<'_, AppState>) -> Result<Value, String> {
    let model = state.model.lock().unwrap().clone();
    Ok(json!({
        "model": model,
        "up": current_up(&state).await,
        "fontPx": state.config.get("CODA_BUBBLE_FONT_PX", "13"),
        "bubbleWidth": state.config.get("CODA_BUBBLE_WIDTH_PX", ""),
        "bubbleHeight": state.config.get("CODA_BUBBLE_HEIGHT_PX", ""),
        "characterId": state.config.get("CODA_CHARACTER", "mugi"),
        "characterOverrides": state.config.get_all().get("CODA_CHARACTER_OVERRIDES").cloned().unwrap_or_else(|| json!({})),
    }))
}

#[tauri::command]
pub fn pet_set_character_profile(
    character_id: String,
    profile: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if character_id.is_empty()
        || character_id.len() > 64
        || !character_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("キャラクターIDが不正です。".to_string());
    }
    if profile.chars().count() > 4000 {
        return Err("キャラクター人格設定は4000文字以内にしてください。".to_string());
    }
    *state.character_profile.lock().unwrap() = profile;
    state.reset_conversation();
    Ok(json!({ "ok": true }))
}

/// ドラッグで確定したフキダシサイズを設定の正本へ保存する。
#[tauri::command]
pub fn pet_set_bubble_size(
    width: u32,
    height: u32,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !(180..=528).contains(&width) || !(92..=378).contains(&height) {
        return Err("フキダシのサイズが許可範囲外です。".to_string());
    }
    let mut patch = Map::new();
    patch.insert("CODA_BUBBLE_WIDTH_PX".to_string(), json!(width.to_string()));
    patch.insert(
        "CODA_BUBBLE_HEIGHT_PX".to_string(),
        json!(height.to_string()),
    );
    state
        .config
        .set_all(patch)
        .map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn pet_send(text: String, state: State<'_, AppState>) -> Result<Value, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(json!({ "reply": "" }));
    }

    let id = state.provider_id.lock().unwrap().clone();
    let model = state.model.lock().unwrap().clone();
    let provider = match providers::get(&id) {
        Some(p) => p,
        None => {
            return Ok(
                json!({ "error": true, "reply": format!("プロバイダ \"{}\" が見つかりません。", id) }),
            )
        }
    };

    // std Mutex を await 跨ぎで保持しないよう、会話をクローンして処理し書き戻す。
    let mut convo = state.messages.lock().unwrap().clone();
    convo.push(Message::new("user", text));

    let result = agent::run(&state.config, provider.as_ref(), &model, &mut convo, 5).await;
    *state.messages.lock().unwrap() = convo;

    match result {
        Ok(reply) => {
            Ok(json!({ "reply": if reply.is_empty() { "……".to_string() } else { reply } }))
        }
        Err(e) => Ok(json!({ "error": true, "reply": format!("AIと話せなかったよ。{}", e) })),
    }
}

#[tauri::command]
pub async fn settings_load(state: State<'_, AppState>) -> Result<Value, String> {
    let provider_id = state.provider_id.lock().unwrap().clone();
    let model = state.model.lock().unwrap().clone();
    let mut values = state.config.get_all();
    if values.get("CODA_SYSTEM_PROMPT").and_then(Value::as_str)
        == Some(LEGACY_DEFAULT_SYSTEM_PROMPT)
    {
        values.insert(
            "CODA_SYSTEM_PROMPT".to_string(),
            Value::String(DEFAULT_SYSTEM_PROMPT.to_string()),
        );
    }

    let provider_list: Vec<Value> = providers::all()
        .iter()
        .map(|p| {
            json!({
                "id": p.id(),
                "label": p.label(),
                "defaultModel": p.default_model(&state.config),
                "configSchema": p.config_schema(),
            })
        })
        .collect();

    let tool_list: Vec<Value> = tools::all()
        .iter()
        .map(|t| {
            json!({
                "name": t.name(),
                "label": t.label(),
                "description": t.description(),
                "configSchema": t.config_schema(),
            })
        })
        .collect();

    Ok(json!({
        "values": values,
        "active": { "providerId": provider_id, "model": model },
        "defaults": { "systemPrompt": DEFAULT_SYSTEM_PROMPT },
        "providers": provider_list,
        "tools": tool_list,
    }))
}

#[tauri::command]
pub async fn settings_list_models(
    id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let pid = id.unwrap_or_else(|| state.provider_id.lock().unwrap().clone());
    match providers::get(&pid) {
        Some(p) => Ok(p.list_models(&state.config).await.unwrap_or_default()),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn settings_save(
    patch: Map<String, Value>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    let has_system_prompt = patch.contains_key("CODA_SYSTEM_PROMPT");
    state.config.set_all(patch).map_err(|e| e.to_string())?;

    // グローバル設定を実行中へ反映。
    let pid = state.config.get("CODA_PROVIDER", "local");
    *state.provider_id.lock().unwrap() = pid.clone();
    let provider =
        providers::get(&pid).ok_or_else(|| format!("プロバイダ \"{}\" が見つかりません。", pid))?;
    let model = {
        let m = state.config.get("CODA_MODEL", "");
        if m.is_empty() {
            provider.default_model(&state.config)
        } else {
            m
        }
    };
    *state.model.lock().unwrap() = model.clone();
    if has_system_prompt {
        state.reset_conversation();
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(state.config.get_bool("CODA_ALWAYS_ON_TOP", true));
        let _ = win.emit("pet:model", format!("{} / {}", provider.label(), model));
        // フキダシの見た目（文字サイズ）を実行中へ即反映。
        let _ = win.emit(
            "pet:ui",
            json!({
                "fontPx": state.config.get("CODA_BUBBLE_FONT_PX", "13"),
                "bubbleWidth": state.config.get("CODA_BUBBLE_WIDTH_PX", ""),
                "bubbleHeight": state.config.get("CODA_BUBBLE_HEIGHT_PX", ""),
                "characterId": state.config.get("CODA_CHARACTER", "mugi"),
                "characterOverrides": state.config.get_all().get("CODA_CHARACTER_OVERRIDES").cloned().unwrap_or_else(|| json!({})),
            }),
        );
    }

    Ok(json!({ "ok": true, "up": current_up(&state).await }))
}

/// 同梱エンジンの現在状態（download/starting/ready/error）を返す。
#[tauri::command]
pub async fn engine_status(app: tauri::AppHandle) -> Result<Value, String> {
    use tauri::Manager;
    match app.try_state::<crate::engine::EngineState>() {
        Some(e) => Ok(e.status.lock().unwrap().clone()),
        None => Ok(json!({ "phase": "idle" })),
    }
}

/// 設定ウィンドウを開く（既にあれば前面化）。
pub fn open_settings(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("PicoAgent の設定")
        .inner_size(560.0, 640.0)
        .min_inner_size(420.0, 480.0)
        .build();
}
