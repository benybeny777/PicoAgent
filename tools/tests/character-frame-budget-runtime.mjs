// 共通方針・実ホストの状態遷移・描画回数の境界を固定する。
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { frameBudget, createFrameGate } from "../../ui/character/frame-budget.js";

test("停止を最優先し待機と動作と動きを減らす状態で上限を決める", () => {
  for (const speaking of [false, true]) for (const dragging of [false, true])
    for (const motion of ["idle", "talking"]) for (const reducedMotion of [false, true]) {
      const state = { speaking, dragging, motion, reducedMotion };
      assert.equal(frameBudget({ ...state, active: false }), 0);
      assert.equal(frameBudget({ ...state, active: true }), reducedMotion ? 30 : speaking || dragging || motion !== "idle" ? 60 : 30);
    }
});

test("境界の浮動誤差でも144と240Hzで描画上限を超えず停止後は再開できる", () => {
  for (const hz of [144, 240]) for (const fps of [0, 30, 60]) {
    const gate = createFrameGate(); gate.setBudget(fps);
    let draws = 0;
    for (let tick = 1; tick <= hz; tick++) draws += Number(gate.allow(tick * 1000 / hz));
    assert.equal(draws, fps, `${hz}/${fps}`);
    gate.setBudget(0); assert.equal(gate.allow(10_000), false);
    gate.setBudget(30); assert.equal(gate.allow(20_000), true);
    assert.equal(gate.setBudget(30), false);
    assert.equal(gate.allow(20_001), false);
  }
});

function fixture({ reducedMotion = false, loadRenderer = async () => {} } = {}) {
  const listeners = new Map();
  const media = { matches: reducedMotion, addEventListener(_name, callback) { this.change = callback; } };
  const document = { hidden: false, addEventListener(name, callback) { listeners.set(name, callback); } };
  class Renderer {
    static kind = "2d";
    static supports() { return true; }
    constructor(options) { this.element = { style: {}, setAttribute() {}, removeAttribute() {} }; this.history = []; this.reducedMotion = options.reducedMotion; }
    async load(pack) { await loadRenderer(pack); }
    setLayoutId() {} setFraming() {} setMotion() {} setSpeaking() {}
    setReducedMotion(value) { this.reducedMotion = value; }
    setActive(value) { this.active = value; }
    setFrameBudget(fps) { this.fps = fps; this.history.push(fps); }
    dispose() { this.disposed = true; }
  }
  class Unsupported { static supports() { return false; } }
  const source = readFileSync(new URL("../../ui/character/index.js", import.meta.url), "utf8")
    .replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/export (const|function|class)/g, "$1");
  const factory = runInNewContext(`${source}\ncreateCharacterHost`, {
    frameBudget, ImageRenderer: Renderer, MmdRenderer: Unsupported, MeshRenderer: Unsupported, Live2dRenderer: Unsupported,
    document, window: { matchMedia: () => media }, bridge: {},
    el: () => ({}), createEmitter: () => ({ emit() {} }), logApp() {}, errorMessage: String, errorDetail: String,
    loadPack: async id => ({ id, manifest: { boot: { sleeping: "sleep.png" } } }), personaText: () => "",
    packUrl: (_pack, path) => path, clearTimeout, setTimeout,
  });
  return { host: factory({ container: { append() {}, prepend() {}, dataset: {} } }), document, listeners, media };
}

