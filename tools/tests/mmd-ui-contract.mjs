import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const source = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
test("キャラ切替は旧音声の合成待ちと再生を止めてから表示を変える", () => {
  const load = source.slice(source.indexOf('async function loadCharacter('), source.indexOf('function setAnimation('));
  const stop = load.indexOf('stopVoice(false)');
  assert.ok(stop > load.indexOf('signature === loadedCharacterSignature'));
  assert.ok(stop < load.indexOf('characterManifest = manifest'));
});
test("フレーム画像がないMMDでも会話中と喜びの状態を保つ", () => {
  const animation = source.match(/function setAnimation\(name, force = false\) \{[\s\S]*?\n\}/)[0];
  const calls = [];
  const context = vm.createContext({
    animations: {}, animationName: "idle", live2dActive: false, frame: 0, lastFrameAt: 0,
    performance: { now: () => 1 }, character: { dataset: {} }, wakeAnimationLoop() {},
    mmdController: { setExpression: name => calls.push(name) },
  });
  vm.runInContext(animation, context);
  context.setAnimation("talking");
  assert.equal(context.animationName, "talking");
  context.setAnimation("pleased");
  assert.equal(context.animationName, "pleased");
  assert.deepEqual(calls, ["talking", "pleased"]);
});
const start = source.match(/async function startMmd\(config, generation\) \{[\s\S]*?\n\}/)[0];
test("MMD小型アイコンに通常立ち絵の下方向オフセットを重ねない", () => {
  const framing = source.match(/function applyCharacterFraming\(\) \{[\s\S]*?\n\}/)[0];
  const values = {};
  const context = vm.createContext({
    characterManifest: { id: "demo" }, characterFraming: {}, currentLayout: "chat",
    petApp: { classList: { contains: () => false } },
    characterFramingLayout: () => context.currentLayout,
    character: { dataset: { renderKind: "mmd" } },
    FRAMING_PRESETS: { upper: { zoom: 1.55, x: 0, y: 18 }, bust: { zoom: 2.05, x: 0, y: 38 } },
    applyCharacterZoom: value => { values.zoom = value; },
    applyCharacterOffset: (x, y) => { values.x = x; values.y = y; },
  });
  vm.runInContext(framing, context);
  for (const layout of ["chat", "bar"]) {
    context.currentLayout = layout; context.applyCharacterFraming();
    assert.deepEqual(values, { zoom: 1.55, x: 0, y: 0 });
  }
  context.characterFraming = { demo: { bar: { zoom: 2, x: 4, y: 5 } } };
  context.applyCharacterFraming();
  assert.deepEqual(values, { zoom: 2, x: 4, y: 5 });
});

function harness() {
  let finish;
  let removed = 0;
  let disposed = 0;
  const calls = [];
  const controller = {
    setFraming: value => calls.push(["framing", value]),
    setMotionEnabled: value => calls.push(["motion", value]),
    setMouthLevel: value => calls.push(["mouth", value]),
    setPose: value => calls.push(["pose", value]),
    dispose: () => disposed++,
  };
  const context = vm.createContext({
    AbortController,
    document: { hidden: false, createElement: () => ({ setAttribute() {}, remove: () => removed++ }) },
    characterView: { insertBefore() {} }, petSprite: { style: {} },
    character: { dataset: {} }, characterRoot: "user-characters/demo", characterLoadGeneration: 1,
    currentLayout: "widget", characterFramingLayout: () => "widget", reduceMotion: false, mmdController: null, mmdCanvas: null,
    mmdLoadAbortController: null,
    mmdPoseRow: {}, mmdPoseSelect: {}, retryPendingLipSync() {}, destroyMmd() {},
    loadScriptOnce: async () => {},
    window: { PicoMMD: { create: () => new Promise(resolve => { finish = () => resolve(controller); }) } },
  });
  vm.runInContext(start, context);
  return { context, calls, finish: () => finish(), removed: () => removed, disposed: () => disposed };
}

test("遅れて読込完了したMMDを破棄し、新しいキャラへ適用しない", async () => {
  const h = harness();
  const pending = h.context.startMmd({ model: "mmd/demo.pmx" }, 1);
  await new Promise(setImmediate);
  h.context.characterLoadGeneration = 2;
  h.finish();
  await pending;
  assert.equal(h.disposed(), 1);
  assert.equal(h.removed(), 1);
  assert.equal(h.context.mmdController, null);
  assert.equal(h.calls.length, 0);
});

test("MMD読込後は現在のlayout、閉口、指定した元姿勢を適用する", async () => {
  const h = harness();
  const pending = h.context.startMmd({ model: "mmd/demo.pmx", defaultPose: "original" }, 1);
  await new Promise(setImmediate);
  h.finish();
  await pending;
  assert.deepEqual(h.calls, [["framing", "widget"], ["motion", true], ["mouth", 0], ["pose", "original"]]);
});

test("音声終了・ミュート・キャラ切替がMMD口パクも閉じる", () => {
  assert.match(source, /function stopLipSync\(\) \{\s*if \(mmdController\) \{ detachLipSyncAudio\(\); return; \}/);
  for (const name of ["detachLipSyncAudio", "applyVoiceVolume", "destroyMmd"]) {
    const body = source.match(new RegExp(`function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n\\}`))?.[0];
    assert.match(body, /mmdController\?\.setMouthLevel\(0\)/, name);
  }
});

test("MMD読込のAbortSignalを保持し、破棄時に直ちに中断する", async () => {
  const h = harness();
  let capturedSignal;
  h.context.window.PicoMMD.create = async (_canvas, options) => {
    capturedSignal = options.signal;
    return new Promise((_resolve, reject) => capturedSignal.addEventListener("abort", () => reject(new Error("中断"))));
  };
  const pending = h.context.startMmd({ model: "mmd/demo.pmx" }, 1);
  await new Promise(setImmediate);
  assert.equal(capturedSignal, h.context.mmdLoadAbortController.signal);
  h.context.characterLoadGeneration = 2;
  h.context.mmdLoadAbortController.abort();
  await pending;
  assert.equal(capturedSignal.aborted, true);
  assert.equal(h.context.mmdLoadAbortController, null);
  assert.match(source, /function destroyMmd\(\) \{\s*mmdLoadAbortController\?\.abort\(\)/);
});
