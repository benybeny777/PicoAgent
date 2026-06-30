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
    let configured = config.get("PICOAGENT_SYSTEM_PROMPT", DEFAULT_SYSTEM_PROMPT);
    let mut prompt = if configured == LEGACY_DEFAULT_SYSTEM_PROMPT
        || configured == PREVIOUS_DEFAULT_SYSTEM_PROMPT
    {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        configured
    };
    // 使えるスキル一覧を添え、「何ができる？」に具体的に答えられるようにする。
    let capabilities = crate::tools::capability_summary();
    if !capabilities.is_empty() {
        prompt.push_str("\n\n使えるスキル（必要なときに自動で実行できます）: ");
        prompt.push_str(&capabilities);
        prompt.push_str("\n「何ができる？」「何が得意？」などと聞かれたら、上のスキルから役立つものをキャラクターの口調で具体的に挙げて教えてください。");
    }
    prompt
}

// 日本語文字は約2トークン、ASCII は約0.25トークン（÷4）で粗く見積もる。
fn estimate_tokens(msg: &Message) -> usize {
    let japanese = msg.content.chars().filter(|c| *c as u32 > 0x7F).count();
    let ascii = msg.content.chars().count() - japanese;
    japanese * 2 + (ascii + 3) / 4
}

// 非 system メッセージに割り当てるトークン予算の上限。8k コンテキストの約 75% を目安にする。
const MAX_CONTEXT_TOKENS: usize = 6_000;

// 会話要約が発動するトークン閾値（予算の 70%: 予算に近づいた段階で余裕を作る）。
const SUMMARIZE_THRESHOLD: usize = MAX_CONTEXT_TOKENS * 7 / 10; // ~4,200

pub struct AppState {
    pub config: Config,
    pub provider_id: Mutex<String>,
    pub model: Mutex<String>,
    pub messages: Mutex<Vec<Message>>,
    pub character_profile: Mutex<String>,
    /// 会話文脈が属する選択中キャラクターID。これが変わったら会話を破棄して
    /// 旧キャラの一人称・自己紹介が新キャラの応答へ混ざらないようにする。
    pub character_id: Mutex<String>,
    /// 会話から暗黙的に抽出した長期メモリ。system 文脈へ差し込む。
    pub memory_context: Mutex<String>,
    /// 直近の自発雑談（独り言）。繰り返しを避けるため次回生成へ「避ける文」として渡す。
    pub recent_chatter: Mutex<Vec<String>>,
    /// チャットで確認待ちの未許可コマンド。ユーザーが「許可」または「キャンセル」と返すまで保持する。
    pub pending_command: Mutex<Option<crate::tools::run_command::PendingCommandApproval>>,
    /// キャラクターの現在感情状態（ルールベース推定）。
    pub emotion: Mutex<crate::emotion::EmotionState>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            config: Config::new(),
            provider_id: Mutex::new("local".to_string()),
            model: Mutex::new(String::new()),
            messages: Mutex::new(Vec::new()),
            character_profile: Mutex::new(String::new()),
            character_id: Mutex::new(String::new()),
            memory_context: Mutex::new(String::new()),
            recent_chatter: Mutex::new(Vec::new()),
            pending_command: Mutex::new(None),
            emotion: Mutex::new(crate::emotion::EmotionState::default()),
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
        let memory_context = self.memory_context.lock().unwrap().clone();
        if !memory_context.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&memory_context);
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
            let memory_context = self.memory_context.lock().unwrap().clone();
            if !memory_context.is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(&memory_context);
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

// 会話を「system + クリーンな直近発話（トークン予算内）」へ圧縮する。
// メッセージ数ではなくトークン推定量で制限し、小型モデルのコンテキスト超過を防ぐ。
fn compacted_conversation(convo: &[Message]) -> Vec<Message> {
    let mut turns = clean_turns(convo);
    let mut total: usize = turns.iter().map(estimate_tokens).sum();
    // 予算超過している間、最古の発話から削除する（最低2件は保持）。
    while total > MAX_CONTEXT_TOKENS && turns.len() > 2 {
        total -= estimate_tokens(&turns[0]);
        turns.remove(0);
    }
    let mut next: Vec<Message> = convo
        .iter()
        .filter(|m| m.role == "system")
        .cloned()
        .collect();
    next.extend(turns);
    next
}

/// 会話が SUMMARIZE_THRESHOLD を超えた場合、古いターンをLLMで要約してsystemへ付加する。
/// 要約 API 呼び出しが失敗した場合は元の会話をそのまま返す（会話を止めない）。
async fn maybe_summarize_context(
    convo: Vec<Message>,
    provider: &dyn providers::Provider,
    cfg: &Config,
    model: &str,
) -> Vec<Message> {
    let clean = clean_turns(&convo);
    let total: usize = clean.iter().map(estimate_tokens).sum();
    // 閾値以下またはターン数が少なすぎる場合はスキップ（最低 6 ターン必要）。
    if total <= SUMMARIZE_THRESHOLD || clean.len() < 6 {
        return convo;
    }

    // 古い 40% を要約対象にする（最低 2 ターン、残す側も最低 2 ターン確保）。
    let split = ((clean.len() * 2) / 5).max(2).min(clean.len() - 2);
    let to_summarize = &clean[..split];
    let to_keep = &clean[split..];

    let history_text: String = to_summarize
        .iter()
        .map(|m| {
            let role = if m.role == "user" {
                "ユーザー"
            } else {
                "アシスタント"
            };
            format!("{}: {}", role, m.content)
        })
        .collect::<Vec<_>>()
        .join("\n---\n");

    let summary_msgs = vec![
        Message::new(
            "system",
            "以下の会話履歴を2〜3文で簡潔に日本語で要約してください。重要な事実・指示・約束だけを残し、挨拶や相槌は省いてください。要約文のみを返してください。",
        ),
        Message::new("user", &history_text),
    ];

    let Ok(summary_msg) = provider.chat(cfg, model, &summary_msgs, &[]).await else {
        return convo;
    };
    let summary_text = summary_msg.content.trim().to_string();
    if summary_text.is_empty() {
        return convo;
    }

    // system メッセージへ要約を追記し、古いターンを差し替える。
    let mut result: Vec<Message> = convo
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| {
            let mut m = m.clone();
            m.content
                .push_str(&format!("\n\n[以前の会話の要約（自動）: {}]", summary_text));
            m
        })
        .collect();
    result.extend(to_keep.iter().cloned());
    result
}

/// 履歴ファイルの分割キーになる、現在アクティブな選択中キャラクターID。
/// state 側が未設定（起動直後など）なら設定の選択キャラへフォールバックする。
pub fn active_character_id(state: &AppState) -> String {
    let id = state.character_id.lock().unwrap().clone();
    if id.is_empty() {
        state.config.get("PICOAGENT_CHARACTER", "mugi")
    } else {
        id
    }
}

