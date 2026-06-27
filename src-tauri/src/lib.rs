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
mod providers;
mod tools;

use tauri::{Manager, PhysicalPosition, RunEvent};

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
            commands::pet_set_bubble_size,
            commands::pet_set_character_profile,
            commands::settings_load,
            commands::settings_list_models,
            commands::settings_save,
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
            let config_path = config_dir.join("config.json");
            state.config.load(config_path.clone());
            // ログ置き場は起動時に作っておき、Aivis と同梱エンジンのログをここへ集約する。
            if let Err(error) = log_paths::ensure_log_dir(app.handle()) {
                eprintln!("ログディレクトリの作成に失敗しました: {error}");
            } else {
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
                log_paths::append_app_log(&panic_handle, &format!("panic at {location}: {payload}"));
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
            state.reset_conversation();
            // 直近の会話履歴をファイルから文脈へ復元する（再起動をまたいで踏まえて話す）。
            let recent = history::load_recent(app.handle());
            if !recent.is_empty() {
                state.messages.lock().unwrap().extend(recent);
            }

            // メインウィンドウを作業領域の右下へ寄せ、設定値で常に手前を反映。
            if let Some(win) = app.get_webview_window("main") {
                let _ =
                    win.set_always_on_top(state.config.get_bool("PICOAGENT_ALWAYS_ON_TOP", true));
                if let (Ok(Some(monitor)), Ok(size)) = (win.current_monitor(), win.outer_size()) {
                    let area = monitor.size();
                    let x = area.width as i32 - size.width as i32 - 20;
                    let y = area.height as i32 - size.height as i32 - 40;
                    let _ = win.set_position(PhysicalPosition::new(x.max(0), y.max(0)));
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
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building PicoAgent");

    // 終了時にアプリが裏起動した子プロセスを確実に止める。
    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            aivis::stop(app_handle);
            engine::stop(app_handle);
        }
    });
}
