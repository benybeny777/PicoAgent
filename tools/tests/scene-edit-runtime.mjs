// 配置編集の実装を最小DOMで実行し、保存先と確定タイミングを検査する。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import * as model from "../../ui/app/scene-model.js";

function emitter() {
  const listeners = new Map();
  return {
    on(name, callback) { if (!listeners.has(name)) listeners.set(name, []); listeners.get(name).push(callback); },
    emit(name, value) { for (const callback of listeners.get(name) || []) callback(value); },
  };
}

class Element {
  constructor() {
    this.children = []; this.style = { setProperty() {} }; this.dataset = {};
    this.classList = { add() {}, remove() {}, toggle() {} }; this.hidden = false;
    this.listeners = new Map();
  }
  addEventListener(name, callback) { this.listeners.set(name, callback); }
  removeEventListener(name) { this.listeners.delete(name); }
  setPointerCapture(id) { this.capture = id; }
  hasPointerCapture(id) { return this.capture === id; }
  releasePointerCapture() { this.capture = null; }
  append(...children) { this.children.push(...children); }
  prepend(...children) { this.children.unshift(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute() {}
  querySelectorAll() { return []; }
  getBoundingClientRect() { return { left: 0, top: 0, right: 800, bottom: 600 }; }
  closest() { return null; }
}

function moduleFactory(path, name, context) {
  const source = readFileSync(new URL(`../../ui/app/${path}`, import.meta.url), "utf8")
    .replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/export function/g, "function");
  return runInNewContext(`${source}\n${name}`, context);
}

test("キャラ非表示は形式とキャラごとの保存先に入り、再読込後も表示へ反映する", async () => {
  let persisted;
  const stageElement = new Element(); stageElement.clientWidth = 800; stageElement.clientHeight = 600;
  const source = { element: new Element(), setVisible(value) { this.active = value; } };
  const factory = moduleFactory("stage.js", "createStage", {
    ...model, clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    bridge: { saveScene: async value => { persisted = value; } },
    createEmitter: emitter, logApp() {}, errorDetail: String, errorMessage: String,
    window: { devicePixelRatio: 1 }, ResizeObserver: class { observe() {} }, requestAnimationFrame: () => 1,
  });
  const stage = factory({ stageElement, sceneRoot: new Element(), sources: new Map([["character", source]]),
    character: { resolveFraming: () => "bust", framingAspect: () => 0.5 } });
  stage.load({ pet: { sources: { character: { anchor: "bottom-right", x: 0, y: 0, h: 400 } }, order: ["character"] } }, { version: 1 });
  stage.setCharacter("mugi");
  await stage.setVisible("character", false);
  assert.equal(persisted.pet.characters.mugi.visible, false);
  assert.equal(persisted.pet.sources, undefined);
  assert.equal(source.element.hidden, true);
  assert.equal(source.active, false);
  stage.replaceScenes(persisted); stage.layoutNow();
  assert.equal(source.element.hidden, true);
  stage.setCharacter("rin"); stage.layoutNow();
  assert.equal(source.element.hidden, false);
  assert.equal(source.active, true);
});

test("キャラ非表示中に描画器を読み替えても停止状態を保ち、表示復帰で再開する", async () => {
  class Renderer {
    static supports() { return true; }
    constructor() { this.element = new Element(); }
    async load() {}
    setFraming() {}
    setMotion() {}
    setSpeaking() {}
    setActive(value) { this.active = value; }
    dispose() {}
  }
  class Unsupported { static supports() { return false; } }
  const source = readFileSync(new URL("../../ui/character/index.js", import.meta.url), "utf8")
    .replace(/^import[\s\S]*?;\r?\n/gm, "").replace(/export (function|const)/g, "$1");
  const factory = runInNewContext(`${source}\ncreateCharacterHost`, {
    ImageRenderer: Renderer, MmdRenderer: Unsupported, MeshRenderer: Unsupported, Live2dRenderer: Unsupported,
    loadPack: async id => ({ id, manifest: {} }), personaText: () => "", bridge: {},
    createEmitter: emitter, el: () => new Element(), errorMessage: String, errorDetail: String, logApp() {},
    window: { matchMedia: () => ({ matches: false, addEventListener() {} }) },
    document: { hidden: false, addEventListener() {} },
  });
  const host = factory({ container: new Element() });
  host.setActive(false);
  await host.load("mugi");
  assert.equal(host.renderer.active, false);
  await host.load("rin");
  assert.equal(host.renderer.active, false);
  host.setActive(true);
  assert.equal(host.renderer.active, true);
});

function editorFixture() {
  const commits = [];
  const previews = new Map();
  const events = emitter();
  const stage = {
    events, sources: new Map(["character", "chat"].map(name => [name, { element: new Element() }])),
    sourceNames: () => ["character", "chat"], placementOf: () => ({ visible: true }),
    rectOf: name => previews.get(name) || { left: 20, top: 30, width: 100, height: 200 },
    toWindow: value => value, geometry: () => ({ scale: 1 }), currentFraming: () => "bust",
    preview: (name, value) => previews.set(name, value), cancelPreview: name => previews.delete(name),
    commit: async (name, value) => { commits.push({ name, value }); previews.delete(name); },
  };
  let timerId = 0;
  const timers = new Map();
  const factory = moduleFactory("edit.js", "createEditor", {
    anchorFactors: model.anchorFactors, SOURCE_LABELS: {}, Element,
    el: (_tag, props = {}, children = []) => Object.assign(new Element(), props, { children }),
    document: { body: new Element(), addEventListener() {} }, window: { devicePixelRatio: 1 },
    HTMLInputElement: class {}, HTMLTextAreaElement: class {},
    setTimeout: callback => { const id = ++timerId; timers.set(id, callback); return id; },
    clearTimeout: id => timers.delete(id),
  });
  const editor = factory({ layer: new Element(), toolbar: new Element(), stageElement: new Element(), stage,
    host: { maxHeight: () => Infinity } });
  editor.start("all");
  const wheel = name => editor.wheel({ deltaY: -1, preventDefault() {} }, name);
  return { editor, wheel, commits, previews, timers, events };
}

test("ホイール直後に編集を閉じても最後のサイズを保存する", () => {
  const fixture = editorFixture();
  fixture.wheel("character"); fixture.editor.stop();
  assert.equal(fixture.commits.length, 1);
  assert.ok(fixture.commits[0].value.height > 200);
  assert.equal(fixture.previews.size, 0);
  assert.equal(fixture.timers.size, 0);
});

test("ホイールの対象切替と形式切替でも各対象を確定して一時値を残さない", () => {
  const fixture = editorFixture();
  fixture.wheel("character"); fixture.wheel("chat");
  assert.equal(fixture.commits[0].name, "character");
  fixture.events.emit("context-change");
  assert.deepEqual(fixture.commits.map(value => value.name), ["character", "chat"]);
  assert.equal(fixture.previews.size, 0);
  assert.equal(fixture.editor.active, false);
});

test("ホイール後に会話パネルのつまみでドラッグしても保留を先に確定する", () => {
  const fixture = editorFixture();
  fixture.wheel("chat");
  const grip = new Element();
  fixture.editor.attachGrip("chat", grip);
  grip.listeners.get("pointerdown")({ button: 0, pointerId: 1, clientX: 40, clientY: 40, preventDefault() {}, stopPropagation() {} });
  assert.equal(fixture.commits.length, 1);
  assert.equal(fixture.timers.size, 0);
  fixture.editor.stop();
});

test("バックエンドの配置リセット後に古いホイール配置を再保存しない", () => {
  const fixture = editorFixture();
  fixture.wheel("character");
  fixture.events.emit("context-reset");
  assert.equal(fixture.commits.length, 0);
  assert.equal(fixture.timers.size, 0);
  assert.equal(fixture.previews.size, 0);
  assert.equal(fixture.editor.active, false);
});
