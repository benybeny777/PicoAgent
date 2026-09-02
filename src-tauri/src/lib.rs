// lib.rs - PicoAgent の Tauri アプリ本体。
// コネクタ（providers / tools）・設定・エージェント・コマンド・メニューを束ねて起動する。

mod agent;
mod aivis;
mod alarm;
mod api_server;
mod boot_screen;
#[cfg(test)]
mod bundle_scope;
mod character_framing;
mod character_pack;
mod commands;
mod components;
mod config;
mod daily;
mod diagnostics;
#[cfg(test)]
mod doc_sync;
mod durable_jobs;
mod emotion;
mod engine;
mod engine_backend;
mod gpu;
#[cfg(test)]
mod guard_scan;
mod history;
mod hotkey;
mod layout;
mod license;
mod log_paths;
mod mcp;
mod memory;
mod menu;
mod message;
mod migration;
mod permissions;
mod project_rules;
mod providers;
mod relationship;
mod relationship_preset;
mod scheduler;
mod secret;
mod settings_memory;
mod store;
mod stream_profile;
mod text;
mod tha3_creator;
mod tools;
mod tracer;
mod usage;
mod user_characters;
mod user_skills;
mod window_surface;

use tauri::{Manager, PhysicalPosition, RunEvent};

use aivis::AivisState;
use boot_screen::BootScreenState;
use commands::AppState;
use engine::EngineState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // 多重起動を防ぐ。`store::lock_state` はプロセス内 Mutex なので、2つ目の
        // インスタンスが動くと設定・記憶・使用量・実行履歴を「全体を読んで全体を
        // 書き戻す」際に、片方の保存がもう片方の古いスナップショットで丸ごと
        // 上書きされる（＝利用者のデータが黙って消える）。スケジュールの二重実行や
        // エンジン・ローカルAPIのポート衝突も起きるため、入口で1つに絞る。
        // 2つ目の起動は、既にいるウィンドウを前面へ出して自分は終了する。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            crate::log_paths::append_app_log(
                app,
                "PicoAgent は既に起動しています。既存のウィンドウを前面へ出しました。",
            );
            crate::window_surface::surface_main_window(app);
        }))
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
            commands::pet_place_window,
            commands::pet_start_window_drag,
            commands::pet_set_bubble_layout,
            commands::pet_set_character_profile,
            commands::pet_relationship_state,
            commands::pet_relationship_event,
            commands::pet_relationship_pin_moment,
            commands::pet_relationship_delete_moment,
            commands::pet_relationship_clear,
            commands::pet_complete_onboarding,
            commands::pet_show_onboarding,
            commands::settings_components_status,
            commands::settings_components_install,
            commands::settings_components_remove,
            commands::settings_check_update,
            commands::settings_install_update,
            commands::settings_load,
            settings_memory::settings_memory_list,
            settings_memory::settings_memory_delete,
            commands::settings_character_pick,
            commands::settings_character_install,
            commands::settings_character_install_live2d,
            commands::settings_character_rename,
            commands::settings_character_remove,
            commands::settings_pick_stream_image,
            commands::settings_export_diagnostics,
            commands::settings_tha3_status,
            commands::settings_tha3_pick_source,
            commands::settings_tha3_pick_root,
            commands::settings_tha3_generate,
            commands::settings_tha3_cancel,
            commands::pet_user_characters,
            commands::settings_list_models,
            commands::settings_test_tool,
            commands::settings_save,
            commands::pet_project_rule_undo,
            commands::pet_durable_jobs,
            commands::pet_durable_job_retry,
            commands::pet_durable_job_cancel,
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
            commands::pet_boot_progress,
            commands::pet_finish_boot,
            commands::pet_reset_layout,
            menu::pet_menu,
        ])
        .on_menu_event(|app, event| {
            menu::handle_event(app, event.id().as_ref());
        })
        .setup(|app| {
            components::init(app.handle());
            // 設定の正本を読み込み、選択中プロバイダ・モデル・会話を初期化する。
            let state = app.state::<AppState>();
            // 設定まわりの失敗で `setup` を中断すると、`build()` の `expect` がそのまま
            // パニックになり、ウィンドウも案内画面も出ないまま終了する（＝利用者から見て
            // 「起動しない」）。保存先を決められない場合も、読み取り専用で起動だけは通す。
            match app.path().app_config_dir() {
                Ok(config_dir) => {
                    if let Err(error) = migration::migrate_identity_dirs(app.handle(), &config_dir)
                    {
                        log_paths::append_app_log(
                            app.handle(),
                            &format!("旧識別子からのデータ移行に失敗しました: {error}"),
                        );
                    }
                    let config_path = config_dir.join("config.json");
                    match state.config.load(config_path) {
                        Ok(Some(warning)) => log_paths::append_app_log(app.handle(), &warning),
                        Ok(None) => {}
                        Err(error) => log_paths::append_app_log(
                            app.handle(),
                            &state.config.start_read_only(format!(
                                "設定を読み込めないため、設定を保存しない読み取り専用状態で起動します: {error}"
                            )),
                        ),
                    }
                }
                Err(error) => log_paths::append_app_log(
                    app.handle(),
                    &state.config.start_read_only(format!(
                        "設定の保存先を取得できないため、設定を保存しない読み取り専用状態で起動します: {error}"
                    )),
                ),
            }
            // 削除した失敗版を選択したままの旧設定は、2Dむぎへ一度だけ移す。
            // 保存に失敗しても起動は続ける（読み取り専用起動・空き容量不足でも画面は出す）。
            match state.config.migrate_removed_character("mugi-live2d", "mugi") {
                Ok(true) => log_paths::append_app_log(
                    app.handle(),
                    "削除済みのむぎLive2D選択を2Dむぎへ移行しました。",
                ),
                Ok(false) => {}
                Err(error) => log_paths::append_app_log(
                    app.handle(),
                    &format!("削除済みのむぎLive2D選択を移行できませんでした: {error}"),
                ),
            }
            // 既存の平文 API キー等をOSの資格情報保護へ移行する（次回保存を待たず保護する）。
            commands::encrypt_existing_secrets(app.handle(), &state.config);
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
            // 旧版が短い挨拶へ追記して保存した内部ガイドは、選択中以外もまとめて原文へ戻す。
            history::migrate_saved_greeting_guidance(app.handle());
            // 選択中キャラの直近会話をファイルから文脈へ復元する（再起動をまたいで踏まえて話す）。
            match history::load_recent(app.handle(), &character_id) {
                Ok(recent) if !recent.is_empty() => {
                    state.messages.lock().unwrap().extend(recent);
                }
                Ok(_) => {}
                // 中身が分からない原本は `load_recent` が保存禁止に印を付けているので、
                // このあと会話しても短い新規履歴で上書きされない。
                Err(error) => log_paths::append_app_log(
                    app.handle(),
                    &format!(
                        "会話履歴を読み込めないため、履歴なしで起動します。原本への保存は行いません: {error}"
                    ),
                ),
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
                // 先行起動は同梱リソースの実行時ディレクトリへのコピー（同梱版で約0.8GB）を
                // 伴う。`setup` の中で待つと、その分だけウィンドウが出ない。ブロッキングI/Oを
                // 専用スレッドへ出してから起動確認へ進む。
                tauri::async_runtime::spawn(async move {
                    let background = handle.clone();
                    let started = tauri::async_runtime::spawn_blocking(move || {
                        aivis::start_background(&background)
                    })
                    .await;
                    match started {
                        Ok(Err(error)) => log_paths::append_app_log(
                            &handle,
                            &format!("同梱 Aivis のバックグラウンド起動に失敗しました: {error}"),
                        ),
                        Err(error) => log_paths::append_app_log(
                            &handle,
                            &format!(
                                "同梱 Aivis のバックグラウンド起動を実行できませんでした: {error}"
                            ),
                        ),
                        Ok(Ok(())) => {}
                    }
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
            durable_jobs::start(app.handle().clone());
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
    boot_user_move: std::sync::atomic::AtomicBool,
    boot_placement: std::sync::Mutex<Option<BootWindowPlacement>>,
}

#[derive(Clone, Copy)]
pub(crate) enum BootWindowPlacement {
    Position(i32, i32),
    CenterOnMonitor(i32, i32),
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

/// 収め直さずに尊重する重なりの下限。これだけ見えていれば掴んで動かせる。
const MIN_GRABBABLE_PX: i32 = 80;

/// 保存位置と最も重なるモニタを選び、掴める大きさが残っていなければ内側へ収め直す。
///
/// 端へわざと少しはみ出して置く使い方（デスクトップペットの定位置）は保存値のまま戻す。
/// どのモニタにも重なっていなければ `None`（＝既定位置へ戻すしかない）。
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
    if overlap_w >= MIN_GRABBABLE_PX.min(win_w as i32)
        && overlap_h >= MIN_GRABBABLE_PX.min(win_h as i32)
    {
        return Some((x, y));
    }
    let max_x = (monitor.x + monitor.width.saturating_sub(win_w) as i32).max(monitor.x);
    let max_y = (monitor.y + monitor.height.saturating_sub(win_h) as i32).max(monitor.y);
    Some((x.clamp(monitor.x, max_x), y.clamp(monitor.y, max_y)))
}

/// ウィンドウが 1px でも見えているか。
///
/// 実行中のディスプレイ構成変更（モニタの抜き差し・主副の入れ替え・解像度変更）で
/// ウィンドウが完全に画面外へ取り残されたことの検出に使う。
/// **起動時の `clamp_to_best_monitor()` より緩い判定にしてある**（実行中も画面内へ
/// 収め直すと、端へ寄せる通常のドラッグ操作まで勝手に戻されてしまうため）。
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

/// 現在モニタから完全に外れた場合だけ全モニタを取得し、どこにも見えていないか判定する。
/// 通常のドラッグ／リサイズでは列挙APIを呼ばないため、イベントごとのOS問い合わせを避けられる。
fn is_outside_all_monitors(
    current: MonitorRect,
    x: i32,
    y: i32,
    win_w: u32,
    win_h: u32,
    load_all: impl FnOnce() -> Vec<MonitorRect>,
) -> bool {
    if is_on_screen(std::slice::from_ref(&current), x, y, win_w, win_h) {
        return false;
    }
    let monitors = load_all();
    !monitors.is_empty() && !is_on_screen(&monitors, x, y, win_w, win_h)
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
        if let Err(error) = window.set_position(PhysicalPosition::new(x, y)) {
            log_paths::append_app_log(
                &window.app_handle().clone(),
                &format!("画面内へウィンドウを戻せませんでした: {error}"),
            );
        }
    }
}

/// 起動画面を現在モニタの中央へ置く。通常画面の保存位置は変更せず、起動完了時に戻す。
pub(crate) fn center_window_on_saved_monitor(
    window: &tauri::WebviewWindow,
    config: &config::Config,
    logical_width: f64,
    logical_height: f64,
) {
    let saved = (
        config.get("PICOAGENT_WINDOW_LEFT_PX", "").parse::<i32>(),
        config.get("PICOAGENT_WINDOW_TOP_PX", "").parse::<i32>(),
    );
    let monitors = monitor_rects(window);
    let saved_monitor = match saved {
        (Ok(x), Ok(y)) => monitors.iter().copied().find(|monitor| {
            x >= monitor.x
                && x < monitor.x + monitor.width as i32
                && y >= monitor.y
                && y < monitor.y + monitor.height as i32
        }),
        _ => None,
    };
    let Some(monitor) = saved_monitor.or_else(|| current_monitor_rect(window)) else {
        return;
    };
    let target_scale = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .find(|candidate| {
            candidate.position().x == monitor.x && candidate.position().y == monitor.y
        })
        .map(|candidate| candidate.scale_factor())
        .or_else(|| window.scale_factor().ok())
        .unwrap_or(1.0)
        .max(1.0);
    let win_w = (logical_width * target_scale).round() as u32;
    let win_h = (logical_height * target_scale).round() as u32;
    let x = monitor.x + monitor.width.saturating_sub(win_w) as i32 / 2;
    let y = monitor.y + monitor.height.saturating_sub(win_h) as i32 / 2;
    if let Err(error) = window.set_position(PhysicalPosition::new(x, y)) {
        log_paths::append_app_log(
            &window.app_handle().clone(),
            &format!("起動画面を中央へ配置できませんでした: {error}"),
        );
    }
}

pub(crate) fn mark_boot_window_drag(app: &tauri::AppHandle) {
    if layout::boot_size_active() {
        let cache = app.state::<WindowLayoutCache>();
        cache
            .boot_user_move
            .store(true, std::sync::atomic::Ordering::Release);
        *cache.boot_placement.lock().unwrap() = None;
    }
}

pub(crate) fn remember_boot_window_center(app: &tauri::AppHandle, monitor_x: i32, monitor_y: i32) {
    if !layout::boot_size_active() {
        return;
    }
    let cache = app.state::<WindowLayoutCache>();
    cache
        .boot_user_move
        .store(false, std::sync::atomic::Ordering::Release);
    *cache.boot_placement.lock().unwrap() =
        Some(BootWindowPlacement::CenterOnMonitor(monitor_x, monitor_y));
}

pub(crate) fn boot_window_placement(app: &tauri::AppHandle) -> Option<BootWindowPlacement> {
    let cache = app.state::<WindowLayoutCache>();
    let placement = *cache.boot_placement.lock().unwrap();
    placement
}

pub(crate) fn clear_boot_window_placement(app: &tauri::AppHandle) {
    let cache = app.state::<WindowLayoutCache>();
    cache
        .boot_user_move
        .store(false, std::sync::atomic::Ordering::Release);
    *cache.boot_placement.lock().unwrap() = None;
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
    // Windowsは最小化したウィンドウを (-32000,-32000) へ退避する。通常の画面外判定へ
    // 渡すと復元→最小化を繰り返し、位置移動不能と大量ログになるため保存も補正もしない。
    if matches!(window.is_minimized(), Ok(true)) {
        return;
    }
    if let tauri::WindowEvent::Moved(position) = event {
        if position.x <= -30000 && position.y <= -30000 {
            return;
        }
    }
    // 起動用680×430は一時状態なのでサイズへ混ぜない。ただし利用者が明示的に
    // ドラッグした位置は通常画面へ引き継ぐ。
    if layout::boot_size_active() {
        let cache = window.state::<WindowLayoutCache>();
        if matches!(event, tauri::WindowEvent::Moved(_))
            && cache
                .boot_user_move
                .load(std::sync::atomic::Ordering::Acquire)
        {
            if let Ok(position) = window.outer_position() {
                *cache.boot_placement.lock().unwrap() =
                    Some(BootWindowPlacement::Position(position.x, position.y));
            }
        }
        return;
    }
    let (Ok(Some(monitor)), Ok(pos), Ok(size)) = (
        window.current_monitor(),
        window.outer_position(),
        window.outer_size(),
    ) else {
        return;
    };
    if pos.x <= -30000 && pos.y <= -30000 {
        return;
    }
    let (w, h) = (size.width as i32, size.height as i32);

    // ディスプレイ構成の変更（モニタの抜き差し・主副の入れ替え・解像度変更）で
    // ウィンドウが完全に画面外へ取り残されることがある。Windows がウィンドウを
    // 動かし直すとここへ `Moved` が飛ぶので、その場で見える位置へ引き戻す。
    // **画面外の座標をキャッシュ＝保存させない**ことも兼ねる（次回起動で復元を
    // 諦めて既定位置へ飛ぶより、その場で直したほうが体験が良い）。
    let current_rect = to_rect(&monitor);
    // 通常のドラッグ中は現在モニタとの重なりだけで判定できる。全モニタ列挙は、
    // 現在モニタから完全に外れたとき（モニタ間の移動・構成変更）だけに限定する。
    let outside_all =
        is_outside_all_monitors(current_rect, pos.x, pos.y, size.width, size.height, || {
            window
                .available_monitors()
                .unwrap_or_default()
                .iter()
                .map(to_rect)
                .collect()
        });
    if outside_all {
        let (x, y) = default_window_position(current_rect, size.width, size.height);
        if (x, y) != (pos.x, pos.y) {
            log_paths::append_app_log(
                &window.app_handle().clone(),
                &format!(
                    "ディスプレイ構成の変更でウィンドウが画面外 ({}, {}) になったため、既定位置へ戻します。",
                    pos.x, pos.y
                ),
            );
            // 再配置すると再度 Moved が飛ぶので、そこでキャッシュする。
            if let Err(error) = window.set_position(PhysicalPosition::new(x, y)) {
                log_paths::append_app_log(
                    &window.app_handle().clone(),
                    &format!("画面外のウィンドウを既定位置へ戻せませんでした: {error}"),
                );
            }
        }
        return;
    }

    // 上端スナップは**そのモニタの上端**を基準にする（仮想デスクトップの y=0 ではない。
    // 上下に並ぶ配置では y=0 が別の画面を指してしまう）。
    let monitor_top = monitor.position().y;
    if (pos.y - monitor_top).abs() <= SNAP_THRESHOLD && pos.y != monitor_top {
        // 再配置すると再度 Moved が飛ぶので、そこでキャッシュする。
        if let Err(error) = window.set_position(PhysicalPosition::new(pos.x, monitor_top)) {
            log_paths::append_app_log(
                &window.app_handle().clone(),
                &format!("ウィンドウを画面上端へ寄せられませんでした: {error}"),
            );
        }
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
    // サイズの正本は表示形式ごとの `PICOAGENT_WINDOW_LAYOUTS` だけ。
    // 旧グローバル幅・高さで直前の形式を全形式へ上書きしない。
    let app = window.app_handle().clone();
    let id = layout::current(config);
    let (win_w, win_h) = layout::applied_physical_size(window, &app, &id);
    restore_window_position(window, config, win_w, win_h);
}

/// 保存された絶対座標だけを復元する。起動画面を中央表示した後、通常寸法へ戻す経路でも使う。
pub(crate) fn restore_window_position(
    window: &tauri::WebviewWindow,
    config: &config::Config,
    win_w: u32,
    win_h: u32,
) {
    let Some(monitor) = current_monitor_rect(window) else {
        return;
    };
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
    if let Err(error) = window.set_position(PhysicalPosition::new(x, y)) {
        log_paths::append_app_log(
            &window.app_handle().clone(),
            &format!("保存済みウィンドウ位置を復元できませんでした: {error}"),
        );
    }
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
    let id = layout::current(&state.config);
    if let Err(error) = layout::remember_size(app_handle, &id, layout.width, layout.height) {
        log_paths::append_app_log(app_handle, &error);
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
    fn モニタ内に収まった保存座標はそのまま復元する() {
        let monitors = two_monitors();
        // サブモニタ上の座標。
        assert_eq!(
            clamp_to_best_monitor(&monitors, -1155, 487, 560, 430),
            Some((-1155, 487))
        );
        // 主モニタ上の座標。右へ 69px はみ出していても、掴める大きさが残っていれば
        // 利用者が決めた定位置として尊重する。
        assert_eq!(
            clamp_to_best_monitor(&monitors, 2069, 959, 560, 430),
            Some((2069, 959))
        );
    }

    #[test]
    fn 掴めないほどはみ出した保存位置は起動時にモニタ内へ収める() {
        let monitors = two_monitors();
        // 右下に 60x240 しか残っていない位置（掴める幅 80px 未満）。
        assert_eq!(
            clamp_to_best_monitor(&monitors, 2500, 1200, 680, 430),
            Some((1880, 1010))
        );
        // 左モニタの上へ 30px しか残っていない位置。
        assert_eq!(
            clamp_to_best_monitor(&monitors, -2100, -400, 680, 430),
            Some((-1920, 0))
        );
        // 端へ寄せただけ（左に 500px 見えている）なら動かさない。
        assert_eq!(
            clamp_to_best_monitor(&monitors, -2100, -80, 680, 430),
            Some((-2100, -80))
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
        assert_eq!(
            clamp_to_best_monitor(&only_main, -1155, 487, 560, 430),
            None
        );
    }

    #[test]
    fn 主モニタを入れ替えると保存座標が無効になりうる() {
        // 主副の入れ替えで仮想デスクトップの原点が動くため、同じ物理位置でも座標が変わる。
        // 旧構成のサブモニタ上（負のX）だった座標は、新構成では画面外になる。
        assert_eq!(
            clamp_to_best_monitor(&two_monitors(), -1000, 300, 560, 430),
            Some((-1000, 300))
        );
        assert_eq!(
            clamp_to_best_monitor(&primary_swapped(), -1000, 300, 560, 430),
            None
        );
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
        // 旧解像度では右下に 360x140 が見えており、そのまま復元してよい。
        assert_eq!(
            clamp_to_best_monitor(&before, 2200, 1300, 560, 430),
            Some((2200, 1300))
        );
        // 解像度を下げると重なりが無くなり、既定位置へ戻すしかなくなる。
        assert_eq!(clamp_to_best_monitor(&after, 2200, 1300, 560, 430), None);
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
        assert_eq!(
            clamp_to_best_monitor(&more, 2500, 500, 560, 430),
            Some((2500, 500))
        );
        assert_eq!(
            clamp_to_best_monitor(&more, 4600, 200, 560, 430),
            Some((4600, 200))
        );
    }

    #[test]
    fn 実行中の引き戻しはドラッグでの端寄せを邪魔しない() {
        let monitors = primary_swapped();
        // 20px しか見えていない位置。起動時は画面内へ収め直す対象だが、
        // 実行中の引き戻しはこれを動かしてはいけない（端寄せドラッグの途中）。
        assert!(is_on_screen(&monitors, -540, 300, 560, 430));
        assert_eq!(
            clamp_to_best_monitor(&monitors, -540, 300, 560, 430),
            Some((0, 300))
        );
        // 完全に画面外なら引き戻す。
        assert!(!is_on_screen(&monitors, -560, 300, 560, 430));
    }

    #[test]
    fn 現在モニタ内の移動では全モニタを列挙しない() {
        use std::cell::Cell;

        let current = primary_swapped()[0];
        let enumerated = Cell::new(false);
        assert!(!is_outside_all_monitors(
            current,
            300,
            200,
            560,
            430,
            || {
                enumerated.set(true);
                primary_swapped()
            }
        ));
        assert!(!enumerated.get(), "通常の移動で全モニタを列挙している");
    }

    #[test]
    fn 現在モニタ外では他画面を確認してから引き戻しを決める() {
        use std::cell::Cell;

        let current = primary_swapped()[0];
        let enumerated = Cell::new(false);
        assert!(!is_outside_all_monitors(
            current,
            2400,
            300,
            560,
            430,
            || {
                enumerated.set(true);
                primary_swapped()
            }
        ));
        assert!(enumerated.get(), "他モニタへ移ったのに全体を確認していない");

        assert!(is_outside_all_monitors(
            current,
            7000,
            3000,
            560,
            430,
            primary_swapped,
        ));
    }

    #[test]
    fn 既定位置はモニタ内へ収まる() {
        let monitors = two_monitors();
        let main = monitors[1];
        let (x, y) = default_window_position(main, 560, 430);
        // 既定位置は収め直しの対象にならない（そのまま復元される）。
        assert_eq!(
            clamp_to_best_monitor(&monitors, x, y, 560, 430),
            Some((x, y))
        );
        assert!(x >= main.x && y >= main.y);
        assert!(x + 560 <= main.x + main.width as i32);
        assert!(y + 430 <= main.y + main.height as i32);
    }

    #[test]
    fn 四kの高dpiでも物理寸法で右下へ配置する() {
        let monitor = MonitorRect {
            x: 2560,
            y: -120,
            width: 3840,
            height: 2160,
        };
        // 1920x1080論理pxを150%表示した物理寸法。
        let (x, y) = default_window_position(monitor, 2880, 1620);
        assert_eq!((x, y), (3508, 369));
        assert_eq!(x + 2880 + DEFAULT_RIGHT_OFFSET, 6400);
        assert_eq!(y + 1620 + DEFAULT_BOTTOM_OFFSET, 2040);
    }
}
