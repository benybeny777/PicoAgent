import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const game = readFileSync(new URL("../../ui/game.js", import.meta.url), "utf8");
const functions = ["characterFramingLayout", "applyCharacterFraming", "applyMeshRigFraming", "applyLive2DFraming", "applyAllCharacterFraming"]
  .map(name => renderer.match(new RegExp(`function ${name}\\(\\) \\{[\\s\\S]*?\\n\\}`))?.[0]);
assert.ok(functions.every(Boolean), "構図の実処理が見つかりません");
const gameActions = game.slice(game.indexOf("  function open()"), game.indexOf('  window.addEventListener("beforeunload"'));
const registration = renderer.match(/window\.addEventListener\("pico:game-visibility", applyAllCharacterFraming\);/)?.[0];
assert.ok(registration, "ゲーム表示変更の購読がありません");

function harness(layout, kind = "mesh-rig") {
  const classes = new Set();
  const listeners = new Map();
  const result = {};
  const petApp = { classList: { contains: name => classes.has(name), add: name => classes.add(name), remove: name => classes.delete(name) } };
  const context = {
    petApp, currentLayout: layout,
    character: { dataset: { renderKind: kind, sourceFraming: "native-full", bodyProportion: "standard" } },
    characterManifest: { id: "demo", streamFraming: { zoom: 1, x: 0, y: 0 } },
    characterFraming: { demo: { [layout]: { zoom: 4, x: 500, y: -400 }, pet: { zoom: 3, x: 350, y: 280 } } },
    characterZoom: 4,
    animationName: "idle", setAnimation() {}, setMeshRigVisible() {},
    FRAMING_PRESETS: { upper: { zoom: 1.55, x: 0, y: 18 }, bust: { zoom: 2.05, x: 0, y: 38 }, full: { zoom: 1, x: 0, y: 0 } },
    applyCharacterZoom(value) { result.zoom = value; context.characterZoom = value; },
    applyCharacterOffset(x, y) { result.x = x; result.y = y; },
    meshRigController: { setFraming: value => { result.mesh = value; } },
    mmdController: { setFraming: value => { result.mmd = value; } },
    activeLive2dConfig: { scale: 2, offsetY: 20, iconFraming: { scale: 7, offsetY: 600 }, layoutFraming: { stream: { scale: 5 }, pet: { scale: 2.5 } } },
    live2dModel: { internalModel: { originalWidth: 285, originalHeight: 315 }, scale: { set: value => { result.live2d = value; } }, anchor: { set() {} }, position: { set(x, y) { result.live2dY = y; } } },
    LIVE2D_RENDER_WIDTH: 285, LIVE2D_RENDER_HEIGHT: 315,
    window: { addEventListener: (name, callback) => listeners.set(name, callback), dispatchEvent: event => listeners.get(event.type)?.(event), confirm: () => true },
    document: { getElementById: () => petApp }, Event: class { constructor(type) { this.type = type; } },
    overlay: { hidden: true }, session: null, renderBriefing() {}, closeButton: { focus() {} }, clearTimer() {}, releaseAudioContext() {}, setStatus() {},
  };
  runInNewContext(`${functions.join("\n")}\n${registration}\n${gameActions}`, context);
  return { context, result };
}

test("ゲームは全表示形式からモデル既定の胸上構図へ切り替え、保存値を変更しない", () => {
  for (const layout of ["pet", "mascot", "chat", "bar", "stream", "workspace", "widget"]) {
    const { context, result } = harness(layout);
    const saved = JSON.stringify(context.characterFraming);
    context.open();
    assert.deepEqual([result.zoom, result.x, result.y, result.mesh, result.live2d, result.mmd], [2.05, 0, 38, 1, 2.5, "pet"], layout);
    assert.equal(context.overlay.hidden, false);
    assert.equal(JSON.stringify(context.characterFraming), saved);
    context.close();
    assert.equal(context.overlay.hidden, true);
    assert.equal(result.zoom, layout === "pet" ? 3 : 4);
    assert.equal(result.mmd, layout);
    assert.equal(JSON.stringify(context.characterFraming), saved);
  }
});

test("ゲーム中のキャラ・形式変更でも胸上を維持し、閉じると最新の保存構図を使う", () => {
  const { context, result } = harness("stream");
  context.open();
  context.currentLayout = "bar";
  context.characterManifest = { id: "next", defaultFraming: { zoom: 2.2, x: -12, y: 8 } };
  context.characterFraming.next = { bar: { zoom: 1.8, x: 7, y: 9 } };
  context.applyAllCharacterFraming();
  assert.deepEqual([result.zoom, result.x, result.y, result.mesh, result.live2d, result.mmd], [2.2, -12, 8, 1, 2.5, "pet"]);
  context.close();
  assert.deepEqual([result.zoom, result.x, result.y, result.live2d, result.mmd], [1.8, 7, 9, 7, "bar"]);
  // レイアウト更新・非同期キャラ読込完了も同じ入口を通る。
  assert.match(renderer, /if \(loadGeneration !== characterLoadGeneration\) return;\s+applyAllCharacterFraming\(\);\s+frame = 0;/);
  assert.match(renderer, /syncWidgetClock\(\);\s+applyCharacterOffset\([^;]+;\s+applyAllCharacterFraming\(\);/);
  assert.match(renderer, /controller\.setFraming\(characterFramingLayout\(\)\);/);
});

test("Live2DとMMDはゲーム中にアイコン構図と保存倍率を重ねない", () => {
  for (const kind of ["live2d", "mmd"]) {
    const { context, result } = harness("chat", kind);
    context.open();
    assert.deepEqual([result.zoom, result.x, result.y, result.live2d, result.live2dY, result.mmd], [1.55, 0, 18, 2.5, 335, "pet"]);
    context.close();
    assert.equal(result.live2d, 7);
    assert.equal(result.mmd, "chat");
  }
});

test("chatとbarのメッシュはゲーム中だけ再開し、非表示・起動中・就寝中は描画しない", () => {
  const canRender = renderer.match(/function meshRigCanRender\(\) \{[\s\S]*?\n\}/)?.[0];
  assert.ok(canRender);
  for (const layout of ["chat", "bar"]) {
    const { context } = harness(layout);
    context.napping = false;
    context.document.hidden = false;
    runInNewContext(canRender, context);
    assert.equal(context.meshRigCanRender(), false);
    context.open();
    assert.equal(context.meshRigCanRender(), true);
    context.document.hidden = true;
    assert.equal(context.meshRigCanRender(), false);
    context.document.hidden = false;
    context.napping = true;
    assert.equal(context.meshRigCanRender(), false);
    context.napping = false;
    context.petApp.classList.add("is-booting");
    assert.equal(context.meshRigCanRender(), false);
    context.petApp.classList.remove("is-booting");
    context.close();
    assert.equal(context.meshRigCanRender(), false);
  }
});

test("ゲーム中断を取り消した場合はゲーム構図を維持する", () => {
  const { context, result } = harness("chat");
  context.open();
  context.session = { phase: "answer" };
  context.window.confirm = () => false;
  context.close();
  assert.equal(context.overlay.hidden, false);
  assert.equal(result.mmd, "pet");
  assert.equal(result.zoom, 2.05);
});
