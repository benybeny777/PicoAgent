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
    /// 会話文脈が属する選択中キャラクターID。これが変わったら会話を破棄して
    /// 旧キャラの一人称・自己紹介が新キャラの応答へ混ざらないようにする。
    pub character_id: Mutex<String>,
    /// 会話から暗黙的に抽出した長期メモリ。system 文脈へ差し込む。
    pub memory_context: Mutex<String>,
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
            character_id: Mutex::new(String::new()),
            memory_context: Mutex::new(String::new()),
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
    let user_text = text.clone();

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

    let result = agent::run(&state.config, provider, &model, &mut convo, 5).await;
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

fn default_character_tts_voice(character_id: &str) -> Option<TtsVoice> {
    match character_id {
        "rin" => Some(TtsVoice {
            speaker: RIN_AIVIS_SPEAKER.to_string(),
            model_uuid: Some(RIN_AIVIS_MODEL_UUID.to_string()),
            speaker_uuid: Some(RIN_AIVIS_SPEAKER_UUID.to_string()),
            style_name: Some(RIN_AIVIS_STYLE_NAME.to_string()),
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
    // AivisHub モデル指定があり未導入なら導入する（導入確認は speaker_uuid で行う）。
    if let (Some(model_uuid), Some(speaker_uuid)) =
        (voice.model_uuid.as_deref(), voice.speaker_uuid.as_deref())
    {
        aivis::ensure_model_installed(&app, &endpoint, model_uuid, speaker_uuid)
            .await
            .map_err(|error| log_command_error(&app, "Aivis モデルの導入に失敗しました", error))?;
    }
    // speaker_uuid があれば実 style ID を `/speakers` から解決する（取り違え防止）。
    // 解決できなければ設定値の style ID を使う。
    let speaker = match voice.speaker_uuid.as_deref() {
        Some(speaker_uuid) => {
            aivis::resolve_style_id(&endpoint, speaker_uuid, voice.style_name.as_deref())
                .await
                .unwrap_or(voice.speaker)
        }
        None => voice.speaker,
    };
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
            "label": preset.label,
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
    let old_provider_id = state.config.get("PICOAGENT_PROVIDER", "local");
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
    state
        .config
        .set_all(patch)
        .map_err(|e| log_command_error(&app, "設定保存に失敗しました", e))?;

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
    if has_system_prompt || has_memory_setting {
        *state.memory_context.lock().unwrap() = crate::memory::context(&app, &state.config);
        state.reset_conversation();
    }
    if tts_runtime_changed {
        aivis::stop(&app);
        if state.config.get_bool("PICOAGENT_TTS_ENABLED", false) {
            let handle = app.clone();
            if let Err(error) = aivis::start_background(&handle) {
                crate::log_paths::append_app_log(
                    &handle,
                    &format!("設定保存後の同梱 Aivis バックグラウンド起動に失敗しました: {error}"),
                );
            }
            tauri::async_runtime::spawn(async move {
                if let Err(error) = aivis::ensure_started(&handle).await {
                    crate::log_paths::append_app_log(
                        &handle,
                        &format!("設定保存後の同梱 Aivis 起動確認に失敗しました: {error}"),
                    );
                }
            });
        }
    }
    if old_provider_id == "local" && pid != "local" {
        crate::engine::stop(&app);
    }
    if pid == "local" && (local_engine_changed || (provider_changed && old_provider_id != "local"))
    {
        crate::engine::stop(&app);
        let handle = app.clone();
        tauri::async_runtime::spawn(crate::engine::ensure_and_start(handle));
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
