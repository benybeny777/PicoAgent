// picoagent.js - 旧 Electron preload（window.picoagent）を Tauri 上で再現する互換シム。
// tauri.conf.json の app.withGlobalTauri=true により window.__TAURI__ が使える前提。
// これにより renderer.js / settings.js は Electron 版とほぼ同じコードで動く。

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

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
  setBubbleSize: (width, height) => call("pet_set_bubble_size", { width, height }),
  setCharacterProfile: (characterId, profile) => call("pet_set_character_profile", { characterId, profile }),
  menu: () => call("pet_menu"),
  on: (channel, callback) => {
    if (!["pet:model", "pet:reset", "engine:status", "pet:ui", "pet:reload"].includes(channel)) return;
    // Tauri のイベントは payload に値が入る。Electron の callback(value) 形に合わせる。
    listen(channel, (event) => callback(event.payload));
  },
  engineStatus: () => call("engine_status"),
  restartApp: (restartEngine, restartAivis) => call("restart_app", { restartEngine: !!restartEngine, restartAivis: !!restartAivis }),
  settings: {
    load: () => call("settings_load"),
    listModels: (id) => call("settings_list_models", { id }),
    save: (patch) => call("settings_save", { patch }),
  },
};
