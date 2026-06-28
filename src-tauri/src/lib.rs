// lib.rs - PicoAgent の Tauri アプリ本体。
// コネクタ（providers / tools）・設定・エージェント・コマンド・メニューを束ねて起動する。

mod agent;
mod aivis;
mod alarm;
mod commands;
mod config;
mod daily;
mod engine;
mod history;
mod log_paths;
mod memory;
mod menu;
mod message;
mod migration;
mod providers;
mod scheduler;
mod tools;

use tauri::{LogicalSize, Manager, PhysicalPosition, RunEvent};

use aivis::AivisState;
use commands::AppState;
use engine::EngineState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(AppState::new())
        .manage(EngineState::new())
        .manage(AivisState::new())
        .invoke_handler(tauri::generate_handler![
            commands::pet_init,
            commands::pet_send,
            commands::pet_idle_chatter,
            commands::pet_daily_events,
            commands::pet_daily_greeting,
            commands::pet_alarms,
            commands::pet_alarm_wake,
            commands::pet_speak,
            commands::aivis_status,
            commands::pet_prepare_voice_model,
            commands::pet_set_bubble_size,
            commands::pet_set_bubble_layout,
            commands::pet_set_character_profile,
            commands::settings_load,
            commands::settings_list_models,
            commands::settings_save,
            commands::restart_app,
            commands::engine_status,
            commands::pet_log,
            menu::pet_menu,
        ])
        .on_menu_event(|app, event| {
            menu::handle_event(app, event.id().as_ref());
        })
        .setup(|app| {
            // 設定の正本を読み込み、選択中プロバイダ・モデル・会話を初期化する。
            let state = app.state::<AppState>();
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            if let Err(error) = migration::migrate_identity_dirs(app.handle(), &config_dir) {
                log_paths::append_app_log(
                    app.handle(),
                    &format!("旧識別子からのデータ移行に失敗しました: {error}"),
                );
            }
            let config_path = config_dir.join("config.json");
            state.config.load(config_path.clone());
            // ログ置き場は起動時に作っておき、Aivis と同梱エンジンのログをここへ集約する。
            if let Err(error) = log_paths::ensure_log_dir(app.handle()) {
                eprintln!("ログディレクトリの作成に失敗しました: {error}");
            } else {
                // 1 週間より古い日付のログを削除してから当日分を書き始める。
                log_paths::prune_old_logs(app.handle());
                log_paths::append_app_log(app.handle(), "PicoAgent を起動しました。");
            }

            // バックエンドのパニックを app.log に記録する（release では stderr が見えないため、
            // すべての異常終了をログに残す）。既定フックも引き続き呼ぶ。
            let panic_handle = app.handle().clone();
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let location = info
                    .location()
                    .map(|loc| format!("{}:{}", loc.file(), loc.line()))
                    .unwrap_or_else(|| "不明".to_string());
                let payload = info
                    .payload()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| info.payload().downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "（詳細なし）".to_string());
                log_paths::append_app_log(
                    &panic_handle,
                    &format!("panic at {location}: {payload}"),
                );
                default_hook(info);
            }));

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
            *state.memory_context.lock().unwrap() = memory::context(app.handle(), &state.config);
            // 履歴はキャラクター別。選択中キャラを文脈の持ち主として記録しておく。
            let character_id = state.config.get("PICOAGENT_CHARACTER", "mugi");
            *state.character_id.lock().unwrap() = character_id.clone();
            state.reset_conversation();
            // 旧仕様の単一 history.json は、選択中キャラではなく既定キャラ mugi の
            // ファイルへ一度だけ移行する。旧履歴はキャラ別管理が無かった時代のもので、
            // 当時の唯一/既定キャラは mugi。選択中キャラ（後から追加した rin 等）へ
            // 割り当てると、別キャラの自己紹介・口調が混ざってしまうため。
            history::migrate_legacy(app.handle(), "mugi");
            // 選択中キャラの直近会話をファイルから文脈へ復元する（再起動をまたいで踏まえて話す）。
            let recent = history::load_recent(app.handle(), &character_id);
            if !recent.is_empty() {
                state.messages.lock().unwrap().extend(recent);
            }

            // 保存済みサイズを復元し、位置は解像度非依存の右下オフセットで配置する。
            if let Some(win) = app.get_webview_window("main") {
                let _ =
                    win.set_always_on_top(state.config.get_bool("PICOAGENT_ALWAYS_ON_TOP", true));
                if let (Ok(Some(monitor)), Ok(scale), Ok(current)) =
                    (win.current_monitor(), win.scale_factor(), win.outer_size())
                {
                    let area = monitor.size();
                    let scale = scale.max(1.0);
                    // 位置計算は適用後の物理サイズを使う（set_size の反映前に outer_size を読むと古い値になるため）。
                    let (mut win_w, mut win_h) = (current.width, current.height);
                    let saved_width = state.config.get("PICOAGENT_WINDOW_WIDTH_PX", "");
                    let saved_height = state.config.get("PICOAGENT_WINDOW_HEIGHT_PX", "");
                    if let (Ok(width), Ok(height)) =
                        (saved_width.parse::<f64>(), saved_height.parse::<f64>())
                    {
                        // 全画面時の汚染などでモニタを超える保存値が来ても、モニタ内へ収める。
                        let width = width.min(area.width as f64 / scale);
                        let height = height.min(area.height as f64 / scale);
                        if width >= 420.0 && height >= 340.0 {
                            let _ = win.set_size(LogicalSize::new(width, height));
                            win_w = (width * scale).round() as u32;
                            win_h = (height * scale).round() as u32;
                        }
                    }
                    let (x, y) = default_window_position(area.width, area.height, win_w, win_h);
                    let _ = win.set_position(PhysicalPosition::new(x, y));
                }
            }

            // 既定が local（同梱エンジン）なら、モデル取得とエンジン起動を非同期で開始する。
            if pid == "local" {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(engine::ensure_and_start(handle));
            }
            if state.config.get_bool("PICOAGENT_TTS_ENABLED", false) {
                let handle = app.handle().clone();
                if let Err(error) = aivis::start_background(&handle) {
                    log_paths::append_app_log(
                        &handle,
                        &format!("同梱 Aivis のバックグラウンド起動に失敗しました: {error}"),
                    );
                }
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = aivis::ensure_started(&handle).await {
                        log_paths::append_app_log(
                            &handle,
                            &format!("同梱 Aivis の起動確認に失敗しました: {error}"),
                        );
                    }
                });
            }
            // 長時間連続起動でも保持期間を守れるよう、1 日ごとに古いログを掃除する。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut tick =
                        tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
                    tick.tick().await; // 初回分は起動時の掃除と重複するので捨てる。
                    loop {
                        tick.tick().await;
                        log_paths::prune_old_logs(&handle);
                    }
                });
            }
            scheduler::start(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building PicoAgent");

    // 終了時にアプリが裏起動した子プロセスを確実に止める。
    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            save_window_layout_on_exit(app_handle);
            aivis::stop(app_handle);
            engine::stop(app_handle);
        }
    });
}

