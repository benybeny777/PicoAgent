// lib.rs - PicoAgent の Tauri アプリ本体。
// コネクタ（providers / tools）・設定・エージェント・コマンド・メニューを束ねて起動する。

mod agent;
mod aivis;
mod alarm;
mod api_server;
mod boot_screen;
#[cfg(test)]
mod bundle_scope;
mod character_pack;
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
mod hotkey;
mod layout;
mod log_paths;
mod mcp;
mod memory;
mod menu;
mod message;
mod migration;
mod permissions;
mod providers;
mod relationship;
mod relationship_preset;
mod scheduler;
mod secret;
mod store;
mod text;
mod tha3_creator;
mod tools;
mod tracer;
mod usage;
mod user_characters;

use tauri::{LogicalSize, Manager, PhysicalPosition, RunEvent};

use aivis::AivisState;
use boot_screen::BootScreenState;
use commands::AppState;
use engine::EngineState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // キャラクターパックを選ぶネイティブダイアログ（Rust 側だけで使う）。
        .plugin(tauri_plugin_dialog::init())
        // グローバルホットキー（他のアプリの裏に居ても呼び出せるようにする）。
        // 個々のキー登録は設定を読んでから hotkey::apply が行う。
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::new())
        .manage(EngineState::new())
        .manage(AivisState::new())
        .manage(BootScreenState::new())
        .manage(tha3_creator::Tha3CreatorState::new())
        .manage(WindowLayoutCache::default())
        .manage(tracer::Tracer::new(200))
        // フロントを読み込めなかったときの案内画面（配信経路に依存しないよう埋め込みを返す）。
        .register_uri_scheme_protocol(boot_screen::SCHEME, |ctx, _request| {
            boot_screen::serve(ctx.app_handle())
        })
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
            commands::pet_set_character_offset,
            commands::pet_set_bubble_layout,
            commands::pet_set_character_profile,
            commands::pet_relationship_state,
            commands::pet_relationship_event,
            commands::pet_relationship_pin_moment,
            commands::pet_relationship_delete_moment,
            commands::pet_relationship_clear,
            commands::pet_complete_onboarding,
            commands::settings_load,
            commands::settings_character_pick,
            commands::settings_character_install,
            commands::settings_character_remove,
            commands::settings_tha3_status,
            commands::settings_tha3_pick_source,
            commands::settings_tha3_pick_root,
            commands::settings_tha3_generate,
            commands::settings_tha3_cancel,
            commands::pet_user_characters,
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
            commands::pet_frontend_ready,
            commands::pet_finish_boot,
            commands::pet_reset_layout,
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

            // 選択中の表示形式をウィンドウ寸法へ反映する。保存済みレイアウトの復元より先に
            // 行い、ユーザーが手で決めた寸法があればそちらで上書きされるようにする。
            {
                let id = layout::current(&state.config);
                if let Err(error) = layout::apply(app.handle(), &id) {
                    log_paths::append_app_log(app.handle(), &error);
                }
            }

            // グローバルホットキーを設定どおりに登録する（失敗しても本体は使えるので記録に留める）。
            if let Err(error) = hotkey::apply(app.handle()) {
                log_paths::append_app_log(app.handle(), &error);
            }

            // 保存済みのサイズと「どのディスプレイのどこに居たか」を復元する。
            if let Some(win) = app.get_webview_window("main") {
                let _ =
                    win.set_always_on_top(state.config.get_bool("PICOAGENT_ALWAYS_ON_TOP", true));
                restore_window_layout(&win, &state.config);
            }
            // 保存中の形式が chat / talk / bar でも、起動準備中だけは共通の
            // VTuber画面を見せる。準備完了時にフロントから保存形式へ戻す。
            if let Err(error) = layout::apply_boot(app.handle()) {
                log_paths::append_app_log(app.handle(), &error);
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
            // フロントの準備完了を見張り、時間内に来なければキャラクター付きの案内画面へ切り替える。
            boot_screen::watch(app.handle().clone());
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
            // OS へ確保したグローバルホットキーを明示的に返す。
            hotkey::release(app_handle);
            tha3_creator::shutdown(app_handle);
            aivis::stop(app_handle);
            engine::stop(app_handle);
            mcp::client::stop_all();
            tools::run_command::jobs::stop_all();
            // browser_control が起動したブラウザも一緒に閉じる（残すとデバッグポートが開いたままになる）。
            tools::browser_control::stop();
        }
    });
}

