// 実描画器と共通ticker制御を評価し、高刷新率でも描画上限と停止を守る。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";

const read = path => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");
const moduleSource = path => read(path).replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/export (async function|function|class|const)/g, "$1");
const point = () => ({ set() {} });

function fixture() {
  let now = 0;
  const apps = [], timers = new Map();
  let nextTimer = 0, resolveModel;
  const modelPending = new Promise(resolve => { resolveModel = resolve; });
  const callbacks = new Map(), blinkDeltas = [], modelDeltas = [];
  const motionStarts = [];
  const motionManager = {
    groups: { idle: "DefaultIdle" }, playing: null,
    stopAllMotions() { this.playing = null; },
    // 同梱CubismのMotionManagerは停止後の更新でもidleグループを自動要求する。
    update() { if (!this.playing && this.groups.idle) model.motion(this.groups.idle); },
  };
  const model = {
    update(delta) { modelDeltas.push(delta); motionManager.update(); },
    anchor: point(), scale: point(), position: point(),
    motion(group) { motionManager.playing = group; motionStarts.push(group); },
    destroy() { this.destroyed = true; },
    internalModel: {
      originalWidth: 100, originalHeight: 200, settings: {},
      coreModel: { setParameterValueById() {} }, motionManager,
      eyeBlink: { updateParameters(core, delta) { blinkDeltas.push(delta); } },
      on(name, callback) { callbacks.set(name, callback); },
    },
  };
  class Ticker {
    constructor() { this.started = false; this.starts = 0; this.stops = 0; this.listeners = new Map(); this.rate = 60; this.lastTime = now; }
    set maxFPS(value) { assert.ok(value > 0, "停止を無制限fpsへ変えない"); this.rate = value; }
    get maxFPS() { return this.rate; }
    start() { this.starts++; this.started = true; }
    stop() { this.stops++; this.started = false; }
    add(callback, owner, priority = 0) { this.listeners.set(callback, priority); }
    remove(callback) { this.listeners.delete(callback); }
    update(time) {
      if (!this.started || time - this.lastTime + 1e-7 < 1000 / this.rate) return;
      // PIXIのmaxFPSと同じく端数を次回へ持ち越す。
      this.lastTime += Math.floor((time - this.lastTime + 1e-7) / (1000 / this.rate)) * (1000 / this.rate);
      for (const [callback] of [...this.listeners].sort((a, b) => b[1] - a[1])) callback();
    }
  }
  class Application {
    constructor(options) {
      this.options = options; this.ticker = new Ticker(); this.draws = 0;
      this.screen = { width: options.width, height: options.height };
      this.stage = { pivot: point(), scale: point(), position: point(), addChild() {}, removeChild() {} };
      this.renderer = { resolution: 1, background: {}, resize() {} };
      this.ticker.add(() => { this.render(); callbacks.get("afterMotionUpdate")?.(); });
      if (options.autoStart !== false) this.ticker.start();
      apps.push(this);
    }
    render() { this.draws++; }
    destroy() { this.destroyed = true; this.ticker.stop(); }
  }
  const canvas = () => ({ style: {}, dataset: {}, getContext: () => ({}), remove() { this.removed = true; } });
  const window = { performance: { now: () => now }, devicePixelRatio: 1,
    PIXI: { Application, SimplePlane: class {}, UPDATE_PRIORITY: { HIGH: 25 }, live2d: {
      Live2DModel: { from(url, options) { model.autoUpdate = options.autoUpdate !== false; return modelPending; } },
    } } };
  const context = { window, performance: window.performance, console, Uint8Array, Float32Array,
    setTimeout(callback) { const id = ++nextTimer; timers.set(id, callback); return id; },
    clearTimeout: id => timers.delete(id), el: canvas, errorDetail: String, errorMessage: String, logApp() {},
    packUrl: (pack, path) => `${pack.root}/${path}`, backingResolution: () => 1,
    defaultFramingFor: () => "full", fitTransform: () => ({ scale: 1, offsetX: 0, offsetY: 0 }),
    framingRectsFor: content => ({ full: content, bust: content }), fetchJson: async () => ({ canvas: [100, 200] }),
    sleep: ms => new Promise(resolve => context.setTimeout(resolve, ms)), loadScriptOnce: async () => {}, PIXI: window.PIXI,
    fetch: async () => ({ ok: true, json: async () => ({ version: 2, mode: "anime25d-v1", canvas: [100, 200], layers: [] }) }),
  };
  const api = runInNewContext(`${read("ui/frame-scheduler.js")}
    ${moduleSource("ui/character/runtime.js")}
    ${moduleSource("ui/character/live2d.js")}
    ${moduleSource("ui/character/mesh.js")}
    ${read("ui/mesh-rig.js").replace("Object.freeze({ create })", "Object.freeze({ create, MeshRigController })")}
    ({ Live2dRenderer, MeshRenderer, MeshRigController: window.PicoMeshRig.MeshRigController })`, context);
  return { ...api, apps, window, canvas, model, blinkDeltas, modelDeltas, callbacks, timers, motionStarts, motionManager,
    resolveModel: () => resolveModel(model),
    setNow(value) { now = value; },
    async fireTimer() {
      const [id, callback] = timers.entries().next().value;
      timers.delete(id); callback();
      await Promise.resolve(); await Promise.resolve();
    },
    advance(app, hz, seconds = 1) {
      const start = now;
      for (let index = 1; index <= hz * seconds; index++) { now = start + index * 1000 / hz; if (model.autoUpdate) model.update(1000 / hz); app.ticker.update(now); }
    },
    mesh() {
      const app = new Application({ width: 100, height: 200, autoStart: false });
      const controller = new api.MeshRigController(app, canvas(), {}, new Map());
      const deltas = [];
      for (const name of ["updateBlink", "updateLvsAppearance", "updateAlphas", "updateBlush"]) controller[name] = () => {};
      controller.updateMesh = (time, delta) => deltas.push({ time, delta });
      return { renderer: controller, app, deltas };
    },
  };
}