/// ユーザー発話とアシスタント返答を会話へ記録し、圧縮・履歴保存して返却 JSON を作る。
/// 承認/キャンセルなど「AI を介さず即返答する」分岐の共通処理。
fn record_turn(
    state: &AppState,
    app: &AppHandle,
    user_text: &str,
    reply: String,
) -> Result<Value, String> {
    let mut convo = state.messages.lock().unwrap().clone();
    convo.push(Message::new("user", user_text));
    convo.push(Message::new("assistant", reply.clone()));
    let compacted = compacted_conversation(&convo);
    *state.messages.lock().unwrap() = compacted.clone();
    crate::history::save(app, &active_character_id(state), &compacted);
    Ok(json!({ "reply": reply }))
}

/// 会話を完全に初期化し、履歴ファイルも消す（「会話をリセット」用）。UI へも通知する。
pub fn clear_conversation_and_history(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.clear_conversation();
    state.recent_chatter.lock().unwrap().clear();
    state.memory_context.lock().unwrap().clear();
    crate::history::clear(app, &active_character_id(&state));
    crate::memory::clear(app);
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

fn log_command_error(app: &AppHandle, context: &str, error: impl std::fmt::Display) -> String {
    let message = format!("{context}: {error}");
    crate::log_paths::append_app_log(app, &message);
    message
}

fn is_approval_text(text: &str) -> bool {
    matches!(
        text.trim().to_lowercase().as_str(),
        "許可" | "実行して" | "実行していい" | "ok" | "yes" | "y"
    )
}

fn is_persistent_approval_text(text: &str) -> bool {
    matches!(
        text.trim().to_lowercase().as_str(),
        "永続許可" | "常に許可" | "今後も許可" | "永久許可" | "always" | "allow always"
    )
}

fn is_cancel_text(text: &str) -> bool {
    matches!(
        text.trim().to_lowercase().as_str(),
        "キャンセル" | "やめて" | "中止" | "cancel" | "no" | "n"
    )
}

#[tauri::command]
pub async fn pet_init(state: State<'_, AppState>) -> Result<Value, String> {
    let model = state.model.lock().unwrap().clone();
    let recent_history: Vec<Message> = clean_turns(&state.messages.lock().unwrap())
        .into_iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(json!({
        "model": model,
        "providerId": state.config.get("PICOAGENT_PROVIDER", "local"),
        "up": current_up(&state).await,
        "fontPx": state.config.get("PICOAGENT_BUBBLE_FONT_PX", "13"),
        "bubbleWidth": state.config.get("PICOAGENT_BUBBLE_WIDTH_PX", ""),
        "bubbleHeight": state.config.get("PICOAGENT_BUBBLE_HEIGHT_PX", ""),
        "bubbleLeft": state.config.get("PICOAGENT_BUBBLE_LEFT_PX", ""),
        "bubbleTop": state.config.get("PICOAGENT_BUBBLE_TOP_PX", ""),
        "voiceVolume": state.config.get("PICOAGENT_TTS_VOLUME", "100"),
        "characterId": state.config.get("PICOAGENT_CHARACTER", "mugi"),
        "characterOverrides": state.config.get_all().get("PICOAGENT_CHARACTER_OVERRIDES").cloned().unwrap_or_else(|| json!({})),
        "history": recent_history,
        "chatterEnabled": state.config.get_bool("PICOAGENT_CHATTER_ENABLED", false),
        "chatterMinSecs": state.config.get("PICOAGENT_CHATTER_MIN_SECS", "90"),
        "chatterMaxSecs": state.config.get("PICOAGENT_CHATTER_MAX_SECS", "240"),
    }))
}

#[tauri::command]
pub fn pet_daily_greeting(state: State<'_, AppState>) -> Result<Value, String> {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    let part = match now.hour() {
        5..=10 => "おはよう",
        11..=16 => "こんにちは",
        17..=18 => "夕方だね",
        _ => "こんばんは",
    };
    let events = crate::daily::load_events(&state.config)
        .into_iter()
        .filter(|event| event.enabled)
        .count();
    let praise = crate::daily::load_praise(&state.config);
    let mut line = format!(
        "{part}。今日は{}月{}日、今は{:02}:{:02}だよ。",
        now.month(),
        now.day(),
        now.hour(),
        now.minute()
    );
    if events > 0 {
        line.push_str(&format!(" 予定してる通知は{events}件あるよ。"));
    }
    if let Some(last) = praise.last() {
        line.push_str(&format!(" この前の「{}」もちゃんと覚えてる。", last.text));
    }
    Ok(json!({ "reply": line }))
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
pub async fn pet_prepare_voice_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !state.config.get_bool("PICOAGENT_TTS_ENABLED", false) {
        return Ok(json!({ "enabled": false }));
    }
    aivis::ensure_started(&app)
        .await
        .map_err(|error| log_command_error(&app, "Aivis の起動確認に失敗しました", error))?;
    let endpoint = aivis::endpoint(&app, &state);
    let voice = effective_tts_voice(&state);
    prepare_tts_voice_model(&app, &endpoint, &voice).await?;
    let speaker = resolve_tts_speaker(&endpoint, voice).await;
    Ok(json!({ "enabled": true, "ready": true, "speaker": speaker }))
}

#[tauri::command]
pub fn pet_set_character_profile(
    character_id: String,
    profile: String,
    state: State<'_, AppState>,
    app: AppHandle,
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
    let previous = {
        let mut current = state.character_id.lock().unwrap();
        let previous = current.clone();
        *current = character_id.clone();
        previous
    };
    if previous.is_empty() || previous == character_id {
        // 起動直後の初回設定、または同一キャラの人格更新。直近の文脈は保持する。
        state.reset_conversation();
    } else {
        // キャラクター変更: 旧キャラの会話文脈を捨て、切替先キャラの履歴を復元する。
        // 旧キャラの履歴はファイルに残るため、戻れば続きから話せる。
        state.clear_conversation();
        let recent = crate::history::load_recent(&app, &character_id);
        if !recent.is_empty() {
            state.messages.lock().unwrap().extend(recent);
        }
    }
    Ok(json!({ "ok": true }))
}

