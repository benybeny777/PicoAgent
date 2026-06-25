// picoagent.js - 旧 Electron preload（window.picoagent）を Tauri 上で再現する互換シム。
// tauri.conf.json の app.withGlobalTauri=true により window.__TAURI__ が使える前提。
// これにより renderer.js / settings.js は Electron 版とほぼ同じコードで動く。

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

window.picoagent = {
  init: () => invoke("pet_init"),
  send: (text) => invoke("pet_send", { text }),
  speak: (text) => invoke("pet_speak", { text }),
  aivisStatus: () => invoke("aivis_status"),
  // ウィンドウをフルスクリーン化／解除する（フキダシ全画面表示で使用）。
  setFullscreen: (on) => getCurrentWindow().setFullscreen(on),
  setBubbleSize: (width, height) => invoke("pet_set_bubble_size", { width, height }),
  setCharacterProfile: (characterId, profile) => invoke("pet_set_character_profile", { characterId, profile }),
  menu: () => invoke("pet_menu"),
  on: (channel, callback) => {
    if (!["pet:model", "pet:reset", "engine:status", "pet:ui"].includes(channel)) return;
    // Tauri のイベントは payload に値が入る。Electron の callback(value) 形に合わせる。
    listen(channel, (event) => callback(event.payload));
  },
  engineStatus: () => invoke("engine_status"),
  settings: {
    load: () => invoke("settings_load"),
    listModels: (id) => invoke("settings_list_models", { id }),
    save: (patch) => invoke("settings_save", { patch }),
  },
};