// 既定の右下オフセット（px）。ウィンドウ全体が選択中モニタへ収まる余白を取る。
const DEFAULT_RIGHT_OFFSET: i32 = 12;
const DEFAULT_BOTTOM_OFFSET: i32 = 51;
// 画面端スナップのしきい値（px）。
const SNAP_THRESHOLD: i32 = 24;

/// ウィンドウの移動／リサイズで確定したレイアウトを覚えておくキャッシュ。
/// 終了時 (`RunEvent::Exit`) はウィンドウが破棄され座標を取得できないため、生存中のイベントで控えておく。
/// 位置は**仮想デスクトップ全体の物理座標**で持つ。マルチモニタでは各モニタが
/// 原点の異なる矩形として並ぶため、モニタ内の相対オフセットだけでは
/// 「どのディスプレイに居たか」を復元できない。
#[derive(Default)]
pub(crate) struct WindowLayoutCache {
    inner: std::sync::Mutex<Option<CachedLayout>>,
}

#[derive(Clone, Copy)]
struct CachedLayout {
    width: f64,  // 論理px
    height: f64, // 論理px
    x: i32,      // 仮想デスクトップ上の物理px
    y: i32,      // 仮想デスクトップ上の物理px
}

/// モニタ矩形（仮想デスクトップ上の物理座標）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// 指定モニタの右下オフセットからウィンドウ左上座標を求める。
/// **モニタ原点を必ず加える**（原点を無視すると、左側に並ぶモニタ＝負の X を持つ
/// 環境で常に主モニタ側へ吸い寄せられ、意図しない画面に出る）。
pub(crate) fn position_from_offsets(
    monitor: MonitorRect,
    win_w: u32,
    win_h: u32,
    right_offset: i32,
    bottom_offset: i32,
) -> (i32, i32) {
    let x = monitor.x + monitor.width as i32 - win_w as i32 - right_offset;
    let y = monitor.y + monitor.height as i32 - win_h as i32 - bottom_offset;
    let max_x = monitor.x + monitor.width.saturating_sub(win_w) as i32;
    let max_y = monitor.y + monitor.height.saturating_sub(win_h) as i32;
    (x.clamp(monitor.x, max_x), y.clamp(monitor.y, max_y))
}

/// 既定の右下オフセットでのウィンドウ位置。
pub(crate) fn default_window_position(monitor: MonitorRect, win_w: u32, win_h: u32) -> (i32, i32) {
    position_from_offsets(
        monitor,
        win_w,
        win_h,
        DEFAULT_RIGHT_OFFSET,
        DEFAULT_BOTTOM_OFFSET,
    )
}

/// ウィンドウ矩形とモニタ矩形の重なり（幅, 高さ）。重なりが無ければ 0。
fn overlap(monitor: MonitorRect, x: i32, y: i32, win_w: u32, win_h: u32) -> (i32, i32) {
    let overlap_w =
        ((x + win_w as i32).min(monitor.x + monitor.width as i32) - x.max(monitor.x)).max(0);
    let overlap_h =
        ((y + win_h as i32).min(monitor.y + monitor.height as i32) - y.max(monitor.y)).max(0);
    (overlap_w, overlap_h)
}

