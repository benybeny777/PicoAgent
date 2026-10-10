// キャラ読込と人格登録の完了順を逆転させ、古い切替が現在の表示を汚さないことを確かめる。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const main = readFileSync(new URL("../../ui/app/main.js", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const loadFunction = main.slice(main.indexOf("async function loadCharacter("), main.indexOf("\nfunction localStorageVariant("));
const hostSource = readFileSync(new URL("../../ui/character/index.js", import.meta.url), "utf8")
  .replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/export (function|const|class)/g, "$1");

function setup(initialSignature = "") {
  const loads = [], profiles = [], greetings = [], appearances = [], diagnostics = [];
  let diagnostic = null;
  const element = { style: {}, dataset: {} };
  let statusText = "";
  const context = {
    signatureOf: id => id, voice: { stop() {}, setCharacter() {}, speak: text => greetings.push(text) },
    host: { setDisplayReady() {}, load: id => { const work = deferred(); loads.push({ id, ...work }); return work.promise; } },
    characterSource: { element }, stage: { setCharacter() {} }, chrome: { setCharacterName() {} },
    status: { set: text => { statusText = text; }, clear: () => { statusText = ""; } },
    renderCharacterOptions() {}, motion: { rest() {} }, personaText: () => "人格",
    characterOverrides: {}, sdkOverrides: {}, rendererOverrides: {}, errorMessage: String,
    characterDiagnostic: { show(id, error) { diagnostic = { id, error }; diagnostics.push(diagnostic); }, clear() { diagnostic = null; } },
    bridge: { setCharacterProfile: (id, profile, greeting, voice) => { const work = deferred(); profiles.push({ id, voice, ...work }); return work.promise; } },
    applyStreamAppearance: value => appearances.push(value), chat: { renderReply() {} },
  };
  const api = runInNewContext(`let loadedSignature = ${JSON.stringify(initialSignature)};
    let characterName = ""; let requestedCharacterId = ""; let characterLoadGeneration = 0; let characterLoads = 0;
    ${loadFunction}
    ({ loadCharacter, get signature() { return loadedSignature; } })`, context);
  return { ...api, api, loads, profiles, greetings, appearances, element, get statusText() { return statusText; },
    diagnostics, get diagnostic() { return diagnostic; },
    async finishLoad(index) {
      const id = loads[index].id;
      loads[index].resolve({ pack: { id, name: id, manifest: { voice: { provider: "aivis", styleId: "123" } } }, greeting: `${id}の挨拶` });
      await Promise.resolve(); await Promise.resolve();
    },
  };
}

test("古い人格登録が後から完了しても新キャラの背景と挨拶を上書きしない", async () => {
  const state = setup();
  const first = state.loadCharacter("a"); await state.finishLoad(0);
  const second = state.loadCharacter("b"); await state.finishLoad(1);
  state.profiles[1].resolve("bの背景"); await second;
  state.profiles[0].resolve("aの背景"); assert.equal(await first, null);
  assert.deepEqual(state.appearances, ["bの背景"]);
  assert.deepEqual(state.greetings, ["bの挨拶"]);
  assert.equal(state.api.signature, "b");
  assert.equal(state.element.style.visibility, "");
});

test("新キャラへ移った後の古い読込失敗は現在の表示を隠さない", async () => {
  const state = setup();
  const first = state.loadCharacter("a");
  const second = state.loadCharacter("b"); await state.finishLoad(1);
  state.profiles[0].resolve("bの背景"); await second;
  state.loads[0].reject(new Error("古い読込の失敗"));
  assert.equal(await first, null);
  assert.equal(state.element.style.visibility, "");
  assert.equal(state.statusText, "");
  assert.equal(state.diagnostics.length, 0);
});

test("切替待ち中に元キャラへ戻した場合は署名一致でも読込を省略しない", async () => {
  const state = setup("a");
  const first = state.loadCharacter("b");
  const second = state.loadCharacter("a");
  assert.deepEqual(state.loads.map(work => work.id), ["b", "a"]);
  await state.finishLoad(1); state.profiles[0].resolve("aの背景"); await second;
  await state.finishLoad(0); assert.equal(await first, null);
  assert.equal(state.api.signature, "a");
});

test("人格登録失敗は未確定キャラを隠して診断を残し同じキャラを再試行できる", async () => {
  const state = setup();
  const first = state.loadCharacter("a"); await state.finishLoad(0);
  state.profiles[0].reject(new Error("人格登録失敗"));
  await assert.rejects(first, /人格登録失敗/);
  assert.equal(state.element.style.visibility, "hidden");
  assert.match(state.statusText, /人格登録失敗/);
  assert.equal(state.diagnostic.id, "a");
  const second = state.loadCharacter("a"); await state.finishLoad(1);
  state.profiles[1].resolve("aの背景"); await second;
  assert.equal(state.element.style.visibility, "");
  assert.equal(state.statusText, "");
  assert.equal(state.diagnostic, null);
});