/// 画面右下を基準にしたウィンドウ位置（解像度非依存）。
/// 右端をモニタ右端より RIGHT_OVERHANG px 外へ出し、下端からは BOTTOM_GAP px 空ける。
pub(crate) fn default_window_position(area_w: u32, area_h: u32, win_w: u32, win_h: u32) -> (i32, i32) {
    const RIGHT_OVERHANG: i32 = 69;
    const BOTTOM_GAP: i32 = 51;
    let x = area_w as i32 - win_w as i32 + RIGHT_OVERHANG;
    let y = area_h as i32 - win_h as i32 - BOTTOM_GAP;
    (x.max(0), y.max(0))
}

/// 終了時に透明ウィンドウの現在サイズを設定へ保存し、次回起動で復元できるようにする。
/// 位置は解像度非依存にするため保存せず、起動時は常に右下オフセットで配置する。
fn save_window_layout_on_exit(app_handle: &tauri::AppHandle) {
    let Some(win) = app_handle.get_webview_window("main") else {
        return;
    };
    // 全画面のまま終了した場合はモニタ全体サイズを保存しないよう除外する。
    if matches!(win.is_fullscreen(), Ok(true)) {
        return;
    }
    let (Ok(size), Ok(scale)) = (win.outer_size(), win.scale_factor()) else {
        return;
    };
    let logical = size.to_logical::<f64>(scale);
    let state = app_handle.state::<AppState>();
    let mut patch = serde_json::Map::new();
    patch.insert(
        "PICOAGENT_WINDOW_WIDTH_PX".to_string(),
        serde_json::json!(logical.width.round().to_string()),
    );
    patch.insert(
        "PICOAGENT_WINDOW_HEIGHT_PX".to_string(),
        serde_json::json!(logical.height.round().to_string()),
    );
    if let Err(error) = state.config.set_all(patch) {
        log_paths::append_app_log(
            app_handle,
            &format!("終了時のウィンドウサイズ保存に失敗しました: {error}"),
        );
    }
}