async function live(state, { reducedMotion = false, config = {} } = {}) {
  const renderer = new state.Live2dRenderer({ reducedMotion });
  const loading = renderer.load({ root: "test", live2dSdk: "4", live2d: { model: "model.json", ...config } });
  await Promise.resolve(); await Promise.resolve();
  const app = state.apps[0];
  assert.equal(app.options.autoStart, false);
  state.advance(app, 240);
  assert.equal(app.draws, 0, "モデル取得途中は描画しない");
  state.resolveModel(); await loading;
  return { renderer, app };
}

for (const kind of ["Live2D", "mesh"]) for (const hz of [144, 240]) {
  test(`${kind}は${hz}Hzで30/60fpsを守り停止・復帰・破棄後を制御する`, async () => {
    const state = fixture();
    const { renderer, app } = kind === "Live2D" ? await live(state) : state.mesh();
    renderer.setActive(true);
    state.advance(app, hz);
    assert.equal(app.draws, 0, "初期予算0ではactiveだけで再開しない");
    for (const fps of [30, 60]) {
      renderer.setFrameBudget(fps);
      const starts = app.ticker.starts;
      renderer.setFrameBudget(fps); renderer.setActive(true);
      assert.equal(app.ticker.starts, starts, "同値で再起動しない");
      const before = app.draws; state.advance(app, hz);
      assert.ok(app.draws - before >= fps - 1 && app.draws - before <= fps + 1, `${fps}fpsの描画上限`);
    }
    assert.equal(app.ticker.starts, 1, "30から60への変更では再起動しない");
    renderer.setActive(false); const stopped = app.draws;
    renderer.setReducedMotion(true); renderer.setFrameBudget(30);
    state.advance(app, hz, 10);
    assert.equal(app.draws, stopped); assert.notEqual(renderer.canvas.style.display, "none", "停止でcanvasを消さない");
    renderer.setActive(true); state.advance(app, hz);
    if (kind === "Live2D") assert.ok(state.blinkDeltas.at(-1) < 0.1);
    renderer.setFrameBudget(0); state.advance(app, hz);
    const atZero = app.draws; state.advance(app, hz);
    assert.equal(app.draws, atZero);
    if (kind === "Live2D") renderer.dispose(); else renderer.destroy();
    const starts = app.ticker.starts;
    renderer.setActive(true); renderer.setFrameBudget(60); state.advance(app, hz);
    assert.equal(app.draws, atZero); assert.equal(app.ticker.starts, starts);
  });
}