/// 保存位置と最も重なるモニタを選び、ウィンドウ全体がその内側へ収まる座標に直す。
fn clamp_to_best_monitor(
    monitors: &[MonitorRect],
    x: i32,
    y: i32,
    win_w: u32,
    win_h: u32,
) -> Option<(i32, i32)> {
    let monitor = monitors.iter().copied().max_by_key(|monitor| {
        let (width, height) = overlap(*monitor, x, y, win_w, win_h);
        width as i64 * height as i64
    })?;
    let (overlap_w, overlap_h) = overlap(monitor, x, y, win_w, win_h);
    if overlap_w == 0 || overlap_h == 0 {
        return None;
    }
    let max_x = (monitor.x + monitor.width.saturating_sub(win_w) as i32).max(monitor.x);
    let max_y = (monitor.y + monitor.height.saturating_sub(win_h) as i32).max(monitor.y);
    Some((x.clamp(monitor.x, max_x), y.clamp(monitor.y, max_y)))
}

/// 保存座標のウィンドウが、どれかのモニタに**掴める大きさ**で載っているか。
/// ディスプレイを外した・主副を入れ替えた・解像度を変えた後に、画面外の座標へ復元して
/// 「起動したのに何も見えない」状態になるのを防ぐ。
#[cfg(test)]
pub(crate) fn is_visible_on_any_monitor(
    monitors: &[MonitorRect],
    x: i32,
    y: i32,
    win_w: u32,
    win_h: u32,
) -> bool {
    // 端にオーバーハングさせる使い方（既定位置も右へはみ出す）を許すため、
    // 完全内包ではなく「掴める大きさの重なりがあるか」で判定する。
    const MIN_VISIBLE_PX: i32 = 80;
    monitors.iter().any(|monitor| {
        let (overlap_w, overlap_h) = overlap(*monitor, x, y, win_w, win_h);
        overlap_w >= MIN_VISIBLE_PX.min(win_w as i32)
            && overlap_h >= MIN_VISIBLE_PX.min(win_h as i32)
    })
}

/// ウィンドウが 1px でも見えているか。
///
/// 実行中のディスプレイ構成変更（モニタの抜き差し・主副の入れ替え・解像度変更）で
/// ウィンドウが完全に画面外へ取り残されたことの検出に使う。
/// **保存位置の採否より緩い判定にしてある**（`is_visible_on_any_monitor` の 80px 基準で
/// 実行中に引き戻すと、端へ寄せる通常のドラッグ操作まで勝手に戻されてしまうため）。
pub(crate) fn is_on_screen(
    monitors: &[MonitorRect],
    x: i32,
    y: i32,
    win_w: u32,
    win_h: u32,
) -> bool {
    monitors.iter().any(|monitor| {
        let (overlap_w, overlap_h) = overlap(*monitor, x, y, win_w, win_h);
        overlap_w > 0 && overlap_h > 0
    })
}

