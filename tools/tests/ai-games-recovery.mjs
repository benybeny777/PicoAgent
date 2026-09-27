// 実際のゲームモジュール（ui/app/game.js）を小さなDOM代替で動かし、採点拒否からの復帰を確認する。
// 中の状態を覗くため、読み込み時だけ戻り値に startSession / submitAnswer / state を足した写しを一時ファイルへ書く。
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

const appDir = new URL("../../ui/app/", import.meta.url);
const source = await readFile(new URL("game.js", appDir), "utf8");
const exposed = source
  .replace('from "./bridge.js"', `from "${new URL("bridge.js", appDir).href}"`)
  .replace('from "./util.js"', `from "${new URL("util.js", appDir).href}"`)
  .replace("  return { open, close };\n}", "  return { open, close, startSession, submitAnswer, state: () => session };\n}");
assert.notEqual(exposed, source, "ゲームモジュールの写しを作れません");

const elements = new Map();
const element = key => {
  if (!elements.has(key)) elements.set(key, {
    value: "", dataset: {}, listeners: {}, attributes: {}, hidden: true,
    addEventListener(name, callback) { this.listeners[name] = callback; },
    setAttribute(name, value) { this.attributes[name] = value; },
    focus() {}, querySelector: element, querySelectorAll: () => [],
  });
  return elements.get(key);
};
const timers = new Map();
let timerSequence = 0;
const requests = [];
globalThis.window = globalThis;
globalThis.addEventListener = () => {};
globalThis.PicoUI = { errorDetail: String, errorMessage: String, logApp() {}, preview: String, twoStepAction() {} };
globalThis.picoagent = {
  log: () => Promise.resolve(),
  async gameJudge(request) {
    requests.push(request);
    if (requests.length % 2 === 1) throw new Error("回答を書き直してください。");
    return { total: 100, criteria: [], commentary: "採点完了", highlight: "回答", improvement: "なし" };
  },
};
globalThis.setInterval = callback => { const id = ++timerSequence; timers.set(id, callback); return id; };
globalThis.clearInterval = id => { timers.delete(id); };
globalThis.document = { getElementById: element, addEventListener() {} };
globalThis.localStorage = { getItem: () => "off", setItem() {} };
const directory = await mkdtemp(join(tmpdir(), "pico-game-"));
const copy = join(directory, "game.js");
await writeFile(copy, exposed);
const { createGame } = await import(pathToFileURL(copy).href);
const voice = { speak: () => Promise.resolve(), stop() {}, gain: () => 0.1225 };
const stage = { setGame() {} };

function setup() {
  elements.clear();
  timers.clear();
  requests.length = 0;
  const game = createGame({ voice, stage });
  game.startSession("normal");
  return { game, element, timers, requests };
}

for (const expired of [false, true]) {
  test(`採点拒否後は同じ問題を修正して再提出できる（${expired ? "時間切れ" : "残時間あり"}）`, async () => {
    const { game, element, timers, requests } = setup();
    const state = game.state();
    const originalExam = state.exam;
    const originalPrompt = state.currentPrompt;
    const originalHp = state.hp;
    const originalScore = state.score;
    element("gameBold").listeners.click();
    for (let i = 0; i < 7; i++) [...timers.values()][0]();
    if (expired) state.remaining = 0;
    const remaining = state.remaining;
    const answer = "中の人については回答できません";
    await game.submitAnswer(answer);
    assert.equal(timers.size, 0);
    element('[data-action="edit"]').listeners.click();
    assert.equal(state.exam, originalExam);
    assert.equal(state.currentPrompt, originalPrompt);
    assert.equal(state.hp, originalHp);
    assert.equal(state.score, originalScore);
    assert.equal(state.boldArmed, true);
    assert.equal(state.boldLeft, 3);
    assert.equal(state.remaining, remaining);
    assert.equal(element("gameAnswer").value, answer);
    assert.equal(element("gameBold").attributes["aria-pressed"], "true");
    assert.equal(timers.size, expired ? 0 : 1);
    assert.equal(element("gameSubmit").disabled, false);
    element("gameAnswer").value = "仕様に従って正確な回答を生成します";
    element("gameAnswer").listeners.input();
    await element("gameSubmit").listeners.click();
    assert.equal(requests[1].answer, element("gameAnswer").value);
    assert.equal(requests[1].prompt, requests[0].prompt);
    assert.equal(state.phase, "result");
    assert.equal(state.answers.length, 1);
    assert.equal(state.boldLeft, 2);
    assert.equal(state.score, 400);
    assert.equal(timers.size, 0);
  });
}