test("meshは復帰後の物理時間を持ち越さず停止中resizeだけ一度描く", () => {
  const state = fixture(), { renderer, app, deltas } = state.mesh();
  renderer.setActive(true); renderer.setFrameBudget(30); state.advance(app, 240);
  renderer.setActive(false); state.advance(app, 240, 10);
  const before = app.draws; renderer.layout(100, 200, 1);
  assert.equal(app.draws, before + 1);
  const resumeIndex = deltas.length;
  renderer.setActive(true); state.advance(app, 240);
  assert.ok(deltas.every(({ delta }) => delta <= 1 / 30));
  assert.equal(deltas[resumeIndex]?.delta, 1 / 60, "復帰初回のdtは停止時間を含めない");
  const callback = renderer.tick; renderer.destroy(); const count = deltas.length;
  callback(); assert.equal(deltas.length, count, "破棄済みcallbackは計算をしない");
});

test("MeshRendererは中央予算をcontrollerへ渡し停止中のまばたき予約を増やさない", async () => {
  const state = fixture(), { renderer: controller, app } = state.mesh();
  state.window.PicoMeshRig = { create: async () => controller };
  const renderer = new state.MeshRenderer();
  await renderer.load({ root: "test", meshRig: { rig: "rig.json" } });
  renderer.setActive(true); assert.equal(state.timers.size, 0);
  renderer.setFrameBudget(30); assert.equal(state.timers.size, 1);
  renderer.setFrameBudget(30); assert.equal(state.timers.size, 1);
  state.advance(app, 240); assert.ok(app.draws <= 31);
  renderer.setActive(false); renderer.setReducedMotion(true); renderer.setFrameBudget(60);
  const before = app.draws; state.advance(app, 240); assert.equal(app.draws, before);
  renderer.dispose(); assert.equal(state.timers.size, 0);
});

for (const pause of ["active", "budget"]) for (const phase of [0, 1, 2, 3]) {
  test(`メッシュのまばたき段階${phase}を${pause}で停止すると開眼へ戻り古い段階を再開しない`, async () => {
    const state = fixture(), { renderer: controller, app } = state.mesh();
    state.window.PicoMeshRig = { create: async () => controller };
    const renderer = new state.MeshRenderer();
    await renderer.load({ root: "test", meshRig: { rig: "rig.json" } });
    renderer.setActive(true); renderer.setFrameBudget(30);
    for (let index = 0; index < phase; index++) await state.fireTimer();
    const stale = [...state.timers.values()][0];
    if (pause === "active") renderer.setActive(false); else renderer.setFrameBudget(0);
    const draws = app.draws;
    assert.equal(state.timers.size, 0, "停止時に段階待機も回収する");
    assert.equal(controller.eyeOpen, 1, "閉眼と補間を復帰へ持ち越さない");
    assert.equal(controller.blinkTween, null);
    state.advance(app, 240, 10); assert.equal(app.draws, draws);
    if (pause === "active") renderer.setActive(true); else renderer.setFrameBudget(30);
    const fresh = [...state.timers.keys()];
    stale(); await Promise.resolve(); await Promise.resolve();
    assert.equal(controller.eyeOpen, 1);
    assert.equal(controller.blinkTween, null, "停止前の閉眼処理が新しい待機へ干渉しない");
    assert.deepEqual([...state.timers.keys()], fresh);
    const beforeDispose = [...state.timers.values()][0];
    renderer.dispose(); beforeDispose();
    assert.equal(state.timers.size, 0, "破棄後は古い段階から予約を復活させない");
  });
}

