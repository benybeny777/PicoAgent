import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";
import { installStorageRuntime } from "./lib/storage-runtime.mjs";

function harness(file, initial = {}, mode = {}) {
  const values = new Map(Object.entries(initial)), writes = [], removals = [], failures = [], recoveries = [];
  const nodes = new Map();
  function node(key, props = {}, children = []) {
    if (!nodes.has(key)) nodes.set(key, { hidden: false, dataset: {}, children, listeners: {}, ...props,
      addEventListener(type, callback) { this.listeners[type] = callback; }, setAttribute() {}, focus() {},
      replaceChildren(...items) { this.children = items; }, append(...items) { this.children.push(...items); },
      querySelector: key => node(key), querySelectorAll: () => [] });
    return nodes.get(key);
  }
  const context = vm.createContext({ console, document: { getElementById: node, addEventListener() {} },
    el: (tag, props, children) => node(Symbol(tag), props, children), bridge: {}, logApp() {}, errorMessage: String,
    notifyBackgroundFailure: (key, message) => failures.push({ key, message }), clearBackgroundFailure: key => recoveries.push(key),
    localStorage: {
      getItem(key) { if (mode.read) throw new Error("秘密の保存値"); return values.get(key) ?? null; },
      setItem(key, value) { writes.push(key); if (mode.write) throw new Error("秘密の保存値"); values.set(key, value); },
      removeItem(key) { removals.push(key); if (mode.remove) throw new Error("秘密の保存値"); values.delete(key); },
    },
    window: { addEventListener() {}, clearInterval() {}, PicoFrameScheduler: { create: callback => ({ schedule: callback, flush: callback }) } },
  });
  installStorageRuntime(context);
  let source = uiFile(`app/${file}.js`).replace(/^import .*;\r?$/gm, "").replace("export function", "function");
  if (file === "game") source = source.replace("return { open, close };", "return { open, close, finish: () => { session = { difficulty: 'normal', score: 500, results: [] }; renderGameOver(); } };");
  vm.runInContext(source + `\nthis.factory = ${file === "game" ? "createGame" : "createNotices"};`, context);
  const panel = node("panel"), button = node("button");
  const api = context.factory(file === "game" ? { voice: { gain: () => 0, speak: async () => {}, stop() {} }, stage: { setGame() {} } }
    : { panel, button, status: { flash() {} } });
  return { api, values, writes, removals, failures, recoveries, nodes, panel, button, mode };
}
const noticeKey = "picoagent.actionCenter.v1";
const recordsKey = "picoagent.ai-games.records.v1";
const ranksKey = "picoagent.ai-games.ranks.v1";
const control = (env, text) => [...env.nodes.values()].find(node => node.textContent === text);
const click = node => node.listeners.click({ stopPropagation() {} });
const reason = env => [...env.nodes.values()].find(node => node.role === "alert");

