// lib.rs - PicoAgent の Tauri アプリ本体。
// コネクタ（providers / tools）・設定・エージェント・コマンド・メニューを束ねて起動する。

mod agent;
mod aivis;
mod alarm;
mod api_server;
#[cfg(test)]
mod bundle_scope;
mod commands;
mod config;
mod daily;
#[cfg(test)]
mod doc_sync;
mod emotion;
mod engine;
#[cfg(test)]
mod guard_scan;
mod history;
mod log_paths;
mod mcp;
mod memory;
mod menu;
mod message;
mod migration;
mod permissions;
mod providers;
mod relationship;
mod scheduler;
mod secret;
mod store;
mod text;
mod tools;
mod tracer;
mod usage;

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
        .manage(WindowLayoutCache::default())
        .manage(tracer::Tracer::new(200))
        .on_window_event(on_main_window_event)
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
            commands::pet_relationship_state,
            commands::pet_relationship_event,
            commands::pet_relationship_pin_moment,
            commands::pet_relationship_delete_moment,
            commands::pet_relationship_clear,
            commands::pet_complete_onboarding,
            commands::settings_load,
            commands::settings_list_models,
            commands::settings_test_tool,
            commands::settings_save,
            commands::pet_schedule_runs,
            commands::pet_rerun_schedule_task,
            commands::pet_open_settings,
            commands::restart_app,
            commands::engine_status,
            commands::pet_list_downloaded_models,
            commands::pet_delete_downloaded_model,
            commands::pet_log,
            commands::pet_emotion,
            commands::pet_usage,
            commands::pet_traces,
            commands::pet_cancel,
            commands::pet_transcribe_audio,
            commands::pet_export_history,
            commands::pet_quit,
            commands::pet_send_image,
            commands::pet_capture_screen,
            commands::pet_read_file_content,
            commands::pet_read_dropped_path,
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
            if let Some(warning) = state.config.load(config_path.clone()) {
                log_paths::append_app_log(app.handle(), &warning);
            }
            // 既存の平文 API キー等を DPAPI 暗号化へ移行する（次回保存を待たず保護する）。
            commands::encrypt_existing_secrets(&state.config);
            // ログ置き場は起動時に作っておき、Aivis と同梱エンジンのログをここへ集約する。
            if let Err(error) = log_paths::ensure_log_dir(app.handle()) {
                eprintln!("ログディレクトリの作成に失敗しました: {error}");
            } else {
                // 1 週間より古い日付のログを削除してから当日分を書き始める。
                log_paths::prune_old_logs(app.handle());
                log_paths::append_app_log(
                    app.handle(),
                    &format!(
                        "PicoAgent v{} を起動しました（{} {}）。",
                        env!("CARGO_PKG_VERSION"),
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ),
                );
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
            let relationship_mode = state.config.get("PICOAGENT_COMPANION_MODE", "both");
            match relationship::load(app.handle(), &character_id, &relationship_mode) {
                Ok(relationship) => {
                    *state.relationship_context.lock().unwrap() =
                        relationship::context(&relationship);
                }
                Err(error) => log_paths::append_app_log(
                    app.handle(),
                    &format!("関係性データの読み込みに失敗しました: {error}"),
                ),
            }
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
                    // 横位置は以前指定の固定値（既定の右オフセット）。縦位置だけ保存値から復元する（解像度非依存）。
                    let bottom_offset = state
                        .config
                        .get("PICOAGENT_WINDOW_BOTTOM_PX", "")
                        .parse::<i32>()
                        .unwrap_or(DEFAULT_BOTTOM_OFFSET);
                    let (x, y) = position_from_offsets(
                        area.width,
                        area.height,
                        win_w,
                        win_h,
                        DEFAULT_RIGHT_OFFSET,
                        bottom_offset,
                    );
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
            // ローカル API サーバ（既定無効。PICOAGENT_API_ENABLED=true で 127.0.0.1 に開く）。
            api_server::start(app.handle().clone());
            // 外部 MCP サーバ（設定 PICOAGENT_MCP_SERVERS があれば起動してツールを動的登録）。
            mcp::client::start(app.handle().clone());
            // 起動時に一度だけ更新を確認する（オフラインや未リリースでは静かにスキップ）。
            commands::spawn_update_check(app.handle().clone());
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
            mcp::client::stop_all();
        }
    });
}