/// 指定寸法にしたウィンドウが画面外へはみ出していたら、見える位置へ引き戻す。
///
/// 表示形式の切り替えでウィンドウが大きくなると、右下寄りに置いていた場合に
/// つまみごと画面外へ出てしまう。`set_size` 直後の `outer_size` は反映前の古い値を
/// 返しうるので、当てた寸法（物理px）を引数で受け取る。
pub(crate) fn pull_back_on_screen(window: &tauri::WebviewWindow, win_w: u32, win_h: u32) {
    let Ok(pos) = window.outer_position() else {
        return;
    };
    let Some(monitor) = current_monitor_rect(window) else {
        return;
    };
    let max_x = monitor.x + monitor.width.saturating_sub(win_w) as i32;
    let max_y = monitor.y + monitor.height.saturating_sub(win_h) as i32;
    let x = pos.x.clamp(monitor.x, max_x);
    let y = pos.y.clamp(monitor.y, max_y);
    if (x, y) != (pos.x, pos.y) {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
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
    let (w, h) = (size.width as i32, size.height as i32);

    // ディスプレイ構成の変更（モニタの抜き差し・主副の入れ替え・解像度変更）で
    // ウィンドウが完全に画面外へ取り残されることがある。Windows がウィンドウを
    // 動かし直すとここへ `Moved` が飛ぶので、その場で見える位置へ引き戻す。
    // **画面外の座標をキャッシュ＝保存させない**ことも兼ねる（次回起動で復元を
    // 諦めて既定位置へ飛ぶより、その場で直したほうが体験が良い）。
    let monitors: Vec<MonitorRect> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(to_rect)
        .collect();
    if !monitors.is_empty() && !is_on_screen(&monitors, pos.x, pos.y, size.width, size.height) {
        let rect = to_rect(&monitor);
        let (x, y) = default_window_position(rect, size.width, size.height);
        if (x, y) != (pos.x, pos.y) {
            log_paths::append_app_log(
                &window.app_handle().clone(),
                &format!(
                    "ディスプレイ構成の変更でウィンドウが画面外 ({}, {}) になったため、既定位置へ戻します。",
                    pos.x, pos.y
                ),
            );
            // 再配置すると再度 Moved が飛ぶので、そこでキャッシュする。
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
        return;
    }

    // 上端スナップは**そのモニタの上端**を基準にする（仮想デスクトップの y=0 ではない。
    // 上下に並ぶ配置では y=0 が別の画面を指してしまう）。
    let monitor_top = monitor.position().y;
    if (pos.y - monitor_top).abs() <= SNAP_THRESHOLD && pos.y != monitor_top {
        // 再配置すると再度 Moved が飛ぶので、そこでキャッシュする。
        let _ = window.set_position(PhysicalPosition::new(pos.x, monitor_top));
        return;
    }

    // サイズ（論理px）と仮想デスクトップ上の絶対座標をキャッシュ（終了時に保存）。
    let scale = window.scale_factor().unwrap_or(1.0).max(1.0);
    let layout = CachedLayout {
        width: w as f64 / scale,
        height: h as f64 / scale,
        x: pos.x,
        y: pos.y,
    };
    *window.state::<WindowLayoutCache>().inner.lock().unwrap() = Some(layout);
}

fn to_rect(monitor: &tauri::window::Monitor) -> MonitorRect {
    MonitorRect {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
    }
}

/// 接続中モニタの矩形一覧（仮想デスクトップ上の物理座標）。
/// モニタ列挙とウィンドウ座標（`outer_position` / `set_position`）は同じ座標系で、
/// どちらも主モニタ左上が原点。そのまま重なり判定に使える。
fn monitor_rects(window: &tauri::WebviewWindow) -> Vec<MonitorRect> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(to_rect)
        .collect()
}

/// 現在のモニタ矩形（取得できなければ主モニタ、それも無ければ None）。
fn current_monitor_rect(window: &tauri::WebviewWindow) -> Option<MonitorRect> {
    let monitor = match window.current_monitor() {
        Ok(Some(monitor)) => Some(monitor),
        _ => window.primary_monitor().ok().flatten(),
    }?;
    Some(to_rect(&monitor))
}

/// 保存済みのサイズと絶対座標からウィンドウを復元する。
/// 座標が現在のモニタ構成のどこにも載らない（ディスプレイを外した等）場合は、
/// 既定の右下位置へ置き直す。
pub(crate) fn restore_window_layout(window: &tauri::WebviewWindow, config: &config::Config) {
    let Some(monitor) = current_monitor_rect(window) else {
        return;
    };
    let Ok(scale) = window.scale_factor() else {
        return;
    };
    let scale = scale.max(1.0);
    let Ok(current) = window.outer_size() else {
        return;
    };

    // サイズ: 全画面時の汚染などでモニタを超える保存値が来てもモニタ内へ収める。
    let (mut win_w, mut win_h) = (current.width, current.height);
    if let (Ok(width), Ok(height)) = (
        config.get("PICOAGENT_WINDOW_WIDTH_PX", "").parse::<f64>(),
        config.get("PICOAGENT_WINDOW_HEIGHT_PX", "").parse::<f64>(),
    ) {
        let width = width.min(monitor.width as f64 / scale);
        let height = height.min(monitor.height as f64 / scale);
        if width >= 420.0 && height >= 340.0 {
            let _ = window.set_size(LogicalSize::new(width, height));
            // 位置計算は適用後の物理サイズを使う（set_size の反映前に outer_size を読むと古い値になる）。
            win_w = (width * scale).round() as u32;
            win_h = (height * scale).round() as u32;
        }
    }

    // 位置: 保存された絶対座標をそのまま使う（＝同じディスプレイの同じ場所へ戻る）。
    let saved = (
        config.get("PICOAGENT_WINDOW_LEFT_PX", "").parse::<i32>(),
        config.get("PICOAGENT_WINDOW_TOP_PX", "").parse::<i32>(),
    );
    let monitors = monitor_rects(window);
    let (x, y) = match saved {
        (Ok(x), Ok(y)) => match clamp_to_best_monitor(&monitors, x, y, win_w, win_h) {
            Some(position) => position,
            None => {
                // 認識しているモニタ構成も残す。ディスプレイの増減・配置変更・DPI 設定で
                // 座標系がずれたときに、ログだけで原因を追えるようにするため。
                let layout = monitors
                    .iter()
                    .map(|m| format!("({}, {}) {}x{}", m.x, m.y, m.width, m.height))
                    .collect::<Vec<_>>()
                    .join(" / ");
                log_paths::append_app_log(
                &window.app_handle().clone(),
                &format!(
                    "保存されたウィンドウ位置 ({x}, {y}) {win_w}x{win_h} が現在のディスプレイ構成に載らないため、既定位置へ戻します。認識中のモニタ: {layout}"
                ),
            );
                default_window_position(monitor, win_w, win_h)
            }
        },
        _ => default_window_position(monitor, win_w, win_h),
    };
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// 終了時に、生存中のイベントで控えておいたサイズと絶対座標を設定へ保存する。
/// `RunEvent::Exit` ではウィンドウ座標を取得できないため、キャッシュ値のみを書き出す。
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
        "PICOAGENT_WINDOW_LEFT_PX".to_string(),
        serde_json::json!(layout.x.to_string()),
    );
    patch.insert(
        "PICOAGENT_WINDOW_TOP_PX".to_string(),
        serde_json::json!(layout.y.to_string()),
    );
    if let Err(error) = state.config.set_all(patch) {
        log_paths::append_app_log(
            app_handle,
            &format!("終了時のウィンドウ位置・サイズ保存に失敗しました: {error}"),
        );
    }
}

