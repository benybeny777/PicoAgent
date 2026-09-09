// picoagent.js - 旧 Electron preload（window.picoagent）を Tauri 上で再現する互換シム。
// tauri.conf.json の app.withGlobalTauri=true により window.__TAURI__ が使える前提。
// これにより renderer.js / settings.js は Electron 版とほぼ同じコードで動く。

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;
const { LogicalSize, PhysicalPosition } = window.__TAURI__.dpi || {};
const eventUnlisteners = new Set();
let eventListenersClosing = false;

// フロントとTauriの橋渡しで起きた失敗を app.log へ残す。pet_log 自身は再帰を避けて
// 素の invoke を使い、最後の失敗だけ開発者コンソールへ出す。
function logBridgeError(context, error) {
  const detail = error && (error.stack || error.message) ? error.stack || error.message : String(error);
  try {
    invoke("pet_log", { level: "error", message: `${context} 失敗`, detail: String(detail) }).catch(logError => {
      // フォールバック許可: pet_log 自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
      console.warn("invoke エラーログ保存に失敗しました", logError);
    });
  } catch (logError) {
    // フォールバック許可: pet_log 自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
    console.warn("invoke エラーログ保存に失敗しました", logError);
  }
}
// invoke 失敗は呼び出し側で処理されても診断ログへ残す。
function logInvokeError(name, error) {
  logBridgeError(`invoke ${name}`, error);
}
async function call(name, args) {
  try {
    return await invoke(name, args);
  } catch (error) {
    logInvokeError(name, error);
    throw error;
  }
}

async function managedListen(channel, callback) {
  const unlisten = await listen(channel, (event) => {
    try {
      const result = callback(event.payload);
      if (result && typeof result.then === "function") {
        result.catch(error => logBridgeError(`event ${channel} callback`, error));
      }
    } catch (error) {
      logBridgeError(`event ${channel} callback`, error);
    }
  });
  if (eventListenersClosing) {
    try { unlisten(); }
    catch (error) { logBridgeError(`event ${channel} release`, error); }
    return () => {};
  }
  let active = true;
  const release = () => {
    if (!active) return;
    active = false;
    eventUnlisteners.delete(release);
    try { unlisten(); }
    catch (error) { logBridgeError(`event ${channel} release`, error); }
  };
  eventUnlisteners.add(release);
  return release;
}

window.addEventListener("beforeunload", () => {
  eventListenersClosing = true;
  for (const release of [...eventUnlisteners]) release();
});

