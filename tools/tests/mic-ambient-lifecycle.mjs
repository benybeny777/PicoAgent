import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

function target() {
  const handlers = new Map();
  return {
    handlers, disabled: false,
    addEventListener(name, callback) { handlers.set(name, callback); },
    setAttribute() {}, classList: { toggle() {} },
  };
}

function environment(file, factory, extra = {}) {
  const timers = new Map();
  let id = 0;
  const schedule = callback => { timers.set(++id, callback); return id; };
  const context = vm.createContext({
    document: { ...target(), hidden: false }, window: target(),
    setTimeout: schedule, setInterval: schedule,
    clearTimeout: value => timers.delete(value), clearInterval: value => timers.delete(value),
    performance: { now: () => 0 }, console,
    toast() {}, logApp() {}, clearBackgroundFailure() {}, notifyBackgroundFailure() {},
    errorDetail: String, preview: String, ...extra,
  });
  vm.runInContext(uiFile(file).replace(/^import .*;\r?$/gm, "").replace("export function", "function") + `\nthis.factory = ${factory};`, context);
  return { context, timers, factory: context.factory };
}

for (const event of ["pointerup", "pointercancel"]) {
  test(`マイク許可待ちの長押しは${event}で取り消して遅延取得のトラックを停止する`, async () => {
    let resolveMedia;
    let stopped = 0;
    const env = environment("app/mic.js", "createMic", {
      navigator: { mediaDevices: { getUserMedia: () => new Promise(resolve => { resolveMedia = resolve; }) } },
    });
    const micButton = target();
    const mic = env.factory({
      chat: { buttons: { micButton, voiceLoopButton: target() },
        composer: { value: "", focus() {} }, status: { clear() {} } },
      voice: { stop() {} }, motion: { rest() {} }, isBusy: () => false, canListen: () => true,
    });
    micButton.handlers.get("pointerdown")({ button: 0 });
    const begin = [...env.timers.values()][0]();
    await micButton.handlers.get(event)();
    resolveMedia({ getTracks: () => [{ stop() { stopped += 1; } }] });
    await begin;
    assert.equal(stopped, 1);
    assert.equal(mic.active, false);
    assert.equal(micButton.disabled, false);
  });
}

test("録音開始後の長押し取消は取得済みのトラックと音声処理を解放する", async () => {
  let stopped = 0;
  let closed = 0;
  const env = environment("app/mic.js", "createMic", {
    navigator: { mediaDevices: { getUserMedia: async () => ({ getTracks: () => [{ stop() { stopped += 1; } }] }) } },
  });
  env.context.window.AudioContext = class {
    sampleRate = 16000;
    createMediaStreamSource() { return { connect() {} }; }
    createScriptProcessor() { return { connect() {}, disconnect() {} }; }
    close() { closed += 1; return Promise.resolve(); }
  };
  const micButton = target();
  const mic = env.factory({
    chat: { buttons: { micButton, voiceLoopButton: target() },
      composer: { value: "", focus() {} }, status: { clear() {}, set() {} } },
    voice: { stop() {}, speaking: false }, motion: { rest() {}, react() {} },
    isBusy: () => false, canListen: () => true,
  });
  micButton.handlers.get("pointerdown")({ button: 0 });
  await [...env.timers.values()][0]();
  assert.equal(mic.active, true);
  micButton.handlers.get("pointercancel")();
  assert.equal(mic.active, false);
  assert.equal(stopped, 1);
  assert.equal(closed, 1);
});

test("同時に期限が来た全リマインダーを一つの通知として表示と音声と履歴へ渡す", async () => {
  const displayed = [], prepared = [], history = [];
  const env = environment("app/ambient.js", "createAmbient", {
    bridge: { dailyEvents: async () => ({ events: [{ text: "会議" }, { text: "薬" }] }), alarms: async () => ({ alarms: [] }) },
  });
  const ambient = env.factory({
    chat: { status: {}, composer: { isOpen: false }, history: { add: (_role, text) => history.push(text) },
      showAnswer: text => displayed.push(text) },
    voice: { speaking: false, prepare: async text => { prepared.push(text); return null; } },
    host: { sleeping: false }, motion: {}, isBusy: () => false, isExpanded: () => false,
  });
  ambient.start({ napMinutes: 0 });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(displayed, ["会議\n薬"]);
  assert.deepEqual(prepared, displayed);
  assert.deepEqual(history, displayed);
});

test("通知の音声準備中に入力を開いても待機通知を失わず次の機会に表示する", async () => {
  let resolveSpeech;
  let first = true;
  const displayed = [];
  const composer = { isOpen: true };
  const env = environment("app/ambient.js", "createAmbient");
  const ambient = env.factory({
    chat: { composer, status: { clear() {} }, history: { add() {} }, showAnswer: text => displayed.push(text) },
    voice: { speaking: false, prepare: () => first ? new Promise(resolve => { resolveSpeech = resolve; first = false; }) : Promise.resolve(null) },
    host: { sleeping: false }, motion: {}, isBusy: () => false, isExpanded: () => false,
  });
  await ambient.say("必ず知らせるリマインダー");
  composer.isOpen = false;
  const flushing = ambient.flushNotice();
  composer.isOpen = true;
  resolveSpeech(null);
  await flushing;
  assert.deepEqual(displayed, []);
  composer.isOpen = false;
  await ambient.flushNotice();
  assert.deepEqual(displayed, ["必ず知らせるリマインダー"]);
});

test("キャラ切替前に生成を始めた雑談は遅れて届いても新キャラの画面へ出さない", async () => {
  let resolveChatter;
  const displayed = [];
  const env = environment("app/ambient.js", "createAmbient", {
    bridge: { idleChatter: () => new Promise(resolve => { resolveChatter = resolve; }) },
  });
  const ambient = env.factory({
    chat: { composer: { isOpen: false }, status: { clear() {} }, history: { add() {} }, showAnswer: text => displayed.push(text) },
    voice: { speaking: false, prepare: async () => null }, host: { sleeping: false }, motion: {},
    isBusy: () => false, isExpanded: () => false,
  });
  ambient.applyChatter({ chatterEnabled: true, chatterMinSecs: 30, chatterMaxSecs: 30 });
  const old = [...env.timers.values()][0]();
  ambient.reset();
  resolveChatter({ enabled: true, reply: "旧キャラの雑談" });
  await old;
  assert.deepEqual(displayed, []);
});