test("読込途中の描画器は寸法と状態を確定するまで可視にしない", async () => {
  const work = deferred();
  const instances = [];
  class Renderer {
    static supports() { return true; }
    constructor() { this.element = { style: {} }; instances.push(this); }
    load() { return work.promise; }
    setFraming() { assert.equal(this.element.style.visibility, "hidden"); }
    setMotion() {} setSpeaking() {} setActive(value) { this.active = value; }
    dispose() { this.disposed = true; }
  }
  const unsupported = class { static supports() { return false; } };
  const factory = runInNewContext(`${hostSource}\ncreateCharacterHost`, {
    ImageRenderer: Renderer, MmdRenderer: unsupported, Live2dRenderer: unsupported, MeshRenderer: unsupported,
    window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
    document: { hidden: false, addEventListener() {} },
    el: () => ({}), createEmitter: () => ({ emit() {} }), errorMessage: String, errorDetail: String, logApp() {},
    loadPack: async id => ({ id, manifest: {} }), personaText: () => "", bridge: {},
  });
  const container = { dataset: {}, append() {}, prepend() {} };
  const host = factory({ container });
  host.setDisplayReady(false);
  const loading = host.load("a"); await new Promise(setImmediate);
  assert.equal(instances[0].element.style.visibility, "hidden");
  work.resolve(); await loading;
  assert.equal(instances[0].element.style.visibility, "");
  assert.equal(instances[0].active, false, "未確定キャラの描画は停止する");
  host.setActive(true);
  assert.equal(instances[0].active, false, "配置の可視更新で未確定描画を再開しない");
  host.setDisplayReady(true);
  assert.equal(instances[0].active, true, "人格登録が確定した後に描画を再開する");
  host.setDisplayReady(false);
  assert.equal(instances[0].active, false, "失敗して隠した後も描画を止める");
});

function expandedSetup() {
  const calls = [];
  const messages = [];
  const stage = { expanded: false, setExpanded: on => { stage.expanded = on; } };
  const button = { textContent: "拡大" };
  const queueAt = main.indexOf("let fullscreenQueue =");
  const body = main.slice(queueAt >= 0 ? queueAt : main.indexOf("async function setExpanded("), main.indexOf("// ── キーボード"));
  const setExpanded = runInNewContext(`${body}\nsetExpanded`, {
    stage, buttons: { expandButton: button }, status: { flash: text => messages.push(text) },
    bridge: { setFullscreen: on => { const work = deferred(); calls.push({ on, ...work }); return work.promise; } },
    errorMessage: String, ambient: { flushNotice() {} }, setTimeout: callback => callback(),
  });
  return { setExpanded, stage, button, calls, messages };
}

test("全画面連打はネイティブ操作を直列化し古い失敗で最新状態を戻さない", async () => {
  const state = expandedSetup();
  const first = state.setExpanded(true); await new Promise(setImmediate);
  const second = state.setExpanded(false);
  assert.equal(state.calls.length, 1);
  state.calls[0].reject(new Error("最初の切替失敗")); await first; await new Promise(setImmediate);
  assert.deepEqual(state.calls.map(call => call.on), [true, false]);
  assert.equal(state.stage.expanded, false); assert.equal(state.messages.length, 0);
  state.calls[1].resolve(); await second;
  assert.equal(state.button.textContent, "拡大");
});

test("まだ送信していない全画面連打の途中操作は省略する", async () => {
  const state = expandedSetup();
  const first = state.setExpanded(true);
  const second = state.setExpanded(false);
  const third = state.setExpanded(true);
  await new Promise(setImmediate);
  assert.deepEqual(state.calls.map(call => call.on), [true]);
  state.calls[0].resolve(); await Promise.all([first, second, third]);
  assert.equal(state.stage.expanded, true); assert.equal(state.button.textContent, "戻す");
});

test("全画面連打が両方失敗したら未実行の楽観表示ではなく確定済み通常表示へ戻す", async () => {
  const state = expandedSetup();
  const first = state.setExpanded(true); await new Promise(setImmediate);
  const second = state.setExpanded(false);
  state.calls[0].reject(new Error("拡大失敗")); await first; await new Promise(setImmediate);
  state.calls[1].reject(new Error("復帰失敗")); await second;
  assert.equal(state.stage.expanded, false);
  assert.equal(state.button.textContent, "拡大");
  assert.equal(state.messages.length, 1);
});

test("ネイティブドロップ読込中は最後のファイルの失敗を含め送信抑止を保持して解除する", async () => {
  let pending = 0;
  const reads = [];
  const added = [];
  const body = main.slice(main.indexOf("async function handleNativeDrop("), main.indexOf("\nfor (const [channel, callback]"));
  const handleNativeDrop = runInNewContext(`${body}\nhandleNativeDrop`, {
    ambient: { markActivity() {} }, motion: { react() {} }, toast() {}, logApp() {}, errorMessage: String, errorDetail: String,
    composer: { beginAttachmentLoad: () => { pending++; return () => pending--; },
      addLoaded: info => { added.push(info); return ""; }, open() {} },
    bridge: { readDroppedPath: path => { const work = deferred(); reads.push({ path, ...work }); return work.promise; } },
  });
  const loading = handleNativeDrop(["a", "b"]);
  assert.equal(pending, 1);
  reads[0].resolve("aの添付"); await new Promise(setImmediate);
  assert.equal(pending, 1); assert.equal(reads.length, 2);
  reads[1].reject(new Error("読込失敗")); await loading;
  assert.equal(pending, 0); assert.deepEqual(added, ["aの添付"]);
});

test("拡大中に表示形式を切り替える時はシーンだけでなく窓の全画面も解除する", () => {
  const calls = [];
  const stage = { layoutId: "pet", expanded: true, setLayout: layout => calls.push(["layout", layout]) };
  const body = main.slice(main.indexOf("function applyLayout("), main.indexOf("function showStartupReady("));
  const applyLayout = runInNewContext(`${body}\napplyLayout`, {
    stage, LAYOUT_IDS: ["pet", "mascot", "stream"], app: { dataset: {} }, editor: { render() {} },
    setExpanded: on => calls.push(["expanded", on]),
  });
  applyLayout("stream");
  assert.deepEqual(calls, [["expanded", false], ["layout", "stream"]]);
});

 test("人格登録は読み込めたmanifestの声を第4引数へ渡す", async () => {
  const state = setup();
  const work = state.loadCharacter("a"); await state.finishLoad(0);
  assert.equal(state.profiles[0].voice?.styleId, "123");
  state.profiles[0].resolve({}); await work;
});