/// ドラッグで確定したフキダシサイズを設定の正本へ保存する。
#[tauri::command]
pub fn pet_set_bubble_size(
    width: u32,
    height: u32,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !(180..=2000).contains(&width) || !(92..=1400).contains(&height) {
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

/// ドラッグで確定したフキダシのサイズと座標を設定の正本へ保存する。
#[tauri::command]
pub fn pet_set_bubble_layout(
    width: u32,
    height: u32,
    left: u32,
    top: u32,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !(180..=2000).contains(&width) || !(92..=1400).contains(&height) || left > 2000 || top > 1400
    {
        return Err("フキダシの表示位置またはサイズが許可範囲外です。".to_string());
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
    patch.insert(
        "PICOAGENT_BUBBLE_LEFT_PX".to_string(),
        json!(left.to_string()),
    );
    patch.insert(
        "PICOAGENT_BUBBLE_TOP_PX".to_string(),
        json!(top.to_string()),
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
    let user_text = text.clone();

    let pending = state.pending_command.lock().unwrap().clone();
    if let Some(pending) = pending {
        if is_persistent_approval_text(&text) {
            *state.pending_command.lock().unwrap() = None;
            let reply = crate::tools::run_command::execute_with_persistent_approval(
                &state.config,
                &pending,
            )
            .await;
            return record_turn(&state, &app, &user_text, reply);
        }
        if is_approval_text(&text) {
            *state.pending_command.lock().unwrap() = None;
            let reply = crate::tools::run_command::execute_pending(&pending).await;
            return record_turn(&state, &app, &user_text, reply);
        }
        if is_cancel_text(&text) {
            *state.pending_command.lock().unwrap() = None;
            return record_turn(
                &state,
                &app,
                &user_text,
                "コマンド実行をキャンセルしました。".to_string(),
            );
        }
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
    // セマンティックメモリ: クエリに関連するメモリを今回のリクエスト向けに絞り込み、
    // 今回の convo の system メッセージのみ更新する（state.memory_context は変えない）。
    {
        let semantic_ctx = crate::memory::context_for_query(&app, &state.config, &text);
        let base_ctx = state.memory_context.lock().unwrap().clone();
        if !semantic_ctx.is_empty() && semantic_ctx != base_ctx {
            if let Some(sys) = convo.iter_mut().find(|m| m.role == "system") {
                if base_ctx.is_empty() {
                    sys.content.push_str("\n\n");
                    sys.content.push_str(&semantic_ctx);
                } else {
                    sys.content = sys
                        .content
                        .replacen(base_ctx.as_str(), semantic_ctx.as_str(), 1);
                }
            }
        }
    }
    convo.push(Message::new("user", text));

    // 会話が長くなった場合、古いターンをLLMで要約してコンテキストを節約する。
    let mut convo = maybe_summarize_context(convo, provider, &state.config, &model).await;

    let result = agent::run(&state.config, provider, &model, &mut convo, 5, Some(&app)).await;
    // 会話文脈を圧縮（system + クリーンな直近発話）して保持し、ファイルへ永続化する。
    let compacted = compacted_conversation(&convo);
    *state.messages.lock().unwrap() = compacted.clone();
    let character_id = active_character_id(&state);
    crate::history::save(&app, &character_id, &compacted);
    if result.is_ok() {
        if let Some(memory_context) =
            crate::memory::update_from_turn(&app, &state.config, &user_text)
        {
            *state.memory_context.lock().unwrap() = memory_context;
            state.reset_conversation();
            let refreshed = state.messages.lock().unwrap().clone();
            crate::history::save(&app, &character_id, &refreshed);
        }
    }

    match result {
        Ok(reply) => {
            if let Some(pending) = crate::tools::run_command::parse_approval_marker(&reply) {
                *state.pending_command.lock().unwrap() = Some(pending.clone());
                let prompt = crate::tools::run_command::approval_prompt(&pending);
                let mut refreshed = state.messages.lock().unwrap().clone();
                if let Some(last) = refreshed
                    .iter_mut()
                    .rev()
                    .find(|message| message.role == "assistant")
                {
                    last.content = prompt.clone();
                }
                let compacted = compacted_conversation(&refreshed);
                *state.messages.lock().unwrap() = compacted.clone();
                crate::history::save(&app, &active_character_id(&state), &compacted);
                return Ok(json!({ "reply": prompt }));
            }
            // 感情状態を更新してフロントへ通知する。
            let new_emotion = {
                let current = state.emotion.lock().unwrap().clone();
                crate::emotion::update(&current, &user_text, &reply)
            };
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.emit(
                    "pet:emotion",
                    serde_json::to_value(&new_emotion).unwrap_or_default(),
                );
            }
            *state.emotion.lock().unwrap() = new_emotion;
            Ok(json!({ "reply": if reply.is_empty() { "……".to_string() } else { reply } }))
        }
        Err(e) => {
            let message = log_command_error(&app, "AI応答に失敗しました", e);
            Ok(json!({ "error": true, "reply": format!("AIと話せなかったよ。{}", message) }))
        }
    }
}

// アイドル時の自発雑談を生成する。雑談スキル(chitchat)が指示文を提供し、ここで
// 選択中キャラの人格(system)込みでプロバイダへ依頼する。Web検索やツールは使わない。
// 生成後は「直前の自発雑談」として会話文脈にも短く残す。
// ユーザーが「今の雑談は何の話？」と聞いたとき、モデルが直前の独り言を参照できるようにするため。
// 現在のローカル日時と時間帯を、雑談が時間に合った内容を言えるよう短く渡す。
fn current_time_context() -> String {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    let weekday =
        ["月", "火", "水", "木", "金", "土", "日"][now.weekday().num_days_from_monday() as usize];
    let hour = now.hour();
    let part = match hour {
        5..=10 => "朝",
        11..=16 => "昼",
        17..=18 => "夕方",
        _ => "夜",
    };
    // 夜・早朝に「日差し」「昼の景色」など時間に合わない描写をしないよう明示する。
    let caution = if (5..=18).contains(&hour) {
        ""
    } else {
        "。いまは夜なので、日差しや昼の景色など昼前提の描写はしない"
    };
    format!(
        "現在の日時: {}年{}月{}日({weekday}) {hour:02}:{:02}、時間帯は「{part}」です。独り言は必ずこの時間帯に合う内容にする{caution}。",
        now.year(),
        now.month(),
        now.day(),
        now.minute(),
    )
}

// 返信と記事タイトルの「内容のある文字バイグラム」の重なりから、最も関連する出典URLを選ぶ。
// ASCII・記号は除外し、地震→地震ニュースのように話題に触れたときだけURLを添えるための簡易一致。
fn best_source_url(
    reply: &str,
    sources: &[crate::tools::chitchat::trends::TopicSource],
) -> Option<String> {
    fn content_bigrams(s: &str) -> std::collections::HashSet<(char, char)> {
        let chars: Vec<char> = s
            .chars()
            .filter(|c| !c.is_ascii() && c.is_alphanumeric())
            .collect();
        chars.windows(2).map(|w| (w[0], w[1])).collect()
    }
    let reply_bg = content_bigrams(reply);
    if reply_bg.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &str)> = None;
    for source in sources {
        if source.url.is_empty() {
            continue;
        }
        let score = content_bigrams(&source.title)
            .intersection(&reply_bg)
            .count();
        if score >= 1 && best.map(|(b, _)| score > b).unwrap_or(true) {
            best = Some((score, &source.url));
        }
    }
    best.map(|(_, url)| url.to_string())
}

#[tauri::command]
pub async fn pet_idle_chatter(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
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
    // 話題メモ（ニュース等）を1件取得。雑談がこの話題に触れたら出典URLを末尾に添える。
    let brief = crate::tools::chitchat::trends::topic_brief(&state.config).await;
    let topic_memo = brief.as_ref().map(|b| b.memo.clone()).unwrap_or_default();
    let instruction = tools::dispatch(
        &state.config,
        "chitchat",
        json!({ "avoid": avoid, "topic": topic_memo }),
    )
    .await;
    // 雑談は「人格(system)＋現在時刻＋指示」だけで生成する。直近の会話（ユーザー発話）は
    // 文脈へ入れない（小型モデルが入力をオウム返しするのを防ぐため）。
    let system = state
        .messages
        .lock()
        .unwrap()
        .iter()
        .find(|m| m.role == "system")
        .cloned();
    let mut convo: Vec<Message> = Vec::new();
    if let Some(system) = system {
        convo.push(system);
    }
    convo.push(Message::new(
        "user",
        format!("{}\n\n{instruction}", current_time_context()),
    ));

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
                let compacted = {
                    let mut messages = state.messages.lock().unwrap();
                    messages.push(Message::new(
                        "assistant",
                        format!("直前の自発雑談: {reply}"),
                    ));
                    compacted_conversation(&messages)
                };
                *state.messages.lock().unwrap() = compacted.clone();
                crate::history::save(&app, &active_character_id(&state), &compacted);
            }
            // 雑談が話題メモ（ニュース等）に触れていれば、一致する出典URLを末尾に添える。
            // URL 行は読み上げ（speech_text）で除外されるため音声には乗らない。
            let display = match brief
                .as_ref()
                .and_then(|b| best_source_url(&reply, &b.sources))
            {
                Some(url) if !reply.is_empty() => format!("{reply}\n\n参考:\n{url}"),
                _ => reply,
            };
            Ok(json!({ "enabled": true, "reply": display }))
        }
        Err(error) => Err(log_command_error(
            &app,
            "自発雑談の生成に失敗しました",
            error,
        )),
    }
}