test("通知の読取不能・破損削除失敗は原本を空保存せず秘密を通知へ出さない", () => {
  for (const mode of [{ read: true }, { remove: true }]) {
    const env = harness("notice", { [noticeKey]: "秘密の壊れたJSON" }, mode);
    env.api.push({ title: "新しい通知", body: "内容" });
    assert.equal(env.writes.length, 0);
    assert.equal(env.values.get(noticeKey), "秘密の壊れたJSON");
    assert.equal(env.failures.length, 1);
    assert.doesNotMatch(JSON.stringify(env.failures), /秘密/);
  }
});
test("破損通知の削除成功後だけ消去通知を一件保存する", () => {
  const env = harness("notice", { [noticeKey]: "[null]" });
  assert.equal(env.removals.length, 1);
  const entries = JSON.parse(env.values.get(noticeKey));
  assert.equal(entries.length, 1);
  assert.match(entries[0].body, /消しました/);
});
test("通知保存失敗は再帰保存せず成功時に解除し次の失敗を通知する", () => {
  const env = harness("notice", {}, { write: true });
  assert.equal(env.writes.length, 1);
  env.mode.write = false;
  env.api.push({ title: "復旧" });
  assert.equal(env.recoveries.length, 1);
  env.mode.write = true;
  env.api.push({ title: "再失敗" });
  assert.equal(env.failures.length, 2);
  assert.equal(env.writes.length, 3);
});
test("通知履歴の失敗理由と再読込操作は常設され、再失敗でも新着と未読を保持する", () => {
  const old = { kind: "schedule", runId: "同じ実行", title: "古い履歴", body: "過去", at: "2026-10-10T10:00:00Z", ok: true };
  const env = harness("notice", { [noticeKey]: JSON.stringify([old]) }, { read: true });
  const retry = control(env, "通知履歴を再読込");
  assert.equal(retry.hidden, false);
  assert.equal(reason(env).hidden, false);
  assert.match(reason(env).textContent, /読み込めず保存を停止/);
  env.api.push({ ...old, title: "新着", body: "現在", at: "2026-10-10T11:00:00Z" });
  env.api.push({ kind: "通知", title: "別の新着" });
  click(retry);
  assert.equal(env.button.dataset.count, "2");
  assert.equal(env.writes.length, 0);
  assert.equal(env.values.get(noticeKey), JSON.stringify([old]));
  env.mode.read = false;
  click(retry);
  const entries = JSON.parse(env.values.get(noticeKey));
  assert.equal(entries.length, 2);
  assert.equal(entries.find(entry => entry.runId === old.runId).body, "現在");
  assert.equal(env.button.dataset.count, "2");
  assert.equal(reason(env).hidden, true);
  assert.equal(retry.hidden, true);
  assert.equal(env.recoveries.length, 1);
});
test("破損の再読込は削除成功後だけ消去項目を追加し新着を残す", () => {
  const env = harness("notice", { [noticeKey]: "壊れた原本" }, { remove: true });
  env.api.push({ kind: "通知", title: "保存待ちの新着" });
  click(control(env, "通知履歴を再読込"));
  assert.equal(env.writes.length, 0);
  assert.match(reason(env).textContent, /原本を保持/);
  env.mode.remove = false;
  click(control(env, "通知履歴を再読込"));
  const entries = JSON.parse(env.values.get(noticeKey));
  assert.equal(entries.filter(entry => entry.title === "通知履歴を消しました").length, 1);
  assert.ok(entries.some(entry => entry.title === "保存待ちの新着"));
  assert.equal(env.button.dataset.count, "2");
  assert.equal(reason(env).hidden, true);
});
test("通知の明示消去失敗は理由と新着を残し成功時に保存停止と原因を解除する", () => {
  const env = harness("notice", {}, { read: true, remove: true });
  env.api.push({ title: "消去前の新着" });
  const clear = control(env, "消去");
  click(clear);
  assert.equal(env.button.dataset.count, "1");
  assert.equal(env.writes.length, 0);
  assert.match(reason(env).textContent, /消去できません/);
  env.mode.remove = false;
  click(clear);
  assert.equal(env.button.dataset.count, "");
  assert.equal(reason(env).hidden, true);
  assert.deepEqual(JSON.parse(env.values.get(noticeKey)), []);
  assert.equal(env.recoveries.length, 2);
});
test("ゲームの破損・型不正・読取不能は遊べても全記録の原本保存を停止する", () => {
  for (const [raw, mode] of [["秘密のJSON", {}], ['{"normal":{"score":-1,"rounds":2}}', {}], ['{"normal":{"score":"5","rounds":2}}', {}], ["{}", { read: true }]]) {
    const env = harness("game", { [recordsKey]: raw }, mode);
    env.api.open(); env.api.finish();
    assert.equal(env.writes.length, 0);
    assert.equal(env.removals.length, 0);
    assert.equal(env.values.get(recordsKey), raw);
    assert.doesNotMatch(env.nodes.get("gameContent").innerHTML, /NEW RANK|NEW RECORD/);
    assert.match(env.nodes.get("gameStatus").textContent, /読めません/);
    assert.doesNotMatch(JSON.stringify(env.failures), /秘密/);
  }
});
test("ゲームの保存失敗で達成演出を出さず、復旧した保存後だけ達成を出す", () => {
  const env = harness("game", {}, { write: true });
  env.api.finish();
  assert.doesNotMatch(env.nodes.get("gameContent").innerHTML, /NEW RANK|NEW RECORD/);
  assert.match(env.nodes.get("gameStatus").textContent, /保存できません/);
  env.mode.write = false; env.api.finish();
  assert.match(env.nodes.get("gameContent").innerHTML, /NEW RANK/);
  assert.equal(JSON.parse(env.values.get(recordsKey)).normal.score, 500);
  assert.equal(JSON.parse(env.values.get(ranksKey)).length, 1);
  assert.equal(env.recoveries.length, 2);
  assert.doesNotMatch(env.nodes.get("gameStatus").textContent, /保存できません/);
});
test("音設定の不在は有効、読取不能は不明として表示する", () => {
  assert.equal(harness("game").nodes.get("gameSound").textContent, "音あり");
  assert.equal(harness("game", {}, { read: true }).nodes.get("gameSound").textContent, "音設定を確認できません");
});
test("称号の型不正も全記録保存を停止し、既存の大きな有限記録は保持する", () => {
  for (const raw of ['{}', '[null]', '["未知の称号"]', '["ai-itself","ai-itself"]']) {
    const env = harness("game", { [ranksKey]: raw });
    env.api.finish();
    assert.equal(env.writes.length, 0);
    assert.equal(env.values.get(ranksKey), raw);
    assert.match(env.nodes.get("gameStatus").textContent, /形式が不正/);
  }
  const record = { normal: { score: 999999, rounds: 999 } };
  const env = harness("game", { [recordsKey]: JSON.stringify(record) });
  env.api.finish();
  assert.deepEqual(JSON.parse(env.values.get(recordsKey)), record);
  assert.ok(!env.writes.includes(recordsKey));
});
test("明示した音の変更は保存失敗理由を出し、次の保存成功で解除する", () => {
  const env = harness("game", {}, { write: true });
  env.nodes.get("gameSound").listeners.click();
  assert.match(env.nodes.get("gameStatus").textContent, /保存できません/);
  env.mode.write = false;
  env.nodes.get("gameSound").listeners.click();
  assert.equal(env.recoveries.length, 1);
  assert.equal(env.nodes.get("gameStatus").textContent, "");
});
