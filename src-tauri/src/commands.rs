// commands.rs - Tauri command（フロントの window.picoagent が叩く入口）と共有状態。

use std::sync::Mutex;

use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::agent;
use crate::aivis;
use crate::config::Config;
use crate::message::Message;
use crate::providers;
use crate::tools;

const LEGACY_DEFAULT_SYSTEM_PROMPT: &str = "あなたは小さなデスクトップペットです。日本語で、親しみやすく簡潔に話します。選択中キャラクターの人格設定を守り、ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";
const PREVIOUS_DEFAULT_SYSTEM_PROMPT: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";
pub const DEFAULT_SYSTEM_PROMPT: &str = "選択中キャラクター本人として、日本語で親しみやすく簡潔に話してください。挨拶や自己紹介ではアプリやデスクトップペットの説明をせず、キャラクターの名前・性格・できることをそのキャラの口調で短く伝えてください。「こんにちは」「こんにちわ」などの短い挨拶には表記の正誤を説明せず、自然に挨拶を返してください。ユーザーの作業を邪魔せず、必要なときだけ役に立つ相棒として応答してください。";

fn effective_system_prompt(config: &Config) -> String {
    let prompt = config.get("PICOAGENT_SYSTEM_PROMPT", DEFAULT_SYSTEM_PROMPT);
    if prompt == LEGACY_DEFAULT_SYSTEM_PROMPT || prompt == PREVIOUS_DEFAULT_SYSTEM_PROMPT {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        prompt
    }
}

// 会話の文脈に保持する最大メッセージ数（system 除く）。小型モデルのコンテキスト超過を避ける。
const MAX_CONTEXT_MESSAGES: usize = 24;

pub struct AppState {
    pub config: Config,
    pub provider_id: Mutex<String>,
    pub model: Mutex<String>,
    pub messages: Mutex<Vec<Message>>,
    pub character_profile: Mutex<String>,
    /// 直近の自発雑談（独り言）。繰り返しを避けるため次回生成へ「避ける文」として渡す。
    pub recent_chatter: Mutex<Vec<String>>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            config: Config::new(),
            provider_id: Mutex::new("local".to_string()),
            model: Mutex::new(String::new()),
            messages: Mutex::new(Vec::new()),
            character_profile: Mutex::new(String::new()),
            recent_chatter: Mutex::new(Vec::new()),
        }
    }

    /// system プロンプトを設定から作り直す。会話の流れ（system 以外）は保持する
    /// （人格やプロンプト変更で直近の会話文脈を失わないため）。
    pub fn reset_conversation(&self) {
        let mut prompt = effective_system_prompt(&self.config);
        let character_profile = self.character_profile.lock().unwrap().clone();
        if !character_profile.is_empty() {
            prompt.push_str("\n\n選択中キャラクターの人格設定:\n");
            prompt.push_str(&character_profile);
        }
        let mut messages = self.messages.lock().unwrap();
        let kept: Vec<Message> = messages
            .iter()
            .filter(|m| m.role != "system")
            .cloned()
            .collect();
        let mut next = Vec::with_capacity(kept.len() + 1);
        next.push(Message::new("system", prompt));
        next.extend(kept);
        *messages = next;
    }

    /// 会話を完全に初期化する（system のみに戻す）。「新しい会話」用。
    pub fn clear_conversation(&self) {
        let prompt = {
            let mut prompt = effective_system_prompt(&self.config);
            let character_profile = self.character_profile.lock().unwrap().clone();
            if !character_profile.is_empty() {
                prompt.push_str("\n\n選択中キャラクターの人格設定:\n");
                prompt.push_str(&character_profile);
            }
            prompt
        };
        *self.messages.lock().unwrap() = vec![Message::new("system", prompt)];
    }
}