for (const configured of [false, true]) for (const reducedMotion of [false, true]) {
  test(`Live2Dの待機グループ${configured ? "指定あり" : "既定"}は動きを減らす初期値${reducedMotion}から切替と復帰を守る`, async () => {
    const state = fixture();
    const idle = configured ? "ConfiguredIdle" : "DefaultIdle";
    const config = configured ? { motions: { idle } } : {};
    const { renderer, app } = await live(state, { reducedMotion, config });
    renderer.setActive(true); renderer.setFrameBudget(30);
    const check = reduced => {
      renderer.setReducedMotion(reduced);
      const count = state.motionStarts.length;
      state.motionManager.playing = null;
      state.advance(app, 240);
      assert.equal(state.motionManager.groups.idle, reduced ? "" : idle);
      assert.equal(state.motionStarts.length, count + Number(!reduced), "更新時の自動待機再生も設定に従う");
      if (!reduced) assert.equal(state.motionStarts.at(-1), idle);
    };
    check(reducedMotion); check(!reducedMotion); check(reducedMotion);
    renderer.dispose();
  });
}

test("常時アニメーションを指定したLive2DはOS設定変更で待機を失わない", async () => {
  const state = fixture(), { renderer, app } = await live(state, { reducedMotion: true, config: { alwaysAnimate: true } });
  renderer.setActive(true); renderer.setFrameBudget(30);
  for (const reduced of [false, true]) {
    renderer.setReducedMotion(reduced); state.motionManager.playing = null;
    state.advance(app, 240);
    assert.equal(state.motionManager.groups.idle, "DefaultIdle");
    assert.equal(state.motionManager.playing, "DefaultIdle");
  }
  renderer.dispose();
});

test("Live2Dの古い読込が破棄後に完了しても描画を再開しない", async () => {
  const state = fixture(), renderer = new state.Live2dRenderer();
  const loading = renderer.load({ root: "test", live2dSdk: "4", live2d: { model: "model.json" } });
  await Promise.resolve(); await Promise.resolve();
  renderer.dispose(); state.resolveModel(); await loading;
  assert.equal(state.model.destroyed, true);
  assert.equal(renderer.model, null);
  assert.equal(state.apps[0].ticker.started, false);
});


test("Live2Dは読込途中に予算が届いても完成前に開始しない", async () => {
  const state = fixture(), renderer = new state.Live2dRenderer();
  const loading = renderer.load({ root: "test", live2dSdk: "4", live2d: { model: "model.json" } });
  await Promise.resolve(); await Promise.resolve();
  renderer.setActive(true); renderer.setFrameBudget(60);
  state.advance(state.apps[0], 240);
  assert.equal(state.apps[0].draws, 0);
  state.resolveModel(); await loading;
  state.advance(state.apps[0], 240);
  assert.ok(state.apps[0].draws >= 59);
  renderer.dispose();
});

test("実mesh生成は自動tickerを止めたままcontrollerへ渡す", async () => {
  const state = fixture();
  const controller = await state.window.PicoMeshRig.create({ canvas: state.canvas(), rigUrl: "rig.json", rootUrl: ".", width: 100, height: 200 });
  const app = state.apps[0];
  assert.equal(app.options.autoStart, false);
  state.advance(app, 240);
  assert.equal(app.draws, 0);
  controller.destroy();
});


test("Live2Dはshared tickerへ登録せずapp描画予算でモデルも更新する", async () => {
  const state = fixture(), { renderer, app } = await live(state);
  assert.equal(state.model.autoUpdate, false, "fromへautoUpdate:falseを明示する");
  renderer.setActive(true); renderer.setFrameBudget(30);
  state.advance(app, 240);
  assert.ok(state.modelDeltas.length >= 29 && state.modelDeltas.length <= 31);
  renderer.setFrameBudget(0); const count = state.modelDeltas.length;
  state.advance(app, 240, 10);
  assert.equal(state.modelDeltas.length, count, "app停止中はsharedの更新も0");
  renderer.setFrameBudget(60); state.advance(app, 240);
  assert.ok(state.modelDeltas.length - count >= 59 && state.modelDeltas.length - count <= 61);
  assert.ok(state.modelDeltas.slice(count).every(delta => delta < 20), "復帰は停止期間をモデルへ渡さない");
  const oldUpdate = renderer.updateModel;
  renderer.dispose(); oldUpdate(); state.advance(app, 240);
  assert.equal(state.modelDeltas.length, count + 60);
});
