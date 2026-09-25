// ゲーム1操作の所有者。入力権限・回数・記録はモデルへ委ねない。
mod platform;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Emitter;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::{Field, Message};
use crate::permissions::{self, SafePath};
use crate::providers::Provider;

const PROMPT: &str = include_str!("prompts/observe.md");
const KEYS: &[&str] = &[
    "up", "down", "left", "right", "z", "x", "c", "w", "a", "s", "d", "f", "enter", "space",
];
static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Budget {
    request: Weak<()>,
    owner: Weak<AtomicBool>,
    used: usize,
    history: Vec<Value>,
    stopped: bool,
}

fn budgets() -> &'static Mutex<Vec<Budget>> {
    static BUDGETS: OnceLock<Mutex<Vec<Budget>>> = OnceLock::new();
    BUDGETS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Futureの破棄も中断として扱う。入力済みの操作を再実行させない。
struct Operation {
    cancel: Arc<AtomicBool>,
    complete: bool,
    record: Option<SafePath>,
    input_attempted: bool,
    failure: Option<String>,
}

impl Operation {
    fn record_stop(&self) -> Result<(), String> {
        if let Some(path) = &self.record {
            let stopped_path = SafePath::app_owned(path.as_path().with_file_name("stopped.json"));
            save_json(
                &stopped_path,
                &json!({"status":"stopped", "input_attempted":self.input_attempted,
                "reason":self.failure.as_deref().unwrap_or("停止または処理中断。操作は再送していません。"),
                "recorded_at":chrono::Utc::now().to_rfc3339()}),
            )?;
        }
        Ok(())
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.complete {
            self.cancel.store(true, Ordering::SeqCst);
            if let Err(error) = self.record_stop() {
                let body = format!("ゲーム停止記録の保存に失敗: {error}");
                crate::log_paths::log_detached(&body);
                if let Some(app) = crate::durable_jobs::app_handle() {
                    if let Err(error) = app.emit(
                        "pet:notice",
                        json!({
                            "kind":"game_play", "title":"ゲーム操作の記録失敗",
                            "body":body, "ok":false, "at":chrono::Utc::now().to_rfc3339()
                        }),
                    ) {
                        crate::log_paths::log_detached(&format!(
                            "ゲーム停止記録の失敗を画面へ通知できません: {error}"
                        ));
                    }
                }
            }
        }
        BUSY.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Settings {
    title: String,
    keys: Vec<String>,
    controls: String,
    limit: usize,
}

impl Settings {
    fn read(cfg: &Config) -> Result<Self, String> {
        if !cfg.get_bool("PICOAGENT_GAME_ENABLED", false) {
            return Err("設定の「ゲームプレイ」を有効にしてください。".into());
        }
        let title = cfg
            .get("PICOAGENT_GAME_WINDOW_TITLE", "")
            .trim()
            .to_string();
        if title.is_empty() {
            return Err("設定に対象ゲームの正確なウィンドウ名を入力してください。".into());
        }
        let keys: Vec<String> = cfg
            .get("PICOAGENT_GAME_KEYS", "")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if keys.is_empty()
            || keys.len() > KEYS.len()
            || keys.iter().any(|s| !KEYS.contains(&s.as_str()))
        {
            return Err("ゲームの操作設定を確認し、許可キーをカンマ区切りで設定してください。使用可能: up,down,left,right,z,x,c,w,a,s,d,f,enter,space。".into());
        }
        let controls = cfg.get("PICOAGENT_GAME_CONTROLS", "");
        if controls.trim().is_empty() || controls.len() > 2000 {
            return Err("確認済みのキーと操作の対応を設定してください（例: w=上移動,a=左移動,s=下移動,d=右移動,f=攻撃）。".into());
        }
        let limit = cfg
            .get("PICOAGENT_GAME_MAX_OPS", "1")
            .parse::<usize>()
            .map_err(|_| "最大操作数は1〜100の整数で設定してください。".to_string())?;
        if !(1..=100).contains(&limit) {
            return Err("最大操作数は1〜100で設定してください。".into());
        }
        Ok(Self {
            title,
            keys,
            controls,
            limit,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    recognized: bool,
    confidence: f64,
    scene: String,
    hp: Option<u32>,
    hunger: Option<u32>,
    summary: String,
    action: Option<String>,
    comment: String,
}

fn validate(observation: &Observation, keys: &[String], decision: bool) -> Result<(), String> {
    if !observation.recognized
        || !(0.9..=1.0).contains(&observation.confidence)
        || observation.scene != "dungeon"
        || observation.hp.is_none_or(|hp| hp == 0)
        || observation.hunger.is_none_or(|hunger| hunger > 100)
        || observation.summary.trim().is_empty()
        || observation.summary.len() > 4000
        || observation.comment.len() > 1200
    {
        return Err(
            "必要な状況を確実に認識できないため停止しました。画面・HP・満腹度を確認してください。"
                .into(),
        );
    }
    if decision {
        let action = observation
            .action
            .as_ref()
            .ok_or("操作を確定できないため停止しました。")?;
        if !keys.contains(action) || observation.comment.trim().is_empty() {
            return Err(
                "許可されていない操作、または判断コメントの欠落により停止しました。".into(),
            );
        }
    } else if observation.action.is_some() {
        return Err("結果確認に新しい操作が含まれているため停止しました。".into());
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("ゲーム操作を停止しました。".into())
    } else {
        Ok(())
    }
}

fn stop_active() {
    if let Ok(budgets) = budgets().lock() {
        for budget in budgets
            .iter()
            .filter(|budget| budget.request.strong_count() > 0)
        {
            if let Some(owner) = budget.owner.upgrade() {
                owner.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn reserve(
    request: &Arc<()>,
    owner: &Arc<AtomicBool>,
    limit: usize,
    sequence: usize,
) -> Result<Vec<Value>, String> {
    let mut budgets = budgets().lock().map_err(|_| "操作回数を確認できません。")?;
    budgets.retain(|budget| budget.request.strong_count() > 0);
    let request_weak = Arc::downgrade(request);
    if !budgets
        .iter()
        .any(|budget| budget.request.ptr_eq(&request_weak))
    {
        budgets.push(Budget {
            request: request_weak.clone(),
            owner: Arc::downgrade(owner),
            ..Budget::default()
        });
    }
    let budget = budgets
        .iter_mut()
        .find(|budget| budget.request.ptr_eq(&request_weak))
        .ok_or("依頼の操作回数を確認できません。")?;
    if budget.stopped {
        return Err(
            "この依頼のゲーム操作は停止済みです。状態を確認して新しく依頼してください。".into(),
        );
    }
    if budget.used >= limit {
        return Err(format!(
            "この依頼の最大操作数（{limit}回）に達しました。続ける場合は新しく依頼してください。"
        ));
    }
    if sequence != budget.used + 1 {
        return Err(
            "操作番号が直前の操作と連続していません。古い操作を再送せず停止します。".into(),
        );
    }
    // 失敗・キャンセルした試行も枠を消費する。モデルから枠をリセットするAPIは設けない。
    budget.used += 1;
    Ok(budget.history.clone())
}

async fn recognize(
    cfg: &Config,
    png: &[u8],
    settings: &Settings,
    history: &[Value],
    decision: bool,
) -> Result<Observation, String> {
    let provider = crate::providers::local::Local;
    let model = provider.default_model(cfg);
    let prompt = json!({"目的": if decision {"次の1操作を選ぶ"} else {"操作後の結果確認だけ"}, "許可キー":settings.keys, "確認済み操作設定":settings.controls, "直近の観測記録":history});
    let messages = [
        Message::new("system", PROMPT),
        Message::user_with_images(
            prompt.to_string(),
            vec![format!("data:image/png;base64,{}", STANDARD.encode(png))],
        ),
    ];
    let response = tokio::time::timeout(Duration::from_secs(60), provider.chat(cfg, &model, &messages, &[]))
        .await.map_err(|_| "画面認識が60秒以内に完了しないため停止しました。".to_string())?
        .map_err(|_| "ローカルモデルの画面認識に失敗しました。モデルの画像対応とエンジンの状態を確認してください。".to_string())?;
    let content = crate::agent::guard_public_answer(response.content, PROMPT)?;
    let observation: Observation = serde_json::from_str(content.trim())
        .map_err(|_| "画面認識の応答が指定形式ではないため停止しました。".to_string())?;
    validate(&observation, &settings.keys, decision)?;
    Ok(observation)
}

fn save_json(path: &SafePath, data: &Value) -> Result<(), String> {
    let _guard = crate::store::lock_state("game_play_record");
    crate::store::write_json_atomic(path.as_path(), data)
        .map_err(|e| format!("ゲーム記録を保存できません: {e}"))
}

async fn step(cfg: &Config, operation: &mut Operation, sequence: usize) -> Result<String, String> {
    let cancel = &operation.cancel;
    let settings = Settings::read(cfg)?;
    if cfg.get("PICOAGENT_PROVIDER", "local") != "local" {
        return Err("ゲームプレイはPicoAgentのローカルモデルを選択して実行してください。".into());
    }
    crate::engine::ensure_vision_available(cfg).map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    let request = crate::agent::request_id().ok_or("依頼を識別できないため停止しました。")?;
    let history = reserve(&request, cancel, settings.limit, sequence)?;
    let target = platform::select(&settings.title)?;
    let root = cfg.config_dir()?.join("gameplay");
    let root_safe = SafePath::app_owned(root.clone());
    permissions::fs::create_dir_all(&root_safe)?;
    // 一意なアプリ所有ディレクトリを永続記録として残す。利用者由来パスは受け取らない。
    let folder = SafePath::app_owned(
        tempfile::Builder::new()
            .prefix("step-")
            .tempdir_in(root)
            .map_err(|e| format!("記録先を作成できません: {e}"))?
            .keep(),
    );
    permissions::fs::create_dir_all(&folder)?;
    let before_path = SafePath::app_owned(folder.as_path().join("before.png"));
    let after_path = SafePath::app_owned(folder.as_path().join("after.png"));
    let decision_path = SafePath::app_owned(folder.as_path().join("decision.json"));
    let result_path = SafePath::app_owned(folder.as_path().join("result.json"));
    operation.record = Some(SafePath::app_owned(result_path.as_path().to_path_buf()));
    let start = Instant::now();
    let before = platform::capture(&target)?;
    permissions::fs::write(&before_path, &before)?;
    let chosen = recognize(cfg, &before, &settings, &history, true).await?;
    check_cancel(cancel)?;
    if Settings::read(cfg)? != settings || start.elapsed() > Duration::from_secs(60) {
        return Err(
            "認識中に設定が変わったか画面の取得から時間が経過したため、入力せず停止しました。"
                .into(),
        );
    }
    let action = chosen
        .action
        .as_deref()
        .ok_or("操作を決定できませんでした。")?;
    if platform::capture(&target)? != before {
        return Err("判断中にゲーム画面が変化したため、古い状況で入力せず停止しました。アニメーションのある画面も現在は停止対象です。".into());
    }
    let decision = json!({"kind":"decision_at_action", "recorded_at":chrono::Utc::now().to_rfc3339(),
        "window":settings.title,"observation":chosen,"before":"before.png","status":"入力前。実行結果はresult.jsonを参照"});
    save_json(&decision_path, &decision)?;
    // 入力直前にフォーカス・PID・矩形を再検査。モデルには入力関数を渡さない。
    crate::engine::ensure_vision_available(cfg).map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    operation.input_attempted = true;
    platform::press(&target, action, cancel)?;
    let input_at = chrono::Utc::now().to_rfc3339();
    save_json(
        &result_path,
        &json!({"status":"input_sent_result_pending", "input_at":input_at, "action":action}),
    )?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    check_cancel(cancel)?;
    let after = platform::capture(&target)?;
    permissions::fs::write(&after_path, &after)?;
    if before == after {
        return Err(format!(
            "操作前後の画像が同じため、連打せず停止しました。記録: {}",
            folder.display()
        ));
    }
    let observed = recognize(
        cfg,
        &after,
        &settings,
        std::slice::from_ref(&decision),
        false,
    )
    .await?;
    check_cancel(cancel)?;
    let result = json!({"status":"observed_after_input","input_at":input_at,"observed_at":chrono::Utc::now().to_rfc3339(),
        "action":action,"observation":observed,"after":"after.png","decision":"decision.json"});
    save_json(&result_path, &result)?;
    let mut budgets = budgets().lock().map_err(|_| "履歴を保存できません。")?;
    let budget = budgets
        .iter_mut()
        .find(|budget| budget.request.ptr_eq(&Arc::downgrade(&request)))
        .ok_or("依頼の履歴を確認できません。")?;
    budget.history.push(result);
    if budget.history.len() > 16 {
        budget.history.remove(0);
    }
    Ok(format!("ゲームを1操作しました（{action}）。\n選択時のコメント: {}\n操作後の観測: {}\n記録: {}\n攻略成功を意味する判定ではありません。", chosen.comment, observed.summary, folder.display()))
}

pub struct GamePlayTool;

#[async_trait]
impl Tool for GamePlayTool {
    fn name(&self) -> &'static str {
        "game_play"
    }
    fn label(&self) -> &'static str {
        "ゲームプレイ"
    }
    fn description(&self) -> String {
        "Windowsの指定ゲームをローカル画像対応LLMで認識し、許可キーを1回だけ送って結果を記録する。stepの繰返し上限は利用者の1依頼ごとに適用。失敗時は停止し、desktop_control等で迂回しない。事前にゲームを前面にする。".into()
    }
    fn intent_keywords(&self) -> &'static [&'static str] {
        &[
            "ゲームを",
            "ゲームプレイ",
            "トルネコ",
            "1手進め",
            "一手進め",
        ]
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"action":{"type":"string","enum":["windows","step","stop"]},"sequence":{"type":"integer","minimum":1,"maximum":100,"description":"この依頼の操作番号。最初は1、正常終了ごとに1増やす。"}},"required":["action"],"additionalProperties":false})
    }
    fn examples(&self) -> Vec<&'static str> {
        vec!["トルネコを1手進めて", "ゲームプレイを停止して"]
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_GAME_ENABLED",
                "ゲームプレイを有効にする",
                "checkbox",
                "false",
            ),
            Field::new(
                "PICOAGENT_GAME_WINDOW_TITLE",
                "ゲームのウィンドウ名（完全一致）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_GAME_KEYS",
                "確認済みの許可キー（カンマ区切り）",
                "text",
                "w,a,s,d,f",
            ),
            Field::new(
                "PICOAGENT_GAME_CONTROLS",
                "確認済みのキーと操作の対応",
                "textarea",
                "w=上移動,a=左移動,s=下移動,d=右移動,f=攻撃",
            ),
            Field::new(
                "PICOAGENT_GAME_MAX_OPS",
                "1回の依頼の最大操作数（1〜100）",
                "number",
                "1",
            ),
        ]
    }
    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        if action == "stop" {
            stop_active();
            return "ゲームプレイを停止しました。".into();
        }
        if cfg.get("PICOAGENT_PROVIDER", "local") != "local" {
            return "エラー: ゲームプレイはローカルモデルを選択して利用してください。".into();
        }
        if !cfg.get_bool("PICOAGENT_GAME_ENABLED", false) {
            return super::config_guide(
                "ゲームプレイ",
                &["PICOAGENT_GAME_ENABLED"],
                "対象ウィンドウと確認済みの許可キーも設定してください。",
            );
        }
        if action == "windows" {
            return platform::windows().unwrap_or_else(|e| format!("エラー: {e}"));
        }
        let Some(cancel) = crate::agent::cancel_flag() else {
            return "エラー: 停止ボタンが使えるチャットからゲームプレイを依頼してください。".into();
        };
        if action != "step" {
            return "エラー: actionはwindows / step / stopを指定してください。".into();
        }
        if BUSY
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            stop_active();
            cancel.store(true, Ordering::SeqCst);
            return "エラー: ゲーム操作の重複要求を検出したため停止しました。".into();
        }
        let mut operation = Operation {
            cancel: cancel.clone(),
            complete: false,
            record: None,
            input_attempted: false,
            failure: None,
        };
        let sequence = match args.get("sequence") {
            None => 1,
            Some(value) => match value.as_u64().and_then(|n| usize::try_from(n).ok()) {
                Some(n) if (1..=100).contains(&n) => n,
                _ => return "エラー: 操作番号は1〜100の整数で指定してください。".into(),
            },
        };
        match step(cfg, &mut operation, sequence).await {
            Ok(result) => {
                operation.complete = true;
                result
            }
            Err(error) => {
                operation.failure = Some(error.clone());
                if let (Some(request), Ok(mut budgets)) =
                    (crate::agent::request_id(), budgets().lock())
                {
                    if let Some(budget) = budgets
                        .iter_mut()
                        .find(|budget| budget.request.ptr_eq(&Arc::downgrade(&request)))
                    {
                        budget.stopped = true;
                    }
                }
                let record_result = operation.record_stop();
                operation.complete = true;
                let location = operation
                    .record
                    .as_ref()
                    .map(|path| format!("\n記録先: {}", path.display()))
                    .unwrap_or_default();
                match record_result {
                    Ok(()) => format!("エラー: {error}\nゲームプレイを停止しました。同じ依頼では再操作しません。{location}"),
                    Err(record_error) => format!("エラー: {error}\nゲームプレイ停止。停止記録の保存にも失敗しました: {record_error}"),
                }
            }
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(GamePlayTool) } }

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> Observation {
        serde_json::from_value(
            json!({"recognized":true,"confidence":0.95,"scene":"dungeon",
            "hp":12,"hunger":90,"summary":"通路が見える","action":"up","comment":"通路へ移動する"}),
        )
        .unwrap()
    }

    #[test]
    fn 不明な状態と未許可キーと結果確認からの操作を拒否する() {
        let keys = vec!["up".into()];
        let mut value = observation();
        assert!(validate(&value, &keys, true).is_ok());
        value.hp = None;
        assert!(validate(&value, &keys, true).is_err());
        value = observation();
        value.confidence = f64::NAN;
        assert!(validate(&value, &keys, true).is_err());
        value = observation();
        value.scene = "menu".into();
        assert!(validate(&value, &keys, true).is_err());
        value = observation();
        value.action = Some("alt+f4".into());
        assert!(validate(&value, &keys, true).is_err());
        value = observation();
        assert!(validate(&value, &keys, false).is_err());
        value.action = None;
        assert!(validate(&value, &keys, false).is_ok());
    }

    #[test]
    fn 操作設定は空や修飾キーや範囲外を拒否する() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_GAME_ENABLED", "true");
        cfg.set("PICOAGENT_GAME_WINDOW_TITLE", "ゲーム");
        cfg.set("PICOAGENT_GAME_CONTROLS", "up=上移動,down=下移動");
        cfg.set("PICOAGENT_GAME_KEYS", "");
        assert!(Settings::read(&cfg).is_err());
        cfg.set("PICOAGENT_GAME_KEYS", "up,alt");
        assert!(Settings::read(&cfg).is_err());
        cfg.set("PICOAGENT_GAME_KEYS", "up,down");
        cfg.set("PICOAGENT_GAME_MAX_OPS", "0");
        assert!(Settings::read(&cfg).is_err());
        cfg.set("PICOAGENT_GAME_MAX_OPS", "1");
        assert_eq!(Settings::read(&cfg).unwrap().limit, 1);
        cfg.set("PICOAGENT_GAME_KEYS", "w,a,s,d,f,c");
        assert!(Settings::read(&cfg).is_ok());
        cfg.set("PICOAGENT_GAME_CONTROLS", "");
        assert!(Settings::read(&cfg).is_err());
        cfg.set("PICOAGENT_GAME_ENABLED", "false");
        assert!(Settings::read(&cfg).is_err());
    }

    #[tokio::test]
    async fn 同じ停止フラグの連続依頼は別予算で同一依頼の並列と委譲は共有する() {
        let owner = Arc::new(AtomicBool::new(false));
        crate::agent::with_cancel(owner.clone(), async {
            let request = crate::agent::request_id().unwrap();
            let (first, duplicate) =
                tokio::join!(async { reserve(&request, &owner, 2, 1) }, async {
                    reserve(&request, &owner, 2, 1)
                },);
            assert_ne!(first.is_ok(), duplicate.is_ok());
            crate::agent::with_cancel(owner.clone(), async {
                let nested = crate::agent::request_id().unwrap();
                assert!(Arc::ptr_eq(&request, &nested));
                assert!(reserve(&nested, &owner, 2, 1).is_err());
                assert!(reserve(&nested, &owner, 2, 2).is_ok());
                assert!(reserve(&nested, &owner, 2, 3).is_err());
            })
            .await;
            stop_active();
            assert!(check_cancel(&owner).is_err());
            let mut budgets = budgets().lock().unwrap();
            budgets
                .iter_mut()
                .find(|budget| budget.request.ptr_eq(&Arc::downgrade(&request)))
                .unwrap()
                .stopped = true;
        })
        .await;
        // GUIと同じく停止用Arcを再利用し、次の利用者依頼を開始する。
        owner.store(false, Ordering::SeqCst);
        crate::agent::with_cancel(owner.clone(), async {
            let request = crate::agent::request_id().unwrap();
            assert!(reserve(&request, &owner, 1, 1).is_ok());
            assert!(reserve(&request, &owner, 1, 2).is_err());
        })
        .await;
    }

    #[test]
    fn 別依頼が間に入っても生存中の依頼の予算を初期化しない() {
        let first = Arc::new(());
        let second = Arc::new(());
        let owner = Arc::new(AtomicBool::new(false));
        assert!(reserve(&first, &owner, 1, 1).is_ok());
        assert!(reserve(&second, &owner, 1, 1).is_ok());
        assert!(reserve(&first, &owner, 1, 1).is_err());
    }

    #[test]
    fn 中断時は入力時刻を残したまま別ファイルへ停止を記録する() {
        let temp = tempfile::tempdir().unwrap();
        let result = SafePath::app_owned(temp.path().join("result.json"));
        let sent = json!({"input_at":"2026-09-25T00:00:00Z","action":"up"});
        save_json(&result, &sent).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let _operation = Operation {
                cancel: cancel.clone(),
                complete: false,
                record: Some(result),
                input_attempted: true,
                failure: None,
            };
        }
        assert!(cancel.load(Ordering::SeqCst));
        let original: Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("result.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(original, sent);
        let stopped: Value = serde_json::from_str(
            &std::fs::read_to_string(temp.path().join("stopped.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stopped["input_attempted"], true);
    }
}