#[tauri::command]
pub fn pet_daily_events(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    match crate::daily::due_events(&state.config) {
        Ok(events) => Ok(json!({ "events": events })),
        Err(error) => Err(log_command_error(&app, "通知の確認に失敗しました", error)),
    }
}

// 登録中の目覚まし一覧を返す（フロントの時刻ポーリング用）。
#[tauri::command]
pub async fn pet_alarms(state: State<'_, AppState>) -> Result<Value, String> {
    let alarms = crate::alarm::load(&state.config);
    Ok(json!({ "alarms": serde_json::to_value(&alarms).unwrap_or_else(|_| json!([])) }))
}

// 既定の起こし文（生成に失敗・エンジン未起動でも必ず声がけできるよう用意）。
fn default_wake_message(label: &str) -> String {
    if label.is_empty() {
        "おはよう…そろそろ起きる時間だよ。ゆっくりでいいから、少しずつ目を覚まそうね。".to_string()
    } else {
        format!("おはよう…「{label}」の時間だよ。あせらなくていいからね、ゆっくり起きよう。")
    }
}

// 選択中キャラの口調で、やさしい起こし文を生成する（失敗時は None）。
async fn generate_wake_message(state: &AppState, label: &str) -> Option<String> {
    let id = state.provider_id.lock().unwrap().clone();
    let model = state.model.lock().unwrap().clone();
    let provider = providers::get(&id)?;
    let label_hint = if label.is_empty() {
        String::new()
    } else {
        format!("これは「{label}」の目覚ましです。")
    };
    let instruction = format!(
        "いまは設定した目覚ましの時刻です。{label_hint}眠っている相手を、選択中キャラクター本人の口調で、やさしく起こす短い声がけを1つだけ言ってください。急かさず温かく、35〜60文字程度の1〜2文。質問・指示・長い説明・URLは入れない。",
    );
    let mut convo = state.messages.lock().unwrap().clone();
    convo.push(Message::new("user", instruction));
    let message = provider
        .chat(&state.config, &model, &convo, &[])
        .await
        .ok()?;
    let reply = message.content.trim().to_string();
    if reply.is_empty() {
        None
    } else {
        Some(reply)
    }
}

