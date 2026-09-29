// MMD 描画器（ui/character/mmd.js）とキャラ切替の契約。実物のモジュールを最小の DOM で動かす。
import assert from "node:assert/strict";
import test from "node:test";
import { installFakeDom } from "./lib/fake-dom.mjs";
import { uiFile } from "./lib/ui-source.mjs";

installFakeDom();
const { MmdRenderer } = await import("../../ui/character/mmd.js");
const main = uiFile("app/main.js");
const host = uiFile("character/index.js");

function controllerStub(calls) {
  return {
    setFraming: value => calls.push(["framing", value]),
    setMotionEnabled: value => calls.push(["motion", value]),
    setMouthLevel: value => calls.push(["mouth", value]),
    setPose: value => calls.push(["pose", value]),
    setExpression: value => calls.push(["expression", value]),
    setPixelRatio() {},
    dispose: () => calls.push(["dispose"]),
  };
}

test("キャラ切替は旧音声の合成待ちと再生を止めてから表示を変える", () => {
  const load = main.slice(main.indexOf("async function loadCharacter("));
  const stop = load.indexOf("voice.stop();");
  assert.ok(stop > load.indexOf("if (signatureOf(characterId) === loadedSignature) return null;"));
  assert.ok(stop < load.indexOf("host.load(characterId"));
});

test("MMD読込後は閉口・指定した元姿勢で始め、会話中・喜びの表情を渡す", async () => {
  const calls = [];
  let config;
  window.PicoMMD = {
    create: async (_canvas, options) => { config = options.config; return controllerStub(calls); },
    framingAspect: name => ({ full: 0.65, upper: 0.92, bust: 1.2 })[name],
  };
  const renderer = new MmdRenderer();
  await renderer.load({ root: "user-characters/demo", bodyProportion: "compact", mmd: { model: "mmd/demo.pmx", defaultPose: "original" } });
  assert.equal(config.bodyProportion, "compact");
  assert.equal(renderer.framingRect("bust").w, window.PicoMMD.framingAspect("bust"));
  assert.deepEqual(calls, [["motion", true], ["mouth", 0], ["pose", "original"]]);
  renderer.setFraming("upper");
  renderer.setMotion("talking");
  renderer.setMotion("pleased");
  assert.deepEqual(calls.slice(3), [["framing", "upper"], ["expression", "talking"], ["expression", "pleased"]]);
});

test("発話の終了・非表示・破棄でMMDの口を閉じる", async () => {
  const calls = [];
  window.PicoMMD = { create: async () => controllerStub(calls) };
  const renderer = new MmdRenderer();
  await renderer.load({ root: "r", mmd: { model: "m.pmx" } });
  calls.length = 0;
  renderer.setSpeaking(false);
  renderer.setActive(false);
  renderer.dispose();
  assert.deepEqual(calls.filter(([name]) => name === "mouth"), [["mouth", 0], ["mouth", 0], ["mouth", 0]]);
  assert.deepEqual(calls.at(-1), ["dispose"]);
});

test("MMD読込のAbortSignalを渡し、破棄したら直ちに中断する", async () => {
  let signal;
  window.PicoMMD = {
    create: (_canvas, options) => {
      signal = options.signal;
      return new Promise((_resolve, reject) => signal.addEventListener("abort", () => reject(new Error("中断"))));
    },
  };
  const renderer = new MmdRenderer();
  const pending = renderer.load({ root: "r", mmd: { model: "m.pmx" } });
  await new Promise(resolve => setTimeout(resolve, 5));
  renderer.dispose();
  await assert.rejects(pending, /中断/);
  assert.equal(signal.aborted, true);
});

test("遅れて読込完了した描画器を破棄し、新しいキャラへ適用しない", () => {
  // 読込の世代が変わっていたら、出来上がった描画器は捨てる（古いキャラが一瞬出ない）。
  const load = host.slice(host.indexOf("async load(characterId"));
  assert.match(load, /const loadGeneration = \+\+generation;/);
  assert.match(load, /if \(loadGeneration !== generation\) \{\s*\n\s*next\?\.dispose\(\);\s*\n\s*return null;/);
  // 新しい描画器の準備ができてから古い描画器を捨てる（入れ替えの間に空白を見せない）。
  assert.ok(load.indexOf("renderer?.dispose();") > load.indexOf("next = await createRenderer(Renderer, loaded);"));
});
