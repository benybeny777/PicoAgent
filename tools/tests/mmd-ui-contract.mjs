import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const source = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const start = source.match(/async function startMmd\(config, generation\) \{[\s\S]*?\n\}/)[0];

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
    currentLayout: "widget", reduceMotion: false, mmdController: null, mmdCanvas: null,
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