// 目覚ましを発火する: やさしい起こし文を返し、一度きりの目覚ましは無効化して保存する。
#[tauri::command]
pub async fn pet_alarm_wake(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut alarms = crate::alarm::load(&state.config);
    let target = alarms.iter().find(|a| a.id == id).cloned();
    let label = target.as_ref().map(|a| a.label.clone()).unwrap_or_default();
    // 一度きりの目覚ましは鳴ったら無効化する（曜日繰り返しはそのまま）。
    if let Some(alarm) = target.as_ref() {
        if !alarm.repeat {
            for a in alarms.iter_mut() {
                if a.id == id {
                    a.enabled = false;
                }
            }
            if let Err(error) = crate::alarm::save(&state.config, &alarms) {
                crate::log_paths::append_app_log(
                    &app,
                    &format!("目覚ましの無効化保存に失敗しました: {error}"),
                );
            }
        }
    }
    let reply = match generate_wake_message(&state, &label).await {
        Some(text) => text,
        None => default_wake_message(&label),
    };
    Ok(json!({ "reply": reply }))
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
                        for u in chars.by_ref() {
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
    // スパークライン（ブロック文字）や絵文字は読み上げに乗せない（天気・為替チャート対策）。
    let out: String = out.chars().filter(|c| !is_chart_or_emoji(*c)).collect();
    out.lines()
        .map(str::trim)
        .take_while(|line| !line.starts_with("参考:") && !line.starts_with("参考："))
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("http://") && !line.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ")
}

// 読み上げ不要な装飾文字（罫線・ブロック＝スパークライン、各種記号・絵文字、異体字セレクタ）。
fn is_chart_or_emoji(c: char) -> bool {
    matches!(c as u32,
        0x2580..=0x259F   // 罫線・ブロック要素（▁▂▃▅▇ など）
        | 0x2600..=0x27BF // 記号・装飾・Dingbats（☀ ☔ ✓ など）
        | 0xFE00..=0xFE0F // 異体字セレクタ
        | 0x1F000..=0x1FAFF // 絵文字全般
    )
}

struct TtsVoice {
    /// 合成に使う style ID（フォールバック）。speaker_uuid があれば実 ID は実行時に解決する。
    speaker: String,
    model_uuid: Option<String>,
    /// AivisHub モデルの話者 UUID。あれば導入確認と実 style ID 解決に使う。
    speaker_uuid: Option<String>,
    /// 複数スタイルから選ぶときのスタイル名（例: ノーマル）。
    style_name: Option<String>,
}

// るな（AivisHub）の話者情報。global style ID はエンジンが導入時に算出するため、
// ハードコードはフォールバックに留め、実 ID は speaker_uuid から `/speakers` で解決する。
const RIN_AIVIS_SPEAKER: &str = "345585728";
const RIN_AIVIS_MODEL_UUID: &str = "4f281e78-eba6-495a-8e50-5c322d02b5b1";
const RIN_AIVIS_SPEAKER_UUID: &str = "aa63d66e-92fe-499e-9e0d-15dad4a61d0d";
const RIN_AIVIS_STYLE_NAME: &str = "ノーマル";

// まい（AivisHub「まい」）。実 style ID は speaker_uuid から `/speakers` で解決し、
// 下記はフォールバック。
const MAI_AIVIS_SPEAKER: &str = "1431611904";
const MAI_AIVIS_MODEL_UUID: &str = "e9339137-2ae3-4d41-9394-fb757a7e61e6";
const MAI_AIVIS_SPEAKER_UUID: &str = "41b7785f-35cc-4089-a360-dd8a63da5e75";
const MAI_AIVIS_STYLE_NAME: &str = "ノーマル";

fn default_character_tts_voice(character_id: &str) -> Option<TtsVoice> {
    match character_id {
        "rin" => Some(TtsVoice {
            speaker: RIN_AIVIS_SPEAKER.to_string(),
            model_uuid: Some(RIN_AIVIS_MODEL_UUID.to_string()),
            speaker_uuid: Some(RIN_AIVIS_SPEAKER_UUID.to_string()),
            style_name: Some(RIN_AIVIS_STYLE_NAME.to_string()),
        }),
        "mai" => Some(TtsVoice {
            speaker: MAI_AIVIS_SPEAKER.to_string(),
            model_uuid: Some(MAI_AIVIS_MODEL_UUID.to_string()),
            speaker_uuid: Some(MAI_AIVIS_SPEAKER_UUID.to_string()),
            style_name: Some(MAI_AIVIS_STYLE_NAME.to_string()),
        }),
        _ => None,
    }
}

fn effective_tts_voice(state: &AppState) -> TtsVoice {
    let character_id = state.config.get("PICOAGENT_CHARACTER", "mugi");
    let default = default_character_tts_voice(&character_id);
    let override_voice = state
        .config
        .get_all()
        .get("PICOAGENT_TTS_CHARACTER_OVERRIDES")
        .and_then(|value| value.get(&character_id))
        .cloned();
    let override_speaker = override_voice
        .as_ref()
        .and_then(|value| value.get("speaker"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let override_model_uuid = override_voice
        .as_ref()
        .and_then(|value| value.get("modelUuid"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(speaker) = override_speaker {
        // ユーザーが明示指定した style ID はそのまま使う（実行時解決はしない）。
        return TtsVoice {
            speaker: speaker.to_string(),
            model_uuid: override_model_uuid.map(str::to_string),
            speaker_uuid: None,
            style_name: None,
        };
    }
    default.unwrap_or_else(|| TtsVoice {
        speaker: state.config.get("PICOAGENT_TTS_SPEAKER", "888753760"),
        model_uuid: None,
        speaker_uuid: None,
        style_name: None,
    })
}

async fn prepare_tts_voice_model(
    app: &AppHandle,
    endpoint: &str,
    voice: &TtsVoice,
) -> Result<(), String> {
    if let (Some(model_uuid), Some(speaker_uuid)) =
        (voice.model_uuid.as_deref(), voice.speaker_uuid.as_deref())
    {
        aivis::ensure_model_installed(app, endpoint, model_uuid, speaker_uuid)
            .await
            .map_err(|error| log_command_error(app, "Aivis モデルの導入に失敗しました", error))?;
    }
    Ok(())
}

async fn resolve_tts_speaker(endpoint: &str, voice: TtsVoice) -> String {
    match voice.speaker_uuid.as_deref() {
        Some(speaker_uuid) => {
            aivis::resolve_style_id(endpoint, speaker_uuid, voice.style_name.as_deref())
                .await
                // フォールバック許可: 既存環境の手動 style ID 指定を尊重し、解決失敗時も合成 API 側のエラーを返せるようにする。
                .unwrap_or(voice.speaker)
        }
        None => voice.speaker,
    }
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
        .map_err(|error| log_command_error(&app, "Aivis の起動確認に失敗しました", error))?;
    let endpoint = aivis::endpoint(&app, &state);
    let voice = effective_tts_voice(&state);
    // AivisHub モデル指定があり未導入なら導入する。通常は起動時に事前導入済み。
    prepare_tts_voice_model(&app, &endpoint, &voice).await?;
    // speaker_uuid があれば実 style ID を `/speakers` から解決する（取り違え防止）。
    // 解決できなければ設定値の style ID を使う。
    let speaker = resolve_tts_speaker(&endpoint, voice).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|error| {
            log_command_error(&app, "Aivis HTTP クライアント作成に失敗しました", error)
        })?;
    let query_url = format!("{endpoint}/audio_query");
    let query = client
        .post(&query_url)
        .query(&[("text", text.as_str()), ("speaker", speaker.as_str())])
        .send()
        .await
        .map_err(|error| log_command_error(&app, "Aivis の音声クエリ作成に失敗しました", error))?;
    if !query.status().is_success() {
        return Err(log_command_error(
            &app,
            "Aivis の音声クエリ作成に失敗しました",
            format!("HTTP {}", query.status()),
        ));
    }
    let audio_query: Value = query
        .json()
        .await
        .map_err(|error| log_command_error(&app, "Aivis の音声クエリを読めませんでした", error))?;
    let synthesis_url = format!("{endpoint}/synthesis");
    let wav = client
        .post(&synthesis_url)
        .query(&[("speaker", speaker.as_str())])
        .json(&audio_query)
        .send()
        .await
        .map_err(|error| log_command_error(&app, "Aivis の音声合成に失敗しました", error))?;
    if !wav.status().is_success() {
        return Err(log_command_error(
            &app,
            "Aivis の音声合成に失敗しました",
            format!("HTTP {}", wav.status()),
        ));
    }
    let bytes = wav
        .bytes()
        .await
        .map_err(|error| log_command_error(&app, "Aivis の音声データを読めませんでした", error))?;
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
                "examples": t.examples(),
                "configSchema": t.config_schema(),
            })
        })
        .collect();

    Ok(json!({
        "values": values,
        "active": { "providerId": provider_id, "model": model },
        "defaults": { "systemPrompt": DEFAULT_SYSTEM_PROMPT },
        "providers": provider_list,
        "localModelPresets": crate::providers::local::LOCAL_MODEL_PRESETS.iter().map(|preset| json!({
            "id": preset.id,
            "family": preset.family,
            "label": preset.label,
            "localSize": preset.size,
            "url": preset.url,
            "ctx": preset.ctx,
            "recommendedVramGb": preset.recommended_vram_gb,
            "recommendedRamGb": preset.recommended_ram_gb,
            "downloadSize": preset.download_size,
            "note": preset.note,
        })).collect::<Vec<Value>>(),
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
        Some(p) => p
            .list_models(&state.config)
            .await
            .map_err(|error| format!("モデル一覧を取得できませんでした: {error}")),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn settings_save(
    patch: Map<String, Value>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    let changed_text = |key: &str, fallback: &str| {
        patch
            .get(key)
            .and_then(Value::as_str)
            .map(|next| state.config.get(key, fallback) != next)
            .unwrap_or(false)
    };
    let changed_bool = |key: &str, fallback: bool| {
        patch
            .get(key)
            .and_then(Value::as_bool)
            .map(|next| state.config.get_bool(key, fallback) != next)
            .unwrap_or(false)
    };
    let has_system_prompt = patch.contains_key("PICOAGENT_SYSTEM_PROMPT");
    let has_memory_setting = patch.contains_key("PICOAGENT_MEMORY_ENABLED")
        || patch.contains_key("PICOAGENT_MEMORY_MAX_ITEMS");
    let tts_runtime_changed = changed_bool("PICOAGENT_TTS_ENABLED", false)
        || changed_text("PICOAGENT_TTS_DEVICE", "auto")
        || changed_text("PICOAGENT_TTS_PORT", "11436");
    let provider_changed = changed_text("PICOAGENT_PROVIDER", "local");
    let local_engine_changed = changed_text("PICOAGENT_LOCAL_MODEL_PRESET", "auto")
        || changed_text("PICOAGENT_MODEL", "")
        || changed_text("PICOAGENT_MODEL_URL", "")
        || changed_text("PICOAGENT_LOCAL_PORT", "11435")
        || changed_text("PICOAGENT_LOCAL_CTX", "8192")
        || changed_text("PICOAGENT_LOCAL_NGL", "999")
        || changed_text("PICOAGENT_LOCAL_GPU_DEVICE", "");
    let character_changed = changed_text("PICOAGENT_CHARACTER", "mugi");
    state
        .config
        .set_all(patch)
        .map_err(|e| log_command_error(&app, "設定保存に失敗しました", e))?;

    // モデル DL・エンジン/Aivis 起動・キャラ読込など重い反映は実行中に行わず、
    // 再起動して起動シーケンス（ローディング画面）でまとめて反映する（フキダシに
    // 「準備中…」を出さないため）。何が変わったかを再起動側へ伝え、必要な部分だけ
    // 立ち上げ直す（例: キャラ変更だけなら LLM エンジンは再起動しない）。
    let restart_engine = provider_changed || local_engine_changed;
    let restart_aivis = tts_runtime_changed;
    let requires_restart = restart_engine || restart_aivis || character_changed;

    // 人格プロンプト・メモリは軽いので常に即反映する（再起動対象外）。
    if has_system_prompt || has_memory_setting {
        *state.memory_context.lock().unwrap() = crate::memory::context(&app, &state.config);
        state.reset_conversation();
    }

    let pid = state.config.get("PICOAGENT_PROVIDER", "local");
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
    // 再起動する場合は、キャンセル時に旧エンジンと不整合にならないよう実行中状態を触らない。
    if !requires_restart {
        *state.provider_id.lock().unwrap() = pid.clone();
        *state.model.lock().unwrap() = model.clone();
    }

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(state.config.get_bool("PICOAGENT_ALWAYS_ON_TOP", true));
        // 軽い見た目・雑談設定は常に即反映。キャラ切替・プロバイダ/モデルは再起動側で
        // 反映するため、再起動が必要なときは characterId / pet:model を送らない。
        let mut ui = json!({
            "fontPx": state.config.get("PICOAGENT_BUBBLE_FONT_PX", "13"),
            "bubbleWidth": state.config.get("PICOAGENT_BUBBLE_WIDTH_PX", ""),
            "bubbleHeight": state.config.get("PICOAGENT_BUBBLE_HEIGHT_PX", ""),
            "bubbleLeft": state.config.get("PICOAGENT_BUBBLE_LEFT_PX", ""),
            "bubbleTop": state.config.get("PICOAGENT_BUBBLE_TOP_PX", ""),
            "voiceVolume": state.config.get("PICOAGENT_TTS_VOLUME", "100"),
            "chatterEnabled": state.config.get_bool("PICOAGENT_CHATTER_ENABLED", false),
            "chatterMinSecs": state.config.get("PICOAGENT_CHATTER_MIN_SECS", "90"),
            "chatterMaxSecs": state.config.get("PICOAGENT_CHATTER_MAX_SECS", "240"),
        });
        if !requires_restart {
            let _ = win.emit("pet:model", format!("{} / {}", provider.label(), model));
            ui["characterId"] = json!(state.config.get("PICOAGENT_CHARACTER", "mugi"));
            ui["characterOverrides"] = state
                .config
                .get_all()
                .get("PICOAGENT_CHARACTER_OVERRIDES")
                .cloned()
                .unwrap_or_else(|| json!({}));
        }
        let _ = win.emit("pet:ui", ui);
    }

    Ok(json!({
        "ok": true,
        "requiresRestart": requires_restart,
        "restartEngine": restart_engine,
        "restartAivis": restart_aivis,
        "up": current_up(&state).await
    }))
}

/// 設定反映のための再起動相当処理。重い反映（モデル DL・エンジン/Aivis 起動・
/// キャラ読込）を起動シーケンス（ローディング画面）でまとめて行うための共通入口。
///
/// プロセスの再起動（`app.restart()`）は `tauri dev` の開発配信サーバが落ちて
/// `127.0.0.1` 接続拒否になるため使わない。代わりに「バックエンドを起動シーケンス
/// 相当に再初期化」してから、メインウィンドウへ `pet:reload` を送り webview を
/// 再読込させる。これで dev / 配布版どちらでも同じ挙動になる。
#[tauri::command]
pub fn restart_app(
    app: AppHandle,
    state: State<'_, AppState>,
    restart_engine: bool,
    restart_aivis: bool,
) -> Result<Value, String> {
    crate::log_paths::append_app_log(&app, "設定反映のため再初期化します。");

    let pid = state.config.get("PICOAGENT_PROVIDER", "local");
    *state.provider_id.lock().unwrap() = pid.clone();
    let model = {
        let m = state.config.get("PICOAGENT_MODEL", "");
        if m.is_empty() {
            providers::get(&pid)
                .map(|p| p.default_model(&state.config))
                .unwrap_or_default()
        } else {
            m
        }
    };
    *state.model.lock().unwrap() = model;
    *state.memory_context.lock().unwrap() = crate::memory::context(&app, &state.config);

    // 選択中キャラの履歴を文脈へ読み込み直す（旧キャラの会話は持ち越さない）。
    let character_id = state.config.get("PICOAGENT_CHARACTER", "mugi");
    *state.character_id.lock().unwrap() = character_id.clone();
    state.clear_conversation();
    let recent = crate::history::load_recent(&app, &character_id);
    if !recent.is_empty() {
        state.messages.lock().unwrap().extend(recent);
    }

    // 同梱エンジンは、プロバイダ/モデル関連が変わったときだけ立ち上げ直す。
    // キャラ変更だけのときは LLM を再ロードせず動かしたままにする。
    if restart_engine {
        crate::engine::stop(&app);
        if pid == "local" {
            let handle = app.clone();
            tauri::async_runtime::spawn(crate::engine::ensure_and_start(handle));
        }
    }
    // Aivis は TTS 設定（ON/OFF・デバイス・ポート）が変わったときだけ立ち上げ直す。
    if restart_aivis {
        aivis::stop(&app);
        if state.config.get_bool("PICOAGENT_TTS_ENABLED", false) {
            let handle = app.clone();
            if let Err(error) = aivis::start_background(&handle) {
                crate::log_paths::append_app_log(
                    &handle,
                    &format!("再初期化後の同梱 Aivis 起動に失敗しました: {error}"),
                );
            }
            tauri::async_runtime::spawn(async move {
                if let Err(error) = aivis::ensure_started(&handle).await {
                    crate::log_paths::append_app_log(
                        &handle,
                        &format!("再初期化後の同梱 Aivis 起動確認に失敗しました: {error}"),
                    );
                }
            });
        }
    }

    // メインウィンドウへ再読込（起動シーケンス再実行）を指示する。
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.emit("pet:reload", ());
    }
    Ok(json!({ "ok": true }))
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

#[tauri::command]
pub async fn pet_log(
    level: String,
    message: String,
    detail: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let level = match level.as_str() {
        "debug" | "info" | "warn" | "error" => level,
        _ => "info".to_string(),
    };
    let detail = detail
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| format!(" detail={value}"))
        .unwrap_or_default();
    crate::log_paths::append_app_log(&app, &format!("[frontend:{level}] {message}{detail}"));
    Ok(())
}

/// 現在の感情状態を返す。
#[tauri::command]
pub fn pet_emotion(state: State<'_, AppState>) -> Result<Value, String> {
    let emotion = state.emotion.lock().unwrap().clone();
    Ok(serde_json::to_value(&emotion).map_err(|e| e.to_string())?)
}

/// 今月のトークン使用量（月次集計）と累計を返す。
#[tauri::command]
pub fn pet_usage(app: AppHandle) -> Result<Value, String> {
    let store = crate::usage::load(&app);
    Ok(json!({
        "monthlySummary": store.monthly_summary(),
        "totalTokens": store.total_tokens(),
        "recordCount": store.records.len(),
    }))
}

/// 直近ツール実行のトレース情報（個別エントリと集計）を返す。
#[tauri::command]
pub fn pet_traces(app: AppHandle) -> Result<Value, String> {
    let tracer = app.state::<crate::tracer::Tracer>();
    Ok(json!({
        "entries": tracer.snapshot(),
        "summary": tracer.summary(),
    }))
}

/// 画像付きメッセージをエージェントへ送信する。
/// `images` は base64 data URL のリスト（"data:image/jpeg;base64,..."）。
#[tauri::command]
pub async fn pet_send_image(
    text: String,
    images: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let text = text.trim().to_string();
    if text.is_empty() && images.is_empty() {
        return Ok(json!({ "reply": "" }));
    }
    // 画像の枚数・サイズ制限（1枚あたり最大 10MB base64、最大 8 枚）。
    if images.len() > 8 {
        return Err("画像が多すぎます（最大 8 枚まで）。".to_string());
    }
    for img in &images {
        if img.len() > 10 * 1024 * 1024 {
            return Err("画像サイズが大きすぎます（1枚 10MB まで）。".to_string());
        }
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

    let mut convo = state.messages.lock().unwrap().clone();
    let user_text = if text.is_empty() {
        "この画像について教えてください。".to_string()
    } else {
        text.clone()
    };
    convo.push(crate::message::Message::user_with_images(
        user_text.clone(),
        images,
    ));

    let result = agent::run(&state.config, provider, &model, &mut convo, 5, Some(&app)).await;
    let compacted = compacted_conversation(&convo);
    *state.messages.lock().unwrap() = compacted.clone();
    let character_id = active_character_id(&state);
    crate::history::save(&app, &character_id, &compacted);

    match result {
        Ok(reply) => {
            let new_emotion = {
                let current = state.emotion.lock().unwrap().clone();
                crate::emotion::update(&current, &user_text, &reply)
            };
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.emit(
                    "pet:emotion",
                    serde_json::to_value(&new_emotion).unwrap_or_default(),
                );
            }
            *state.emotion.lock().unwrap() = new_emotion;
            Ok(json!({ "reply": if reply.is_empty() { "……".to_string() } else { reply } }))
        }
        Err(e) => {
            let message = log_command_error(&app, "画像付き AI 応答に失敗しました", e);
            Ok(json!({ "error": true, "reply": format!("AIと話せなかったよ。{}", message) }))
        }
    }
}

/// デスクトップのスクリーンショットを撮影し PNG を base64 data URL で返す。
/// プラットフォームごとに利用可能なツールを順に試みる。
/// Windows: PowerShell + GDI、macOS: screencapture、Linux: grim → scrot → import の順。
#[tauri::command]
pub async fn pet_capture_screen() -> Result<Value, String> {
    use base64::Engine;

    let temp_path = std::env::temp_dir().join("picoagent-screenshot.png");

    #[cfg(target_os = "windows")]
    {
        let path_esc = temp_path.to_string_lossy().replace('\\', "\\\\");
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms,System.Drawing;\
             $s=[System.Windows.Forms.Screen]::PrimaryScreen;\
             $b=New-Object System.Drawing.Bitmap($s.Bounds.Width,$s.Bounds.Height);\
             $g=[System.Drawing.Graphics]::FromImage($b);\
             $g.CopyFromScreen($s.Bounds.Location,[System.Drawing.Point]::Empty,$s.Bounds.Size);\
             $b.Save(\"{path_esc}\");\
             $g.Dispose();$b.Dispose()"
        );
        let out = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .await
            .map_err(|e| format!("スクリーンショット取得に失敗しました: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "スクリーンショット取得に失敗しました: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("screencapture")
            .args(["-t", "png", temp_path.to_str().unwrap_or("")])
            .output()
            .await
            .map_err(|e| format!("スクリーンショット取得に失敗しました: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "スクリーンショット取得に失敗しました: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }

    #[cfg(target_os = "linux")]
    {
        let path_str = temp_path.to_str().unwrap_or("");
        // grim(Wayland) → scrot(X11) → import(ImageMagick) の順に試みる。
        let ok = tokio::process::Command::new("grim")
            .arg(path_str)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let ok2 = tokio::process::Command::new("scrot")
                .arg(path_str)
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok2 {
                let out = tokio::process::Command::new("import")
                    .args(["-window", "root", path_str])
                    .output()
                    .await
                    .map_err(|_| {
                        "スクリーンショット取得に失敗しました。grim / scrot / ImageMagick のいずれかをインストールしてください。".to_string()
                    })?;
                if !out.status.success() {
                    return Err("スクリーンショット取得に失敗しました。grim / scrot / ImageMagick のいずれかをインストールしてください。".to_string());
                }
            }
        }
    }

    // macOS/Windows/Linux 以外はスタブエラー
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    return Err("スクリーンショットはこのプラットフォームでは未対応です。".to_string());

    let bytes = tokio::fs::read(&temp_path)
        .await
        .map_err(|e| format!("スクリーンショットファイルの読み込みに失敗しました: {e}"))?;
    let _ = tokio::fs::remove_file(&temp_path).await;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(json!({ "dataUrl": format!("data:image/png;base64,{b64}") }))
}

/// ドロップ・ペーストされたファイルの内容をテキストとして抽出する。
/// name は拡張子で種別を判定するために使う。data_b64 は data URL またはプレーン base64。
/// 対応フォーマット: テキスト系・CSV・Excel（xlsx）・Word（docx）・PowerPoint（pptx）。
/// PDF はパスを取得できないため pdf_tools スキルへ誘導するメッセージを返す。
#[tauri::command]
pub async fn pet_read_file_content(name: String, data_b64: String) -> Result<String, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    // 巨大ファイル/ZIP爆弾による OOM を避けるため入力サイズを制限する（base64 は ~4/3 倍）。
    const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;
    if data_b64.len() > MAX_FILE_BYTES / 3 * 4 + 1024 {
        return Err("ファイルが大きすぎます（25MB まで）。".to_string());
    }
    // data URL プレフィックスを除去する。
    let b64 = if let Some(pos) = data_b64.find(";base64,") {
        &data_b64[pos + 8..]
    } else {
        data_b64.as_str()
    };
    let bytes = B64
        .decode(b64.trim())
        .map_err(|e| format!("base64 デコード失敗: {e}"))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("ファイルが大きすぎます（25MB まで）。".to_string());
    }

    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    const MAX_CHARS: usize = 12_000;
    let content = match ext.as_str() {
        // テキスト系・コード系はそのまま UTF-8 で読む。
        "txt" | "md" | "markdown" | "rst" | "log" | "json" | "toml" | "yaml" | "yml" | "html"
        | "htm" | "xml" | "css" | "js" | "ts" | "jsx" | "tsx" | "py" | "rs" | "go" | "java"
        | "c" | "cpp" | "h" | "hpp" | "sh" | "bash" | "zsh" | "bat" | "ps1" | "rb" | "php"
        | "kt" | "swift" => String::from_utf8_lossy(&bytes).into_owned(),
        // CSV/TSV → Markdown テーブル（最大 300 行）。
        "csv" | "tsv" => {
            let sep = if ext == "tsv" { '\t' } else { ',' };
            let text = String::from_utf8_lossy(&bytes);
            // 表示は最大 300 行なので 301 行だけ取り込む（巨大ファイルでも全行を抱えない）。
            let rows: Vec<Vec<String>> = text
                .lines()
                .take(301)
                .map(|line| line.split(sep).map(|c| c.to_string()).collect())
                .collect();
            rows_to_markdown_table(&rows)
        }
        // Excel → calamine でシート別 Markdown テーブル。
        "xlsx" | "xlsm" => extract_xlsx_content(&bytes)?,
        // Word → ZIP 展開 + XML ストリップ。
        "docx" | "docm" => {
            let xml = read_zip_entry(&bytes, "word/document.xml")?;
            strip_xml_tags(&xml)
        }
        // PowerPoint → スライドごとにテキスト抽出。
        "pptx" | "pptm" => extract_pptx_content(&bytes)?,
        // PDF → スキル誘導メッセージ。
        "pdf" => {
            format!(
                "[添付ファイル: {name}]\n\
                 ※ PDF の本文抽出には pdf_tools スキルが必要です。\
                 ファイルパスを直接伝えてもらうか、「このPDFを要約して」と依頼してください。"
            )
        }
        _ => match String::from_utf8(bytes.clone()) {
            Ok(s) => s,
            Err(_) => {
                return Err(format!(
                    "「{name}」は対応していないファイル形式です。\
                     対応: txt/md/csv/tsv/json/toml/yaml/xlsx/docx/pptx とコードファイル。"
                ));
            }
        },
    };

    if content.chars().count() > MAX_CHARS {
        let truncated: String = content.chars().take(MAX_CHARS).collect();
        let omitted = content.chars().count() - MAX_CHARS;
        Ok(format!(
            "{truncated}\n\n[...{omitted} 文字以降は省略しました]"
        ))
    } else {
        Ok(content)
    }
}

fn strip_xml_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    let mut last_space = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            _ if in_tag => {}
            '\n' | '\r' | '\t' | ' ' => {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            c => {
                out.push(c);
                last_space = false;
            }
        }
    }
    out.trim().to_string()
}

/// セル行の並びを Markdown テーブルにする。先頭行を見出しとして区切り線を入れ、最大 300 行で打ち切る。
/// CSV/TSV と Excel で共通利用する。
fn rows_to_markdown_table(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for (i, cells) in rows.iter().enumerate().take(300) {
        out.push_str(&cells.join(" | "));
        out.push('\n');
        if i == 0 && rows.len() > 1 {
            out.push_str(&cells.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
            out.push('\n');
        }
    }
    if rows.len() > 300 {
        out.push_str("[300行以降は省略]\n");
    }
    out
}

/// ZIP エントリ 1 件の展開上限（ZIP爆弾対策）。
const MAX_ZIP_ENTRY_BYTES: u64 = 20 * 1024 * 1024;

fn read_zip_entry(data: &[u8], entry: &str) -> Result<String, String> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("ZIP オープン失敗: {e}"))?;
    let file = archive
        .by_name(entry)
        .map_err(|_| format!("'{entry}' が見つかりません"))?;
    // 展開サイズを上限で打ち切る（巨大展開による OOM を防ぐ）。
    let mut buf = Vec::new();
    file.take(MAX_ZIP_ENTRY_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn extract_pptx_content(data: &[u8]) -> Result<String, String> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("ZIP オープン失敗: {e}"))?;
    let mut slides: Vec<(String, String)> = Vec::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| e.to_string())?;
        let fname = file.name().to_string();
        if fname.starts_with("ppt/slides/slide") && fname.ends_with(".xml") {
            let mut buf = Vec::new();
            let _ = file.take(MAX_ZIP_ENTRY_BYTES).read_to_end(&mut buf);
            let xml = String::from_utf8_lossy(&buf).into_owned();
            slides.push((fname, strip_xml_tags(&xml)));
        }
    }
    slides.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(slides
        .into_iter()
        .enumerate()
        .map(|(i, (_, text))| format!("--- スライド {} ---\n{}", i + 1, text))
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn extract_xlsx_content(data: &[u8]) -> Result<String, String> {
    use calamine::{open_workbook_from_rs, Reader, Xlsx};
    let cursor = std::io::Cursor::new(data.to_vec());
    let mut wb: Xlsx<_> = open_workbook_from_rs(cursor)
        .map_err(|e| format!("Excel ファイルの読み込みに失敗しました: {e}"))?;
    let sheet_names: Vec<String> = wb.sheet_names().to_vec();
    let mut output = String::new();
    for sheet_name in sheet_names {
        if let Ok(range) = wb.worksheet_range(&sheet_name) {
            output.push_str(&format!("## {sheet_name}\n"));
            // 表示は最大 300 行なので 301 行だけ取り込む（巨大シートでも全行を抱えない）。
            let rows: Vec<Vec<String>> = range
                .rows()
                .take(301)
                .map(|row| row.iter().map(|c| c.to_string()).collect())
                .collect();
            output.push_str(&rows_to_markdown_table(&rows));
            output.push('\n');
        }
    }
    Ok(output)
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

#[cfg(test)]
mod tests {
    use super::speech_text;

    #[test]
    fn 読み上げでは参考ブロックを除外する() {
        let text =
            "今日は晴れです。\n\n参考:\n- [天気](https://example.com)\nhttps://example.com/raw";
        assert_eq!(speech_text(text), "今日は晴れです。");
    }

    #[test]
    fn 読み上げでは本文内リンクをラベルだけ読む() {
        let text = "[公式サイト](https://example.com)を確認しました。";
        assert_eq!(speech_text(text), "公式サイトを確認しました。");
    }

    #[test]
    fn 読み上げではチャート記号と絵文字を外す() {
        let text = "天気 東京 ☀️ 快晴\n今日の気温 18〜27度 ▁▂▃▅▇";
        let spoken = speech_text(text);
        assert!(!spoken.contains('☀'));
        assert!(!spoken.contains('▇'));
        assert!(spoken.contains("快晴") && spoken.contains("18〜27度"));
    }
}
