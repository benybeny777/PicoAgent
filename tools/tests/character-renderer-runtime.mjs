// 描画器の実ホストを故障注入し、方式跨ぎ・古い世代・明示2Dの診断境界を固定する。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
import { frameBudget, createFrameGate } from "../../ui/character/frame-budget.js";

const read = path => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");
const strip = source => source.replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/export (function|const|class)/g, "$1");
const hostSource = strip(read("ui/character/index.js"));

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function hostSetup({ loadPack = async id => ({ id, manifest: {}, modes: ["mmd", "live2d", "mesh-rig", "2d"] }), loads = {}, imageRenderer } = {}) {
  const instances = [], attempts = [], events = [], logs = [], gpu = [];
  function renderer(kind) {
    return class {
      static kind = kind;
      static supports(pack) { return pack.modes.includes(kind); }
      constructor() { this.kind = kind; this.element = { style: {} }; instances.push(this); }
      async load(pack) { attempts.push([kind, pack.id]); await loads[kind]?.(pack); }
      setFraming() {} setMotion() {} setSpeaking() {} setActive() {} setFrameBudget() {}
      dispose() { this.disposed = true; }
    };
  }
  const factory = runInNewContext(`${hostSource}\ncreateCharacterHost`, {
    frameBudget,
    MmdRenderer: renderer("mmd"), Live2dRenderer: renderer("live2d"), MeshRenderer: renderer("mesh-rig"),
    ImageRenderer: imageRenderer || renderer("2d"), loadPack, personaText: () => "人格",
    window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
    document: { hidden: false, addEventListener() {} },
    createEmitter: () => ({ emit: (...args) => events.push(args) }), el: () => ({}),
    errorMessage: error => error.message || String(error), errorDetail: String, logApp: (...args) => logs.push(args),
    bridge: { requestGpuCacheReset: reason => { gpu.push(reason); return Promise.resolve(); } },
  });
  const host = factory({ container: { dataset: {}, append() {}, prepend() {} } });
  return { host, instances, attempts, events, logs, gpu };
}

test("MMD・Live2D・メッシュの最優先方式が失敗しても下位方式を試さない", async () => {
  for (const [kind, modes] of [["mmd", ["mmd", "live2d", "mesh-rig", "2d"]], ["live2d", ["live2d", "mesh-rig", "2d"]], ["mesh-rig", ["mesh-rig", "2d"]]]) {
    const cause = new Error(`${kind}の準備失敗`);
    const state = hostSetup({ loadPack: async id => ({ id, manifest: {}, modes }), loads: { [kind]: () => { throw cause; } } });
    await assert.rejects(state.host.load("a"), error => {
      assert.equal(error.name, "CharacterRenderError"); assert.equal(error.kind, kind);
      assert.equal(error.cause, cause); assert.equal(error.pack.id, "a"); assert.equal(error.imageAvailable, true);
      return true;
    });
    assert.deepEqual(state.attempts, [[kind, "a"]]);
    assert.equal(state.instances[0].disposed, true);
    assert.equal(state.host.renderer, null);
    assert.equal(state.events.some(([event]) => event === "fallback"), false);
  }
});

test("明示2Dは画像構造がある場合だけ使い設定を外すと通常の最優先方式へ戻る", async () => {
  const state = hostSetup();
  await state.host.load("a", { rendererOverrides: { a: "image" } });
  assert.equal(state.host.kind, "2d");
  await state.host.load("a");
  assert.equal(state.host.kind, "mmd");
  assert.equal(state.instances[0].disposed, true);
  assert.deepEqual(state.attempts, [["2d", "a"], ["mmd", "a"]]);
  const noImage = hostSetup({ loadPack: async id => ({ id, manifest: {}, modes: ["live2d"] }) });
  await assert.rejects(noImage.host.load("a", { rendererOverrides: { a: "image" } }), /2D表示用の画像がありません/);
  assert.equal(noImage.attempts.length, 0);
  assert.equal(noImage.host.renderer, null);
});

test("パック読込失敗は確定パックを持たず2D選択を許可しない", async () => {
  const state = hostSetup({ loadPack: async () => { throw new Error("追加キャラ一覧の取得失敗"); } });
  await assert.rejects(state.host.load("a"), error => error.kind === "pack" && error.pack === null && error.imageAvailable === false);
  assert.equal(state.attempts.length, 0);
});

