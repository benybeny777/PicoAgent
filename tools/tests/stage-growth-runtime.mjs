// 窓の拡張を実際に反映し、アンカーの追従でキャラが再び窓外へ逃げないことを確かめる。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
import * as model from "../../ui/app/scene-model.js";

const source = readFileSync(new URL("../../ui/app/stage.js", import.meta.url), "utf8")
  .replace(/^import[\s\S]*?;\r?\n/gm, "").replace("export function createStage", "function createStage");

function setup({ barHeight = 0, fixed = false, fail = false, ratio = 1 } = {}) {
  const win = { innerWidth: 900, innerHeight: 700, devicePixelRatio: ratio,
    screen: { availWidth: 1920, availHeight: 1080 } };
  const stageElement = { clientWidth: 900, clientHeight: 700 - barHeight,
    closest: () => null, classList: { toggle() {} } };
  const element = () => ({ style: {} });
  const calls = [];
  const position = { x: 200, y: 200 };
  let saved;
  function resize(width, height) {
    if (fail) throw new Error("窓寸法の変更失敗");
    win.innerWidth = width; win.innerHeight = height;
    stageElement.clientWidth = width; stageElement.clientHeight = height - barHeight;
  }
  const factory = runInNewContext(`${source}\ncreateStage`, {
    ...model, clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    createEmitter: () => ({ emit() {} }), logApp() {}, errorDetail: String, errorMessage: String,
    requestAnimationFrame: () => 1, ResizeObserver: class { observe() {} }, window: win,
    bridge: {
      saveScene: async value => { saved = value; },
      setWindowSize: async (width, height) => { calls.push({ width, height }); resize(width, height); },
      moveAndResizeWindow: async (leftDelta, topDelta, width, height) => {
        calls.push({ leftDelta, topDelta, width, height });
        resize(width, height); position.x -= leftDelta; position.y -= topDelta;
      },
    },
  });
  const stage = factory({ stageElement, sceneRoot: element(),
    sources: new Map([["character", { element: element() }], ["chat", { element: element() }]]),
    character: { resolveFraming: stored => stored || "full", framingAspect: () => 0.5 } });
  const chat = { anchor: "top-left", x: 10, y: 10, w: 200, h: 100 };
  const def = { sources: { character: { anchor: "bottom-right", h: 300 }, chat }, order: ["character", "chat"] };
  if (fixed) def.size = { w: 900, h: 700 };
  stage.load({ pet: def }, { version: 1, pet: { sources: { chat },
    characters: { other: { anchor: "center", x: 0, y: 0, w: 100, h: 200, framing: "bust" } } } });
  stage.setCharacter("rice");
  return { stage, win, calls, position, def, get saved() { return saved; } };
}

function close(actual, expected) {
  assert.ok(Math.abs(actual - expected) <= 1, `${actual} は ${expected} からずれています`);
}

test("右アンカーのライス縮小は拡張後も元の座標に残り窓内へ収まる", async () => {
  const state = setup();
  const others = state.stage.scenes.pet;
  await state.stage.commit("character", { left: 665, top: 50, width: 373, height: 600 });
  const rect = state.stage.rectOf("character");
  assert.equal(state.win.innerWidth, 1038);
  assert.equal(rect.left, 665);
  assert.equal(rect.left + rect.width, 1038);
  assert.equal(state.saved.pet.characters.rice.anchor, "right");
  assert.equal(state.saved.pet.characters.rice.x, 0);
  assert.deepEqual(state.saved.pet.sources, others.sources);
  assert.deepEqual(state.saved.pet.characters.other, others.characters.other);
  state.stage.replaceScenes(state.saved); state.stage.layoutNow();
  assert.equal(state.stage.rectOf("character").left, 665);
});

test("左上へ窓を移動した分だけシーン内座標を補正し画面上の位置を保つ", async () => {
  const state = setup({ ratio: 2 });
  const rect = { left: -100, top: -80, width: 400, height: 500 };
  await state.stage.commit("character", rect);
  const actual = state.stage.rectOf("character");
  assert.deepEqual(state.calls[0], { leftDelta: 100, topDelta: 80, width: 1000, height: 780 });
  close(state.position.x + actual.left, 200 + rect.left);
  close(state.position.y + actual.top, 200 + rect.top);
  assert.equal(actual.left, 0); assert.equal(actual.top, 0);
});

test("固定タイトルバーの高さを窓寸法に含め下アンカーの位置を保つ", async () => {
  const state = setup({ barHeight: 40 });
  await state.stage.commit("character", { left: 700, top: 400, width: 300, height: 400 });
  const rect = state.stage.rectOf("character");
  assert.equal(state.win.innerWidth, 1000);
  assert.equal(state.win.innerHeight, 840);
  assert.equal(state.stage.geometry().height, 800);
  assert.equal(rect.left, 700); assert.equal(rect.top, 400);
  assert.equal(rect.top + rect.height, state.stage.geometry().height);
});

test("9つのアンカーすべてで窓拡張後の画面座標を保つ", async () => {
  for (const anchor of model.ANCHORS) {
    const state = setup();
    const rect = { left: -100, top: -80, width: 1100, height: 900 };
    state.def.sources.character = model.fromRect(rect, anchor, 900, 700);
    await state.stage.resetCharacter();
    const actual = state.stage.rectOf("character");
    close(state.position.x + actual.left, 200 + rect.left);
    close(state.position.y + actual.top, 200 + rect.top);
    assert.equal(state.saved.pet.characters.rice.anchor, anchor);
    assert.equal(actual.width, rect.width); assert.equal(actual.height, rect.height);
  }
});

test("固定シーンと会話ソースの配置確定では窓を拡張しない", async () => {
  const state = setup({ fixed: true });
  await state.stage.commit("character", { left: 850, top: 650, width: 300, height: 500 });
  assert.equal(state.calls.length, 0);
  const ordinary = setup();
  await ordinary.stage.commit("chat", { left: -100, top: -80, width: 1100, height: 900 });
  assert.equal(ordinary.calls.length, 0);
});

test("窓の拡張に失敗した場合は未実行の拡張を保存座標へ反映しない", async () => {
  const state = setup({ fail: true });
  const rect = { left: 665, top: 50, width: 373, height: 600 };
  await state.stage.commit("character", rect);
  assert.equal(state.win.innerWidth, 900);
  assert.equal(state.saved.pet.characters.rice.x, 138);
  assert.equal(state.stage.rectOf("character").left, rect.left);
});
