import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const read = name => readFileSync(new URL(`../../${name}`, import.meta.url), "utf8");
const strip = source => source.replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/\bexport /g, "");

function fixture({ reducedMotion = false, springing = true } = {}) {
  let now = 0, sequence = 0;
  const frames = new Map(), timers = new Map(), drawn = [];
  class Image { constructor(name) { this.name = name; this.complete = true; this.naturalWidth = 2; this.naturalHeight = 2; } removeAttribute() {} }
  const context = { setTransform() {}, clearRect() {}, drawImage(image) { drawn.push({ now, name: image.name }); } };
  const canvas = { dataset: {}, style: {}, width: 2, height: 2, getContext: () => context, remove() {} };
  const sandbox = {
    Image, HTMLImageElement: Image, AbortController, queueMicrotask, Date, Math,
    performance: { now: () => now }, document: { hidden: false },
    setTimeout(callback, ms) { const id = ++sequence; timers.set(id, { callback, at: now + ms }); return id; },
    clearTimeout: id => timers.delete(id),
    requestAnimationFrame(callback) { const id = ++sequence; frames.set(id, callback); return id; },
    cancelAnimationFrame: id => frames.delete(id),
    el: () => canvas, packUrl: (_pack, path) => path,
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    fitTransform: () => ({ scale: 1, offsetX: 0, offsetY: 0 }),
    backingResolution: (_w, _h, ratio) => ratio,
  };
  sandbox.window = sandbox;
  const Renderer = runInNewContext(`${read("ui/frame-scheduler.js")}\n${strip(read("ui/character/frame-budget.js"))}\n${strip(read("ui/character/image.js"))}\nImageRenderer`, sandbox);
  const renderer = new Renderer({ reducedMotion });
  renderer.pack = { runtimeLayers: {} };
  renderer.animations = { idle: { framePaths: ["idle.png"] }, talking: { framePaths: ["idle.png"] } };
  renderer.layersAvailable = true;
  renderer.framingRects = { bust: {}, full: {} };
  renderer.modelWidth = 2; renderer.modelHeight = 2;
  if (springing) renderer.bodyMotion = { amplitudeY: 1, frequency: 0.2, phase: 0 };
  for (const name of ["base", "mouth_closed", "mouth_open", "blink_half", "blink_closed", "gaze_left", "gaze_right", "expression_happy"]) {
    renderer.images.set(`layers/${name}.png`, new Image(name));
  }
  renderer.images.set("idle.png", new Image("idle"));
  const tick = value => {
    now = value;
    const scheduled = [...frames.values()]; frames.clear();
    for (const callback of scheduled) callback(now);
  };
  return { renderer, frames, timers, drawn, tick };
}

for (const hz of [144, 240]) for (const fps of [0, 30, 60]) {
  test(`${hz}Hzと口形連続更新でも通常描画は${fps}fpsの共通上限を守る`, () => {
    const state = fixture();
    state.renderer.setFrameBudget(fps);
    for (let i = 0; i < hz; i += 1) {
      for (let burst = 0; burst < 20; burst += 1) state.renderer.setMouth(burst % 2);
      state.renderer.setMouth(1);
      state.tick(i * 1000 / hz);
    }
    assert.equal(state.drawn.length, fps);
    assert.ok(state.drawn.every(frame => frame.name === "mouth_open"), "そのフレームの最新口形だけ描く");
    state.renderer.dispose();
    assert.equal(state.frames.size, 0);
    assert.equal(state.timers.size, 0);
  });
}

test("非activeでは全通常要求と旧まばたき段階が描画せず復帰は経過をリセットする", () => {
  const state = fixture();
  state.renderer.setFrameBudget(30); state.tick(0);
  const oldBlink = [...state.timers.values()][0].callback;
  state.renderer.setActive(false);
  assert.equal(state.frames.size, 0); assert.equal(state.timers.size, 0);
  state.renderer.setMouth(1); state.renderer.setGaze(-1); state.renderer.setMotion("pleased");
  oldBlink(); state.tick(50000);
  assert.equal(state.drawn.length, 1); assert.equal(state.renderer.blinkPhase, null);
  state.renderer.setActive(true); oldBlink(); state.tick(50001);
  assert.equal(state.drawn.length, 2); assert.equal(state.renderer.springLastAt, 0);
  assert.equal(state.renderer.blinkPhase, null);
  state.renderer.dispose();
});