#[cfg(test)]
mod window_layout_tests {
    use super::*;

    /// 主モニタ（必ず原点 (0,0)）と、その左隣に置かれたサブモニタ（原点が負）。
    /// Windows の仮想デスクトップは主モニタ左上が原点なので、左側のモニタは負の X を持つ。
    fn two_monitors() -> Vec<MonitorRect> {
        vec![
            MonitorRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1200,
            },
            MonitorRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
        ]
    }

    /// 主モニタを右側の 2560x1440 から左側の 1920x1200 へ入れ替えた構成。
    /// 主モニタが原点になるため、**同じ物理配置でも全モニタの座標が変わる**。
    fn primary_swapped() -> Vec<MonitorRect> {
        vec![
            MonitorRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1200,
            },
            MonitorRect {
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440,
            },
        ]
    }

    #[test]
    fn 既定位置はモニタ原点を加えて求める() {
        // 原点が負のモニタで、モニタ原点を無視すると主モニタ側の座標になってしまう。
        let left = MonitorRect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1200,
        };
        let (x, y) = default_window_position(left, 560, 430);
        assert!(
            x < 0,
            "左側モニタの既定位置が主モニタ側へ吸い寄せられています: x={x}"
        );
        assert!(
            x >= left.x && y >= left.y,
            "モニタ矩形の外に出ています: ({x}, {y})"
        );
    }

    #[test]
    fn 保存座標がモニタ上にあれば有効と判定する() {
        let monitors = two_monitors();
        // サブモニタ上の座標。
        assert!(is_visible_on_any_monitor(&monitors, -1155, 487, 560, 430));
        // 主モニタ上の座標。
        assert!(is_visible_on_any_monitor(&monitors, 2069, 959, 560, 430));
    }

    #[test]
    fn 保存位置が一部はみ出していても起動時はモニタ内へ収める() {
        let monitors = two_monitors();
        assert_eq!(
            clamp_to_best_monitor(&monitors, 2500, 1200, 680, 430),
            Some((1880, 1010))
        );
        assert_eq!(
            clamp_to_best_monitor(&monitors, -2100, -80, 680, 430),
            Some((-1920, 0))
        );
    }

    #[test]
    fn 保存位置が全モニタから外れていれば復元しない() {
        assert_eq!(
            clamp_to_best_monitor(&two_monitors(), 6000, 3000, 680, 430),
            None
        );
    }

    #[test]
    fn モニタを外した後の座標は無効と判定する() {
        // サブモニタを外した構成では、そこに保存されていた座標は画面外になる。
        let only_main = vec![MonitorRect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        }];
        assert!(!is_visible_on_any_monitor(&only_main, -1155, 487, 560, 430));
    }

    #[test]
    fn 主モニタを入れ替えると保存座標が無効になりうる() {
        // 主副の入れ替えで仮想デスクトップの原点が動くため、同じ物理位置でも座標が変わる。
        // 旧構成のサブモニタ上（負のX）だった座標は、新構成では画面外になる。
        assert!(is_visible_on_any_monitor(
            &two_monitors(),
            -1000,
            300,
            560,
            430
        ));
        assert!(!is_visible_on_any_monitor(
            &primary_swapped(),
            -1000,
            300,
            560,
            430
        ));
    }

    #[test]
    fn 解像度を下げると画面外になった座標を検出する() {
        let before = vec![MonitorRect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        }];
        let after = vec![MonitorRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }];
        // 旧解像度の右下寄りにあった位置は、解像度を下げると載らなくなる。
        assert!(is_visible_on_any_monitor(&before, 2200, 1300, 560, 430));
        assert!(!is_visible_on_any_monitor(&after, 2200, 1300, 560, 430));
    }

    #[test]
    fn モニタ本数が増えた構成でも保存座標は有効なまま() {
        // モニタを足すだけなら既存の座標は載り続ける（無駄に既定位置へ戻さない）。
        let mut more = primary_swapped();
        more.push(MonitorRect {
            x: 4480,
            y: 0,
            width: 1920,
            height: 1080,
        });
        assert!(is_visible_on_any_monitor(&more, 2500, 500, 560, 430));
        assert!(is_visible_on_any_monitor(&more, 4600, 200, 560, 430));
    }

    #[test]
    fn 実行中の引き戻しはドラッグでの端寄せを邪魔しない() {
        let monitors = primary_swapped();
        // 掴める大きさは残っていない（保存には不適）が、まだ見えている位置。
        // 実行中の引き戻しはこれを動かしてはいけない。
        assert!(is_on_screen(&monitors, -540, 300, 560, 430));
        assert!(!is_visible_on_any_monitor(&monitors, -540, 300, 560, 430));
        // 完全に画面外なら引き戻す。
        assert!(!is_on_screen(&monitors, -560, 300, 560, 430));
    }

    #[test]
    fn 既定位置はモニタ内へ収まる() {
        let monitors = two_monitors();
        let main = monitors[1];
        let (x, y) = default_window_position(main, 560, 430);
        assert!(is_visible_on_any_monitor(&monitors, x, y, 560, 430));
        assert!(x >= main.x && y >= main.y);
        assert!(x + 560 <= main.x + main.width as i32);
        assert!(y + 430 <= main.y + main.height as i32);
    }
}