// 完了したターンから、文脈・履歴に残す「発話」だけを取り出す（user と、本文のある
// assistant のみ）。途中の tool 呼び出し/結果は残すとメッセージ順序が壊れるため落とす。
fn clean_turns(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| {
            m.role == "user"
                || (m.role == "assistant" && m.tool_calls.is_none() && !m.content.is_empty())
        })
        .cloned()
        .collect()
}

// 会話を「system + クリーンな直近発話（最大 MAX_CONTEXT_MESSAGES 件）」へ圧縮する。
fn compacted_conversation(convo: &[Message]) -> Vec<Message> {
    let mut turns = clean_turns(convo);
    if turns.len() > MAX_CONTEXT_MESSAGES {
        turns.drain(0..turns.len() - MAX_CONTEXT_MESSAGES);
    }
    let mut next: Vec<Message> = convo
        .iter()
        .filter(|m| m.role == "system")
        .cloned()
        .collect();
    next.extend(turns);
    next
}

/// 会話を完全に初期化し、履歴ファイルも消す（「会話をリセット」用）。UI へも通知する。
pub fn clear_conversation_and_history(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.clear_conversation();
    state.recent_chatter.lock().unwrap().clear();
    crate::history::clear(app);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.emit("pet:reset", ());
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
        "fontPx": state.config.get("PICOAGENT_BUBBLE_FONT_PX", "13"),
        "bubbleWidth": state.config.get("PICOAGENT_BUBBLE_WIDTH_PX", ""),
        "bubbleHeight": state.config.get("PICOAGENT_BUBBLE_HEIGHT_PX", ""),
        "characterId": state.config.get("PICOAGENT_CHARACTER", "mugi"),
        "characterOverrides": state.config.get_all().get("PICOAGENT_CHARACTER_OVERRIDES").cloned().unwrap_or_else(|| json!({})),
        "chatterEnabled": state.config.get_bool("PICOAGENT_CHATTER_ENABLED", false),
        "chatterMinSecs": state.config.get("PICOAGENT_CHATTER_MIN_SECS", "90"),
        "chatterMaxSecs": state.config.get("PICOAGENT_CHATTER_MAX_SECS", "240"),
    }))
}