test("同値の上限とactive通知では再始動せず間引き時刻を消さない", () => {
  const state = fixture();
  state.renderer.setFrameBudget(30); state.tick(0);
  const pending = [...state.frames.keys()];
  state.renderer.setFrameBudget(30); state.renderer.setActive(true);
  assert.deepEqual([...state.frames.keys()], pending);
  state.tick(1);
  assert.equal(state.drawn.length, 1);
  state.tick(1000 / 30);
  assert.equal(state.drawn.length, 2);
  state.renderer.dispose();
});

test("減らす設定の待機は静止し発話の最新口形だけ上限内で反映する", () => {
  const state = fixture({ reducedMotion: true });
  state.renderer.setFrameBudget(30); state.tick(0);
  assert.equal(state.frames.size, 0); assert.equal(state.timers.size, 0);
  state.renderer.setSpeaking(true);
  state.renderer.setMouth(1); state.tick(1);
  assert.equal(state.drawn.length, 1);
  state.tick(34);
  assert.equal(state.drawn.at(-1).name, "mouth_open");
  state.renderer.setSpeaking(false); state.tick(67);
  assert.equal(state.drawn.at(-1).name, "mouth_closed");
  assert.equal(state.timers.size, 0);
  state.renderer.dispose();
});

test("遅れた睡眠画像と破棄前のタイマーは停止や新状態を上書きしない", async () => {
  const state = fixture();
  let resolve;
  state.renderer.preload = () => new Promise(done => { resolve = done; });
  state.renderer.setFrameBudget(60);
  state.renderer.setSleeping("old.png");
  state.renderer.setActive(false); state.renderer.setActive(true);
  resolve({ name: "old" }); await Promise.resolve();
  assert.equal(state.renderer.sleepingImage, null);
  const oldBlink = [...state.timers.values()][0].callback;
  state.renderer.dispose(); oldBlink();
  state.renderer.setMouth(1); state.renderer.setGaze(1); state.renderer.setActive(true);
  state.renderer.setFrameBudget(60); state.renderer.draw(); state.tick(1000);
  assert.equal(state.drawn.length, 0); assert.equal(state.frames.size, 0); assert.equal(state.timers.size, 0);
});


test("表情と視線とまばたき段階も同一フレームへまとめ明示resizeだけは停止中に一度描ける", () => {
  const state = fixture({ springing: false });
  state.renderer.gazeAvailable = true; state.renderer.happyAvailable = true;
  state.renderer.setFrameBudget(30); state.tick(0);
  const blink = [...state.timers.values()][0].callback;
  state.renderer.setMotion("pleased"); state.renderer.setGaze(-1); blink();
  state.tick(1);
  assert.equal(state.drawn.length, 1);
  state.tick(34);
  assert.equal(state.drawn.length, 2);
  assert.equal(state.drawn.at(-1).name, "blink_half");
  state.renderer.setActive(false);
  state.renderer.setFraming("full"); state.renderer.setMouth(1); state.tick(70);
  assert.equal(state.drawn.length, 2);
  state.renderer.resize(2, 2, 1);
  assert.equal(state.drawn.length, 3);
  state.renderer.dispose(); state.renderer.resize(2, 2, 1);
  assert.equal(state.drawn.length, 3);
});


test("減らす設定で口が閉じたまま発話終了しても閉眼を残さず静止顔へ戻る", () => {
  const state = fixture({ reducedMotion: true, springing: false });
  state.renderer.setFrameBudget(30); state.tick(0);
  state.renderer.setSpeaking(true);
  const fireTimer = () => {
    const [id, timer] = [...state.timers][0];
    state.timers.delete(id); timer.callback();
  };
  fireTimer(); state.tick(34);
  fireTimer(); state.tick(68);
  assert.equal(state.drawn.at(-1).name, "blink_closed");
  assert.equal(state.renderer.mouthOpen, false);
  state.renderer.setSpeaking(false); state.tick(102);
  assert.equal(state.drawn.at(-1).name, "mouth_closed");
  assert.equal(state.frames.size, 0); assert.equal(state.timers.size, 0);
  state.renderer.dispose();
});