test("実ホストは準備非表示居眠りを優先し読替え後と復帰時に同じ状態を再同期する", async () => {
  const { host, document, listeners, media } = fixture();
  host.setDisplayReady(false); host.setSpeaking(true); await host.load("first");
  assert.equal(host.renderer.fps, 0);
  host.setDisplayReady(true); assert.equal(host.renderer.fps, 60);
  document.hidden = true; listeners.get("visibilitychange")(); assert.equal(host.renderer.fps, 0);
  document.hidden = false; listeners.get("visibilitychange")(); assert.equal(host.renderer.fps, 60);
  host.setSpeaking(false); assert.equal(host.renderer.fps, 30);
  host.setSleeping(true); assert.equal(host.renderer.fps, 0);
  const old = host.renderer; await host.load("second");
  assert.equal(old.disposed, true); assert.equal(host.sleeping, false); assert.equal(host.renderer.fps, 30);
  host.setMotion("thinking"); assert.equal(host.renderer.fps, 60);
  media.change({ matches: true }); assert.equal(host.renderer.fps, 30);
  host.setActive(false); media.change({ matches: false }); assert.equal(host.renderer.fps, 0);
});

test("重なる通常ドラッグの完了と失敗は最後に待機へ戻り編集ドラッグ状態を消さない", async () => {
  const { host } = fixture(); await host.load("first");
  let release;
  const first = host.withDragging(() => new Promise(resolve => { release = resolve; }));
  assert.equal(host.renderer.fps, 60);
  await assert.rejects(host.withDragging(async () => { throw new Error("移動拒否"); }), /移動拒否/);
  assert.equal(host.renderer.fps, 60);
  host.setDragging(true); release(); await first; assert.equal(host.renderer.fps, 60);
  host.setDragging(false); assert.equal(host.renderer.fps, 30);
});

for (const initial of [false, true]) test(`読込中に動きを減らす設定を${initial ? "解除" : "有効化"}しても新描画器へ最新状態を渡す`, async () => {
  let release, started;
  const loading = new Promise(resolve => { started = resolve; });
  const pending = new Promise(resolve => { release = resolve; });
  const { host, media } = fixture({ reducedMotion: initial, loadRenderer: pack => {
    if (pack.id === "next") { started(); return pending; }
  } });
  await host.load("first");
  host.setMotion("thinking");
  const next = host.load("next");
  await loading;
  media.change({ matches: !initial });
  assert.equal(host.renderer.reducedMotion, !initial, "表示中の旧描画器も更新する");
  release(); await next;
  assert.equal(host.renderer.reducedMotion, !initial, "読込開始時の設定を持ち越さない");
  assert.equal(host.renderer.fps, initial ? 60 : 30);
});

test("全描画器は共通予算を受け取り上限表を複写しない", () => {
  for (const name of ["image", "live2d", "mesh", "mmd"]) {
    const source = readFileSync(new URL(`../../ui/character/${name}.js`, import.meta.url), "utf8");
    assert.match(source, /setFrameBudget\(fps\)/, name);
    assert.doesNotMatch(source, /1000\s*\/\s*(?:30|60)|maxFPS\s*=\s*(?:0|30|60)/, name);
  }
  const host = readFileSync(new URL("../../ui/character/index.js", import.meta.url), "utf8");
  assert.equal((host.match(/active && displayReady && !sleeping && !document\.hidden/g) || []).length, 1);
});

test("描画回数のログは明示した開発確認だけで動き恒久タイマーを持たない", () => {
  let now = 0;
  const root = { performance: { now: () => now } };
  runInNewContext(readFileSync(new URL("../../ui/frame-scheduler.js", import.meta.url), "utf8"), { window: root });
  const reports = [];
  root.PicoFrameScheduler.setFrameReporter((...args) => reports.push(args));
  for (let frame = 0; frame <= 60; frame++) { now = frame * 1000 / 60; root.PicoFrameScheduler.recordCharacterFrame("検査"); }
  assert.equal(reports.length, 0);
  root.PicoCharacterFrameDebug = true;
  for (let frame = 0; frame <= 60; frame++) { now = 2000 + frame * 1000 / 60; root.PicoFrameScheduler.recordCharacterFrame("検査"); }
  assert.equal(reports.length, 1); assert.equal(reports[0][0], "検査");
  root.PicoCharacterFrameDebug = false; now = 4000; root.PicoFrameScheduler.recordCharacterFrame("検査");
  assert.equal(reports.length, 1);
});
