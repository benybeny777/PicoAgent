// lib.rs - PicoAgent の Tauri アプリ本体。
// コネクタ（providers / tools）・設定・エージェント・コマンド・メニューを束ねて起動する。

mod agent;
mod aivis;
mod commands;
mod config;
mod engine;
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
            commands::pet_speak,
            commands::pet_set_bubble_size,
            commands::pet_set_character_profile,
            commands::settings_load,
            commands::settings_list_models,
            commands::settings_save,
            commands::engine_status,
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
            state.config.load(config_dir.join("config.json"));

            let pid = state.config.get("CODA_PROVIDER", "local");
            *state.provider_id.lock().unwrap() = pid.clone();
            let model = {
                let m = state.config.get("CODA_MODEL", "");
                if m.is_empty() {
                    providers::get(&pid)
                        .map(|p| p.default_model(&state.config))
                        .unwrap_or_default()
                } else {
                    m
                }
            };
            *state.model.lock().unwrap() = model;
            state.reset_conversation();

            // メインウィンドウを作業領域の右下へ寄せ、設定値で常に手前を反映。
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_always_on_top(state.config.get_bool("CODA_ALWAYS_ON_TOP", true));
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
