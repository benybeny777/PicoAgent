// picoagent.js - 旧 Electron preload（window.picoagent）を Tauri 上で再現する互換シム。
// tauri.conf.json の app.withGlobalTauri=true により window.__TAURI__ が使える前提。
// これにより renderer.js / settings.js は Electron 版とほぼ同じコードで動く。

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;
const { LogicalSize, PhysicalPosition } = window.__TAURI__.dpi || {};

// すべての invoke 失敗を app.log へ記録してから再スローする（呼び出し側の try/catch で
// 握られたエラーも確実にログへ残すため）。pet_log 自身は再帰を避けて素の invoke を使う。
function logInvokeError(name, error) {
  const detail = error && (error.stack || error.message) ? error.stack || error.message : String(error);
  try { invoke("pet_log", { level: "error", message: `invoke ${name} 失敗`, detail: String(detail) }).catch(() => {}); }
  catch {}
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
  idleChatter: () => call("pet_idle_chatter"),
  dailyGreeting: () => call("pet_daily_greeting"),
  dailyEvents: () => call("pet_daily_events"),
  alarms: () => call("pet_alarms"),
  alarmWake: (id) => call("pet_alarm_wake", { id }),
  speak: (text) => call("pet_speak", { text }),
  prepareVoiceModel: () => call("pet_prepare_voice_model"),
  log: (level, message, detail) => invoke("pet_log", { level, message, detail }),
  aivisStatus: () => call("aivis_status"),
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
  setBubbleSize: (width, height) => call("pet_set_bubble_size", { width, height }),
  setBubbleLayout: (width, height, left, top) => call("pet_set_bubble_layout", { width, height, left, top }),
  setCharacterProfile: (characterId, profile) => call("pet_set_character_profile", { characterId, profile }),
  menu: () => call("pet_menu"),
  on: (channel, callback) => {
    if (!["pet:model", "pet:reset", "pet:layout-reset", "engine:status", "pet:ui", "pet:reload", "pet:emotion"].includes(channel)) return;
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
  engineStatus: () => call("engine_status"),
  restartApp: (restartEngine, restartAivis) => call("restart_app", { restartEngine: !!restartEngine, restartAivis: !!restartAivis }),
  settings: {
    load: () => call("settings_load"),
    listModels: (id) => call("settings_list_models", { id }),
    save: (patch) => call("settings_save", { patch }),
  },
};