// 同梱 Aivis の状態を返す。TTS 無効時は enabled=false（待たずにキャラ描画してよい合図）。
#[tauri::command]
pub async fn aivis_status(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    let enabled = state.config.get_bool("PICOAGENT_TTS_ENABLED", false);
    let up = if enabled {
        aivis::is_ready(&app, &state).await
    } else {
        false
    };
    Ok(json!({ "enabled": enabled, "up": up }))
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
    patch.insert(
        "PICOAGENT_BUBBLE_WIDTH_PX".to_string(),
        json!(width.to_string()),
    );
    patch.insert(
        "PICOAGENT_BUBBLE_HEIGHT_PX".to_string(),
        json!(height.to_string()),
    );
    state
        .config
        .set_all(patch)
        .map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn pet_send(
    text: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
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
    // 会話文脈を圧縮（system + クリーンな直近発話）して保持し、ファイルへ永続化する。
    let compacted = compacted_conversation(&convo);
    *state.messages.lock().unwrap() = compacted.clone();
    crate::history::save(&app, &compacted);

    match result {
        Ok(reply) => {
            Ok(json!({ "reply": if reply.is_empty() { "……".to_string() } else { reply } }))
        }
        Err(e) => Ok(json!({ "error": true, "reply": format!("AIと話せなかったよ。{}", e) })),
    }
}

// アイドル時の自発雑談を生成する。雑談スキル(chitchat)が指示文を提供し、ここで
// 選択中キャラの人格(system)込みでプロバイダへ依頼する。Web検索やツールは使わない。
// 会話履歴には積まない（独り言で本来の文脈を膨らませない）。
#[tauri::command]
pub async fn pet_idle_chatter(state: State<'_, AppState>) -> Result<Value, String> {
    if !state.config.get_bool("PICOAGENT_CHATTER_ENABLED", false) {
        return Ok(json!({ "enabled": false }));
    }
    let id = state.provider_id.lock().unwrap().clone();
    let model = state.model.lock().unwrap().clone();
    let provider = match providers::get(&id) {
        Some(p) => p,
        None => return Ok(json!({ "enabled": true, "reply": "" })),
    };

    // 直近の独り言を「避ける文」として渡し、毎回ちがう内容にする。
    let avoid = state.recent_chatter.lock().unwrap().clone();
    // 雑談スキルから独り言用の指示文を受け取り、現在の人格付き会話のクローンへ一時的に積む。
    // 会話文脈（直近の会話）も含むため、最近の話題を踏まえた独り言になる。
    let instruction = tools::dispatch(&state.config, "chitchat", json!({ "avoid": avoid })).await;
    let mut convo = state.messages.lock().unwrap().clone();
    convo.push(Message::new("user", instruction));

    match provider.chat(&state.config, &model, &convo, &[]).await {
        Ok(message) => {
            let reply = message.content.trim().to_string();
            // 直近の独り言として記録（末尾 6 件まで保持）。次回の重複回避に使う。
            if !reply.is_empty() {
                let mut recent = state.recent_chatter.lock().unwrap();
                recent.push(reply.clone());
                let len = recent.len();
                if len > 6 {
                    recent.drain(0..len - 6);
                }
            }
            Ok(json!({ "enabled": true, "reply": reply }))
        }
        Err(error) => Err(format!("自発雑談の生成に失敗しました: {error}")),
    }
}

fn speech_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            let mut label = String::new();
            while let Some(c) = chars.next() {
                if c == ']' {
                    if chars.peek() == Some(&'(') {
                        let _ = chars.next();
                        while let Some(u) = chars.next() {
                            if u == ')' {
                                break;
                            }
                        }
                        out.push_str(&label);
                    } else {
                        out.push('[');
                        out.push_str(&label);
                        out.push(']');
                    }
                    break;
                }
                label.push(c);
            }
            continue;
        }
        out.push(ch);
    }
    out.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("http://") && !line.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn effective_tts_speaker(state: &AppState) -> String {
    let character_id = state.config.get("PICOAGENT_CHARACTER", "mugi");
    if let Some(speaker) = state
        .config
        .get_all()
        .get("PICOAGENT_TTS_CHARACTER_OVERRIDES")
        .and_then(|value| value.get(&character_id))
        .and_then(|value| value.get("speaker"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return speaker.to_string();
    }
    state.config.get("PICOAGENT_TTS_SPEAKER", "888753760")
}

#[tauri::command]
pub async fn pet_speak(
    text: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !state.config.get_bool("PICOAGENT_TTS_ENABLED", false) {
        return Ok(json!({ "enabled": false }));
    }
    let text = speech_text(&text);
    if text.trim().is_empty() {
        return Ok(json!({ "enabled": true, "skipped": true }));
    }
    aivis::ensure_started(&app)
        .await
        .map_err(|error| error.to_string())?;
    let endpoint = aivis::endpoint(&app, &state);
    let speaker = effective_tts_speaker(&state);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let query_url = format!("{endpoint}/audio_query");
    let query = client
        .post(&query_url)
        .query(&[("text", text.as_str()), ("speaker", speaker.as_str())])
        .send()
        .await
        .map_err(|error| format!("Aivis の音声クエリ作成に失敗しました: {error}"))?;
    if !query.status().is_success() {
        return Err(format!(
            "Aivis の音声クエリ作成に失敗しました: HTTP {}",
            query.status()
        ));
    }
    let audio_query: Value = query
        .json()
        .await
        .map_err(|error| format!("Aivis の音声クエリを読めませんでした: {error}"))?;
    let synthesis_url = format!("{endpoint}/synthesis");
    let wav = client
        .post(&synthesis_url)
        .query(&[("speaker", speaker.as_str())])
        .json(&audio_query)
        .send()
        .await
        .map_err(|error| format!("Aivis の音声合成に失敗しました: {error}"))?;
    if !wav.status().is_success() {
        return Err(format!(
            "Aivis の音声合成に失敗しました: HTTP {}",
            wav.status()
        ));
    }
    let bytes = wav
        .bytes()
        .await
        .map_err(|error| format!("Aivis の音声データを読めませんでした: {error}"))?;
    Ok(json!({ "enabled": true, "mime": "audio/wav", "audio": bytes.to_vec() }))
}

#[tauri::command]
pub async fn settings_load(state: State<'_, AppState>) -> Result<Value, String> {
    let provider_id = state.provider_id.lock().unwrap().clone();
    let model = state.model.lock().unwrap().clone();
    let mut values = state.config.get_all();
    if values
        .get("PICOAGENT_SYSTEM_PROMPT")
        .and_then(Value::as_str)
        == Some(LEGACY_DEFAULT_SYSTEM_PROMPT)
        || values
            .get("PICOAGENT_SYSTEM_PROMPT")
            .and_then(Value::as_str)
            == Some(PREVIOUS_DEFAULT_SYSTEM_PROMPT)
    {
        values.insert(
            "PICOAGENT_SYSTEM_PROMPT".to_string(),
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
    let has_system_prompt = patch.contains_key("PICOAGENT_SYSTEM_PROMPT");
    let has_tts_enabled = patch.contains_key("PICOAGENT_TTS_ENABLED");
    let has_tts_device = patch.contains_key("PICOAGENT_TTS_DEVICE");
    state.config.set_all(patch).map_err(|e| e.to_string())?;

    // グローバル設定を実行中へ反映。
    let pid = state.config.get("PICOAGENT_PROVIDER", "local");
    *state.provider_id.lock().unwrap() = pid.clone();
    let provider =
        providers::get(&pid).ok_or_else(|| format!("プロバイダ \"{}\" が見つかりません。", pid))?;
    let model = {
        let m = state.config.get("PICOAGENT_MODEL", "");
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
    if has_tts_enabled || has_tts_device {
        aivis::stop(&app);
        if state.config.get_bool("PICOAGENT_TTS_ENABLED", false) {
            let handle = app.clone();
            if let Err(error) = aivis::start_background(&handle) {
                eprintln!("同梱 Aivis の起動に失敗しました: {error}");
            }
            tauri::async_runtime::spawn(async move {
                if let Err(error) = aivis::ensure_started(&handle).await {
                    eprintln!("同梱 Aivis の起動に失敗しました: {error}");
                }
            });
        }
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(state.config.get_bool("PICOAGENT_ALWAYS_ON_TOP", true));
        let _ = win.emit("pet:model", format!("{} / {}", provider.label(), model));
        // フキダシの見た目（文字サイズ）を実行中へ即反映。
        let _ = win.emit(
            "pet:ui",
            json!({
                "fontPx": state.config.get("PICOAGENT_BUBBLE_FONT_PX", "13"),
                "bubbleWidth": state.config.get("PICOAGENT_BUBBLE_WIDTH_PX", ""),
                "bubbleHeight": state.config.get("PICOAGENT_BUBBLE_HEIGHT_PX", ""),
                "characterId": state.config.get("PICOAGENT_CHARACTER", "mugi"),
                "characterOverrides": state.config.get_all().get("PICOAGENT_CHARACTER_OVERRIDES").cloned().unwrap_or_else(|| json!({})),
                "chatterEnabled": state.config.get_bool("PICOAGENT_CHATTER_ENABLED", false),
                "chatterMinSecs": state.config.get("PICOAGENT_CHATTER_MIN_SECS", "90"),
                "chatterMaxSecs": state.config.get("PICOAGENT_CHATTER_MAX_SECS", "240"),
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