// 既定の右下オフセット（px）。right はモニタ右端からウィンドウ右端までの距離（負＝外へオーバーハング）。
const DEFAULT_RIGHT_OFFSET: i32 = -69;
const DEFAULT_BOTTOM_OFFSET: i32 = 51;
// 画面端スナップのしきい値（px）。
const SNAP_THRESHOLD: i32 = 24;

/// ウィンドウの移動／リサイズで確定したレイアウトを覚えておくキャッシュ。
/// 終了時 (`RunEvent::Exit`) はウィンドウが破棄され座標を取得できないため、生存中のイベントで控えておく。
/// 横位置（右端基準）は固定なので覚えない。縦位置（下端からのオフセット）とサイズだけを保持する。
#[derive(Default)]
pub(crate) struct WindowLayoutCache {
    inner: std::sync::Mutex<Option<CachedLayout>>,
}

#[derive(Clone, Copy)]
struct CachedLayout {
    width: f64,  // 論理px
    height: f64, // 論理px
    bottom_offset: i32,
}

/// 右下からの相対オフセットでウィンドウ左上座標を求める（解像度非依存）。
pub(crate) fn position_from_offsets(
    area_w: u32,
    area_h: u32,
    win_w: u32,
    win_h: u32,
    right_offset: i32,
    bottom_offset: i32,
) -> (i32, i32) {
    let x = area_w as i32 - win_w as i32 - right_offset;
    let y = area_h as i32 - win_h as i32 - bottom_offset;
    (x.max(0), y.max(0))
}

/// 既定の右下オフセットでのウィンドウ位置。
pub(crate) fn default_window_position(
    area_w: u32,
    area_h: u32,
    win_w: u32,
    win_h: u32,
) -> (i32, i32) {
    position_from_offsets(
        area_w,
        area_h,
        win_w,
        win_h,
        DEFAULT_RIGHT_OFFSET,
        DEFAULT_BOTTOM_OFFSET,
    )
}

/// メインウィンドウの移動／リサイズ時に、画面端スナップと位置オフセットのキャッシュを行う。
fn on_main_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if !matches!(
        event,
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
    ) {
        return;
    }
    if matches!(window.is_fullscreen(), Ok(true)) {
        return;
    }
    let (Ok(Some(monitor)), Ok(pos), Ok(size)) = (
        window.current_monitor(),
        window.outer_position(),
        window.outer_size(),
    ) else {
        return;
    };
    let area = monitor.size();
    let ah = area.height as i32;
    let (w, h) = (size.width as i32, size.height as i32);

    // 横位置（右端基準）は以前指定の固定値なので触らない。縦位置だけ上端スナップする。
    if pos.y.abs() <= SNAP_THRESHOLD && pos.y != 0 {
        // 再配置すると再度 Moved が飛ぶので、そこでキャッシュする。
        let _ = window.set_position(PhysicalPosition::new(pos.x, 0));
        return;
    }

    // サイズ（論理px）と下端からの縦オフセットをキャッシュ（終了時に保存）。横位置は覚えない。
    let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
    let layout = CachedLayout {
        width: w as f64 / scale,
        height: h as f64 / scale,
        bottom_offset: ah - (pos.y + h),
    };
    *window.state::<WindowLayoutCache>().inner.lock().unwrap() = Some(layout);
}

/// 終了時に、生存中のイベントで控えておいたサイズと縦オフセットを設定へ保存する。
/// `RunEvent::Exit` ではウィンドウ座標を取得できないため、キャッシュ値のみを書き出す。
/// 横位置は以前指定の固定値を使うため保存しない。
fn save_window_layout_on_exit(app_handle: &tauri::AppHandle) {
    let Some(layout) = *app_handle
        .state::<WindowLayoutCache>()
        .inner
        .lock()
        .unwrap()
    else {
        return;
    };
    let state = app_handle.state::<AppState>();
    let mut patch = serde_json::Map::new();
    patch.insert(
        "PICOAGENT_WINDOW_WIDTH_PX".to_string(),
        serde_json::json!(layout.width.round().to_string()),
    );
    patch.insert(
        "PICOAGENT_WINDOW_HEIGHT_PX".to_string(),
        serde_json::json!(layout.height.round().to_string()),
    );
    patch.insert(
        "PICOAGENT_WINDOW_BOTTOM_PX".to_string(),
        serde_json::json!(layout.bottom_offset.to_string()),
    );
    if let Err(error) = state.config.set_all(patch) {
        log_paths::append_app_log(
            app_handle,
            &format!("終了時のウィンドウ位置・サイズ保存に失敗しました: {error}"),
        );
    }
}
