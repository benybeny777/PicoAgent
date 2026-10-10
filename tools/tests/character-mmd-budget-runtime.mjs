// Babylonの実描画境界を置換し、本番controllerの予算・停止・更新時刻を検査する。
import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { readFileSync } from "node:fs";
import { createFrameGate } from "../../ui/character/frame-budget.js";
import { installFakeDom } from "./lib/fake-dom.mjs";

const source = readFileSync(new URL("../../ui/mmd-renderer.js", import.meta.url), "utf8");
async function controllerFixture() {
  let now = 0;
  const counts = { draws: 0, starts: 0, stops: 0, updates: 0, previews: 0 };
  const callbacks = new Set(), previous = [];
  const rotation = {
    value: [0, 0, 0], clone() { return this; },
    multiply(value) { return value; }, copyFrom(value) { this.value = value.value; },
  };
  const bone = { name: "上半身", rotationQuaternion: rotation };
  class Vector3 {
    constructor(x, y, z) { Object.assign(this, { x, y, z }); }
    static Zero() { return new Vector3(0, 0, 0); }
    add(point) { return new Vector3(this.x + point.x, this.y + point.y, this.z + point.z); }
    scale(value) { return new Vector3(this.x * value, this.y * value, this.z * value); }
  }
  class Engine {
    runRenderLoop(callback) { counts.starts++; callbacks.add(callback); previous.push(callback); }
    stopRenderLoop(callback) { assert.equal(typeof callback, "function"); counts.stops++; callbacks.delete(callback); }
    setHardwareScalingLevel() {} resize() {} dispose() {}
  }
  class Scene { render() { counts.draws++; } dispose() {} }
  class Camera { constructor() { this.fov = 0.8; } setTarget() {} }
  const skeleton = { bones: [], computeAbsoluteMatrices() {} };
  const mesh = { metadata: { isMmdModel: true, skeleton, morphs: [] }, getVerticesData: () => [0, 0, 0, 0, 20, 0] };
  const container = { meshes: [mesh], addAllToScene() {}, dispose() {} };
  const window = { devicePixelRatio: 1, PicoMMDVendor: {
    Engine, Scene, Vector3, Color3: Vector3, Color4: Vector3,
    HemisphericLight: class {}, DirectionalLight: class {}, ArcRotateCamera: Camera,
    SdefInjector: { OverrideEngineCreateEffect() {} }, MmdStandardMaterialBuilder: class {},
    LoadAssetContainerAsync: async () => container,
    MmdRuntime: class { createMmdModel() { return {
      runtimeBones: [{ name: bone.name, linkedBone: bone }], morph: { setMorphWeight() {} },
      beforePhysics() { counts.updates++; }, afterPhysics() {},
    }; } },
    Quaternion: { FromEulerAngles: (...value) => ({ value }) },
  } };
  const document = { baseURI: "http://localhost/ui/index.html", hidden: false };
  const context = vm.createContext({ window, document, URL, DOMException,
    performance: { now: () => now }, ResizeObserver: class { observe() {} disconnect() {} },
  });
  vm.runInContext(source, context);
  const canvas = { clientWidth: 300, clientHeight: 400, toDataURL() { counts.previews++; return "data:image/png,test"; } };
  const controller = await window.PicoMMD.create(canvas, { modelUrl: "/model.pmx", frameGate: createFrameGate() });
  return { controller, counts, previous, rotation,
    tick(value) { now = value; for (const callback of callbacks) callback(); },
    advance(value) { now = value; },
  };
}

test("MMDは初期停止し144と240Hzでも予算30と60の描画数を守る", async () => {
  for (const refresh of [144, 240]) {
    for (const fps of [30, 60]) {
      const fixture = await controllerFixture();
      try {
        assert.equal(fixture.counts.starts, 0);
        fixture.tick(0);
        assert.equal(fixture.counts.draws, 0);
        fixture.controller.setFrameBudget(fps);
        for (let index = 0; index < refresh; index++) fixture.tick(index * 1000 / refresh);
        assert.ok(Math.abs(fixture.counts.draws - fps) <= 1, `${refresh}Hz/${fps}: ${fixture.counts.draws}`);
        assert.equal(fixture.counts.updates, fixture.counts.draws);
        fixture.controller.setFrameBudget(0);
        const stopped = fixture.counts.draws;
        fixture.tick(2000);
        assert.equal(fixture.counts.draws, stopped);
      } finally { fixture.controller.dispose(); }
    }
  }
});

test("MMDは同値で再登録せず停止復帰で同じcallbackと停止前の時間を守る", async () => {
  const fixture = await controllerFixture();
  try {
    fixture.controller.setMotionEnabled(true);
    fixture.controller.setFrameBudget(30);
    fixture.controller.setFrameBudget(30);
    fixture.tick(0);
    fixture.tick(100);
    const before = [...fixture.rotation.value];
    fixture.controller.setFrameBudget(0);
    fixture.controller.setFrameBudget(0);
    fixture.advance(100000);
    fixture.controller.setFrameBudget(60);
    fixture.tick(100000);
    assert.deepEqual([...fixture.rotation.value], before, "停止時間を呼吸へ持ち越しています");
    assert.equal(fixture.counts.starts, 2);
    assert.equal(fixture.counts.stops, 1);
    assert.equal(fixture.previous[0], fixture.previous[1]);
    fixture.controller.setFrameBudget(0);
    const stopped = fixture.counts.draws;
    fixture.controller.setPixelRatio(2);
    fixture.controller.setFraming("full");
    assert.equal(fixture.counts.draws, stopped, "停止canvasを暗黙更新しています");
    fixture.controller.capturePreview();
    assert.equal(fixture.counts.draws, stopped + 1);
    assert.equal(fixture.counts.previews, 1);
  } finally { fixture.controller.dispose(); }
  const finished = fixture.counts.draws;
  for (const callback of fixture.previous) callback();
  fixture.controller.setFrameBudget(60);
  assert.equal(fixture.counts.draws, finished, "破棄したcallbackが描画しました");
  assert.equal(fixture.counts.starts, 2);
  const active = await controllerFixture();
  active.controller.setFrameBudget(30);
  active.tick(0);
  active.controller.dispose();
  assert.equal(active.counts.stops, 1, "描画中の破棄でcallbackを除去していません");
  const disposedDraws = active.counts.draws;
  active.previous[0]();
  assert.equal(active.counts.draws, disposedDraws);
});

test("MMD wrapperは非表示中のreduced変更で再開せず共通gateを注入する", async () => {
  installFakeDom();
  const { MmdRenderer } = await import("../../ui/character/mmd.js");
  const budgets = [], motions = [];
  let options;
  window.PicoMMD = { create: async (_canvas, value) => {
    options = value;
    return { setFrameBudget: fps => budgets.push(fps), setMotionEnabled: value => motions.push(value),
      setMouthLevel() {}, setPose() {}, dispose() {} };
  } };
  const renderer = new MmdRenderer();
  try {
    await renderer.load({ root: "characters/demo", mmd: { model: "demo.pmx" } });
    assert.equal(typeof options.frameGate.allow, "function");
    assert.deepEqual(budgets, [0]);
    renderer.setFrameBudget(60);
    renderer.setActive(false);
    renderer.setReducedMotion(true);
    renderer.setReducedMotion(false);
    assert.deepEqual(budgets, [0, 60, 0]);
    assert.equal(motions.at(-1), false);
    renderer.setActive(true);
    assert.equal(budgets.at(-1), 60);
  } finally { renderer.dispose(); }
});