test("古い世代の描画失敗は新表示とGPU診断を汚さず失敗描画器を破棄する", async () => {
  const work = deferred();
  const state = hostSetup({ loads: { mmd: pack => pack.id === "old" ? work.promise : undefined } });
  const old = state.host.load("old"); await new Promise(setImmediate);
  await state.host.load("new");
  work.reject(new Error("WebGL context lost"));
  assert.equal(await old, null);
  assert.equal(state.host.pack.id, "new");
  assert.equal(state.instances[0].disposed, true);
  assert.equal(state.instances[1].disposed, undefined);
  assert.equal(state.gpu.length, 0);
  assert.equal(state.logs.length, 0);
});

test("古い世代のパック失敗と描画成功を破棄して新表示を保持する", async () => {
  const packWork = deferred();
  const state = hostSetup({ loadPack: id => id === "old" ? packWork.promise : Promise.resolve({ id, manifest: {}, modes: ["mmd"] }) });
  const old = state.host.load("old"); await state.host.load("new");
  packWork.reject(new Error("古いパックの欠損"));
  assert.equal(await old, null); assert.equal(state.host.pack.id, "new");
  const rendererWork = deferred();
  const success = hostSetup({ loads: { mmd: pack => pack.id === "old" ? rendererWork.promise : undefined } });
  const oldSuccess = success.host.load("old"); await new Promise(setImmediate);
  await success.host.load("new"); rendererWork.resolve();
  assert.equal(await oldSuccess, null); assert.equal(success.host.pack.id, "new");
  assert.equal(success.instances[0].disposed, true);
});

test("実2D描画器は生成preview画像が欠損したら診断へ戻り別方式へ進まない", async () => {
  const canvas = { style: {}, getContext: () => ({}), remove() { this.removed = true; } };
  class MissingImage {
    constructor() { this.naturalWidth = 0; }
    set src(value) { queueMicrotask(() => this.onerror?.()); }
    removeAttribute() {}
  }
  const ImageRenderer = runInNewContext(`${strip(read("ui/character/image.js"))}\nImageRenderer`, {
    createFrameGate,
    el: () => canvas, window: { PicoFrameScheduler: { createLoop: () => ({ stop() {} }) } },
    AbortController, Image: MissingImage, setTimeout, clearTimeout, queueMicrotask,
    packUrl: (pack, path) => `${pack.root}/${path}`,
  });
  const state = hostSetup({ imageRenderer: ImageRenderer, loadPack: async id => ({
    id, root: "missing", manifest: {}, meshRig: { rig: "rig.json" }, modes: ["mesh-rig"],
    animations: { idle: { framePaths: ["preview.png"], frames: 1, fps: 1 } },
  }) });
  await assert.rejects(state.host.load("a", { rendererOverrides: { a: "image" } }), error => {
    assert.equal(error.kind, "2d"); assert.equal(error.imageAvailable, false);
    assert.match(error.message, /待機画像を読み込めません/); return true;
  });
  assert.equal(canvas.removed, true); assert.equal(state.host.renderer, null);
  assert.equal(state.attempts.length, 0);
});


test("同梱表示を続けながら追加一覧の失敗と復旧を通知し古い世代の結果は捨てる", async () => {
  const oldWork = deferred();
  const cause = new Error("追加一覧の権限不足");
  const state = hostSetup({ loadPack: async (id, { onUserCharacters }) => {
    if (id === "old") await oldWork.promise;
    onUserCharacters({ error: id === "broken" || id === "old" ? cause : null, root: "user-root" });
    return { id, manifest: {}, modes: ["2d"] };
  } });
  await state.host.load("broken");
  assert.equal(state.host.pack.id, "broken");
  const old = state.host.load("old");
  await state.host.load("recovered");
  oldWork.resolve();
  assert.equal(await old, null);
  assert.equal(state.host.pack.id, "recovered");
  const notices = state.events.filter(([event]) => event === "userCharacters");
  assert.equal(notices.length, 2);
  assert.equal(notices[0][1].error, cause);
  assert.equal(notices[1][1].error, null);
  assert.equal(notices[1][1].root, "user-root");
});