window.picoagent = {
  init: () => call("pet_init"),
  send: (text) => call("pet_send", { text }),
  // 実行中のエージェント処理へ中断を要求する（次のステップ境界で停止）。
  cancel: () => call("pet_cancel"),
  // マイク録音（WAV base64）をローカル Whisper で文字起こしする。
  transcribeAudio: (dataB64) => call("pet_transcribe_audio", { dataB64 }),
  // 会話履歴 Markdown をダウンロードフォルダへ書き出す。
  exportHistory: (markdown) => call("pet_export_history", { markdown }),
  idleChatter: () => call("pet_idle_chatter"),
  dailyGreeting: () => call("pet_daily_greeting"),
  dailyEvents: () => call("pet_daily_events"),
  // 自動追記した AGENTS.md の作法を1行取り消す。
  undoProjectRule: (rule) => call("pet_project_rule_undo", { rule }),
  // 永続ジョブ（AI Drive転送・成果物生成）の一覧と操作。
  durableJobs: () => call("pet_durable_jobs"),
  retryDurableJob: (id) => call("pet_durable_job_retry", { id }),
  cancelDurableJob: (id) => call("pet_durable_job_cancel", { id }),
  scheduleRuns: () => call("pet_schedule_runs"),
  rerunScheduleTask: (taskId) => call("pet_rerun_schedule_task", { taskId }),
  openSettings: (tab = null) => call("pet_open_settings", { tab }),
  quit: () => call("pet_quit"),
  alarms: () => call("pet_alarms"),
  alarmWake: (id) => call("pet_alarm_wake", { id }),
  speak: (text) => call("pet_speak", { text }),
  prepareVoiceModel: () => call("pet_prepare_voice_model"),
  log: (level, message, detail) => invoke("pet_log", { level, message, detail }),
  // フロントを読み込めたことの通知（届かないと案内画面へ切り替わる）。
  frontendReady: () => call("pet_frontend_ready"),
  // モデル・音声の取得中も起動処理が進行中であることを監視側へ伝える。
  bootProgress: () => call("pet_boot_progress"),
  // 共通VTuber起動画面から、保存中の表示形式と寸法へ戻す。
  finishBoot: () => call("pet_finish_boot"),
  // 配置編集（OBS風）の位置・倍率を設定へ保存する（確定時だけ呼ぶ）。
  saveLayoutPlacements: (placements) => call("pet_save_layout_placements", { placements }),
  // ウィンドウとフキダシの位置・サイズを既定へ戻す。
  resetLayout: () => call("pet_reset_layout"),
  aivisStatus: () => call("aivis_status"),
  // 自分で追加したキャラクター（app_data_dir/characters 配下）。
  userCharacters: () => call("pet_user_characters"),
  // 追加キャラクターの実ファイルを webview から読める URL へ変換する。
  assetUrl: (path) => convertFileSrc(path),
  // ウィンドウをフルスクリーン化／解除する（フキダシ全画面表示で使用）。
  setFullscreen: async (on) => {
    try { return await getCurrentWindow().setFullscreen(on); }
    catch (error) { logInvokeError("setFullscreen", error); throw error; }
  },
  setWindowSize: async (width, height) => {
    try {
      const size = LogicalSize ? new LogicalSize(width, height) : { type: "Logical", width, height };
      return await getCurrentWindow().setSize(size);
    }
    catch (error) { logInvokeError("setWindowSize", error); throw error; }
  },
  moveAndResizeWindow: async (leftDelta, topDelta, width, height) => {
    try {
      const win = getCurrentWindow();
      const scale = await win.scaleFactor();
      const position = await win.outerPosition();
      const nextX = Math.round(position.x - leftDelta * scale);
      const nextY = Math.round(position.y - topDelta * scale);
      const nextPosition = PhysicalPosition ? new PhysicalPosition(nextX, nextY) : { type: "Physical", x: nextX, y: nextY };
      const nextSize = LogicalSize ? new LogicalSize(width, height) : { type: "Logical", width, height };
      await win.setPosition(nextPosition);
      return await win.setSize(nextSize);
    }
    catch (error) { logInvokeError("moveAndResizeWindow", error); throw error; }
  },
  // 立ち絵の移動量（ウィンドウではなくキャラだけの位置）を保存する。
  setCharacterOffset: (x, y, characterId, layout, zoom) => call("pet_set_character_offset", { x, y, characterId, layout, zoom }),
  placeWindow: (position) => call("pet_place_window", { position }),
  // キャラ上の通常ドラッグをOS標準のウィンドウ移動へ渡す。
  startWindowDrag: () => call("pet_start_window_drag"),
  // 枠なしウィンドウの各辺・四隅から、OS標準の縦横リサイズを開始する。
  startWindowResize: async (direction) => {
    try { return await getCurrentWindow().startResizeDragging(direction); }
    catch (error) { logInvokeError("startResizeDragging", error); throw error; }
  },
  setBubbleSize: (width, height) => call("pet_set_bubble_size", { width, height }),
  setBubbleLayout: (width, height, left, top) => call("pet_set_bubble_layout", { width, height, left, top }),
  setCharacterProfile: (characterId, profile, greeting) => call("pet_set_character_profile", { characterId, profile, greeting }),
  relationshipState: () => call("pet_relationship_state"),
  relationshipEvent: (kind, detail = "") => call("pet_relationship_event", { kind, detail }),
  pinRelationshipMoment: (momentId, pinned) => call("pet_relationship_pin_moment", { momentId, pinned }),
  deleteRelationshipMoment: (momentId) => call("pet_relationship_delete_moment", { momentId }),
  clearRelationship: () => call("pet_relationship_clear"),
  completeOnboarding: () => call("pet_complete_onboarding"),
  showOnboarding: () => call("pet_show_onboarding"),
  menu: () => call("pet_menu"),
  on: (channel, callback) => {
    if (!["pet:model", "pet:reset", "pet:layout-reset", "pet:layout-edit", "engine:status", "pet:ui", "pet:reload", "pet:emotion", "pet:agent_status", "pet:notice", "pet:hotkey", "pet:layout", "pet:character-reset"].includes(channel)) return Promise.resolve(() => {});
    // Tauri のイベントは payload に値が入る。Electron の callback(value) 形に合わせる。
    return managedListen(channel, callback).catch(error => {
      logBridgeError(`event ${channel} subscribe`, error);
      return () => {};
    });
  },
  // Tauri イベントをリッスンして unlisten 関数を返す（ストリーミング等に使用）。
  listen: (channel, callback) => managedListen(channel, callback),
  // 感情状態を取得する。
  emotion: () => call("pet_emotion"),
  // 今月のトークン使用量を取得する。
  usage: () => call("pet_usage"),
  // ツール実行トレース情報を取得する。
  traces: () => call("pet_traces"),
  // 画像付きメッセージを送信する（images は base64 data URL の配列）。
  sendImage: (text, images) => call("pet_send_image", { text, images }),
  // デスクトップのスクリーンショットを PNG base64 data URL で返す。
  captureScreen: () => call("pet_capture_screen"),
  // ドロップされたファイルの内容を抽出してテキストを返す（xlsx/docx/pptx/csv/テキスト等）。
  readFileContent: (name, dataUrl) => call("pet_read_file_content", { name, dataB64: dataUrl }),
  // ネイティブ D&D のファイルパスを読み、添付用の kind/name/dataUrl/path を返す。
  readDroppedPath: (path) => call("pet_read_dropped_path", { path }),
  engineStatus: () => call("engine_status"),
  // ダウンロード済みローカルモデル(GGUF)の一覧を取得する。
  listDownloadedModels: () => call("pet_list_downloaded_models"),
  // ダウンロード済みモデルを1つ削除してディスクを解放する。
  deleteDownloadedModel: (name) => call("pet_delete_downloaded_model", { name }),
  restartApp: (restartEngine, restartAivis) => call("restart_app", { restartEngine: !!restartEngine, restartAivis: !!restartAivis }),
  settings: {
    load: () => call("settings_load"),
    listMemories: () => call("settings_memory_list"),
    deleteMemory: (category, text, updatedAt) => call("settings_memory_delete", { category, text, updatedAt }),
    listModels: (id) => call("settings_list_models", { id }),
    testTool: (name) => call("settings_test_tool", { name }),
    save: (patch) => call("settings_save", { patch }),
    // キャラクターパック（フォルダ / zip）の選択・導入・削除。
    userCharacters: () => call("pet_user_characters"),
    pickCharacterPack: (folder) => call("settings_character_pick", { folder }),
    installCharacter: (path) => call("settings_character_install", { path }),
    installLive2dCharacter: (path, id, name) => call("settings_character_install_live2d", { path, id, name }),
    renameCharacter: (id, name) => call("settings_character_rename", { id, name }),
    characterMakerList: () => call("settings_character_maker_list"),
    characterMakerPickSource: () => call("settings_character_maker_pick_source"),
    characterMakerCreate: (input) => call("settings_character_maker_create", { input }),
    characterMakerRun: (characterId, fromStage) => call("settings_character_maker_run", { characterId, fromStage }),
    characterMakerCancel: () => call("settings_character_maker_cancel"),
    characterMakerInstall: (characterId) => call("settings_character_maker_install", { characterId }),
    // 不具合の連絡へ添える診断情報を1ファイルへ書き出す（秘密値は含めない）。
    exportDiagnostics: () => call("settings_export_diagnostics"),
    componentStatus: () => call("settings_components_status"),
    installComponents: (ids, enableTts = false) => call("settings_components_install", { ids, enableTts }),
    removeComponent: (id) => call("settings_components_remove", { id }),
    checkUpdate: () => call("settings_check_update"),
    installUpdate: () => call("settings_install_update"),
    removeCharacter: (id) => call("settings_character_remove", { id }),
    pickStreamImage: (kind) => call("settings_pick_stream_image", { kind }),
  },
};
