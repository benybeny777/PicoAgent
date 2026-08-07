// picoagent.js - 旧 Electron preload（window.picoagent）を Tauri 上で再現する互換シム。
// tauri.conf.json の app.withGlobalTauri=true により window.__TAURI__ が使える前提。
// これにより renderer.js / settings.js は Electron 版とほぼ同じコードで動く。

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;
const { LogicalSize, PhysicalPosition } = window.__TAURI__.dpi || {};

// すべての invoke 失敗を app.log へ記録してから再スローする（呼び出し側の try/catch で
// 握られたエラーも確実にログへ残すため）。pet_log 自身は再帰を避けて素の invoke を使う。
function logInvokeError(name, error) {
  const detail = error && (error.stack || error.message) ? error.stack || error.message : String(error);
  try {
    invoke("pet_log", { level: "error", message: `invoke ${name} 失敗`, detail: String(detail) }).catch(logError => {
      // フォールバック許可: pet_log 自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
      console.warn("invoke エラーログ保存に失敗しました", logError);
    });
  } catch (logError) {
    // フォールバック許可: pet_log 自体の失敗は再帰ログにせず、開発者コンソールへだけ出す。
    console.warn("invoke エラーログ保存に失敗しました", logError);
  }
}
async function call(name, args) {
  try {
    return await invoke(name, args);
  } catch (error) {
    logInvokeError(name, error);
    throw error;
  }
}

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
  // 共通VTuber起動画面から、保存中の表示形式と寸法へ戻す。
  finishBoot: () => call("pet_finish_boot"),
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
  setCharacterOffset: (x, y) => call("pet_set_character_offset", { x, y }),
  // キャラの上から Ctrl+ドラッグしたときなど、明示的にウィンドウを掴んで動かす。
  startWindowDrag: async () => {
    try { return await getCurrentWindow().startDragging(); }
    catch (error) { logInvokeError("startDragging", error); }
  },
  setBubbleSize: (width, height) => call("pet_set_bubble_size", { width, height }),
  setBubbleLayout: (width, height, left, top) => call("pet_set_bubble_layout", { width, height, left, top }),
  setCharacterProfile: (characterId, profile) => call("pet_set_character_profile", { characterId, profile }),
  relationshipState: () => call("pet_relationship_state"),
  relationshipEvent: (kind, detail = "") => call("pet_relationship_event", { kind, detail }),
  pinRelationshipMoment: (momentId, pinned) => call("pet_relationship_pin_moment", { momentId, pinned }),
  deleteRelationshipMoment: (momentId) => call("pet_relationship_delete_moment", { momentId }),
  clearRelationship: () => call("pet_relationship_clear"),
  completeOnboarding: (mode) => call("pet_complete_onboarding", { mode }),
  menu: () => call("pet_menu"),
  on: (channel, callback) => {
    if (!["pet:model", "pet:reset", "pet:layout-reset", "engine:status", "pet:ui", "pet:reload", "pet:emotion", "pet:agent_status", "pet:notice", "pet:hotkey", "pet:layout", "pet:character-reset"].includes(channel)) return;
    // Tauri のイベントは payload に値が入る。Electron の callback(value) 形に合わせる。
    listen(channel, (event) => callback(event.payload));
  },
  // Tauri イベントをリッスンして unlisten 関数を返す（ストリーミング等に使用）。
  listen: (channel, callback) => listen(channel, (event) => callback(event.payload)),
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
    listModels: (id) => call("settings_list_models", { id }),
    testTool: (name) => call("settings_test_tool", { name }),
    save: (patch) => call("settings_save", { patch }),
    // キャラクターパック（フォルダ / zip）の選択・導入・削除。
    userCharacters: () => call("pet_user_characters"),
    pickCharacterPack: (folder) => call("settings_character_pick", { folder }),
    installCharacter: (path) => call("settings_character_install", { path }),
    removeCharacter: (id) => call("settings_character_remove", { id }),
    tha3Status: (rootPath = null) => call("settings_tha3_status", { rootPath }),
    pickTha3Source: () => call("settings_tha3_pick_source"),
    pickTha3Root: () => call("settings_tha3_pick_root"),
    generateTha3Character: (sourcePath, tha3RootPath, id, name) => call("settings_tha3_generate", { sourcePath, tha3RootPath, id, name }),
    cancelTha3Generation: () => call("settings_tha3_cancel"),
  },
};
