import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { installFakeDom } from "./lib/fake-dom.mjs";
import { uiFile } from "./lib/ui-source.mjs";

const document = installFakeDom();
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { resolve, promise }; };
const turn = () => new Promise(resolve => setImmediate(resolve));
function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  if (tag === "button") node.disabled = false;
  Object.assign(node, props);
  node.append(...children.filter(Boolean));
  return node;
}
function load(file, factory, extra) {
  const context = vm.createContext({ el, logApp() {}, toast() {}, errorDetail: String, errorMessage: String,
    notifyBackgroundFailure() {}, clearBackgroundFailure() {},
    setTimeout, clearTimeout, twoStepAction: window.PicoUI.twoStepAction, ...extra });
  vm.runInContext(uiFile(file).replace(/^import .*;\r?$/gm, "").replace("export function", "function")
    + `\nthis.factory = ${factory};`, context);
  return context.factory;
}
function noticesHarness() {
  const snapshot = deferred();
  let stored = "[]";
  const factory = load("app/notice.js", "createNotices", {
    bridge: { scheduleRuns: () => snapshot.promise },
    localStorage: { getItem: () => stored, setItem: (_key, value) => { stored = value; } },
    window: { addEventListener() {}, PicoFrameScheduler: { create: callback => ({ schedule: callback }) } },
  });
  const panel = el("div"), button = el("button");
  panel.hidden = true;
  const notices = factory({ panel, button, status: { flash() {} } });
  return { panel, button, snapshot, notices, entries: () => JSON.parse(stored) };
}

test("スケジュール一覧の遅延応答は到着した通知と未読を消さない", async () => {
  const env = noticesHarness();
  const loading = env.notices.loadScheduleRuns();
  env.notices.push({ kind: "schedule", runId: "新規", title: "新しい結果", at: "2026-10-08T04:00:02Z" });
  env.notices.push({ kind: "通知", title: "通常の通知" });
  assert.equal(env.button.dataset.count, "2");
  env.snapshot.resolve([{ id: "過去", title: "過去の結果", at: "2026-10-08T04:00:00Z" }]);
  await loading;
  assert.ok(env.entries().some(entry => entry.runId === "新規"));
  assert.equal(env.button.dataset.count, "2");
});

test("通知を消去した後に古い一覧応答で消した通知を復活させない", async () => {
  const env = noticesHarness();
  const loading = env.notices.loadScheduleRuns();
  env.notices.push({ kind: "schedule", runId: "削除対象", title: "消す結果" });
  env.panel.querySelector(".chip").dispatch("click");
  env.snapshot.resolve([{ id: "削除対象", title: "消す結果", at: "2026-10-08T04:00:00Z" }]);
  await loading;
  assert.deepEqual(env.entries(), []);
});

for (const [method, command, label] of [["loadScheduleRuns", "scheduleRuns", "スケジュール実行履歴"], ["loadDurableJobs", "durableJobs", "永続ジョブ一覧"]]) {
  test(`${label}の取得失敗・不正応答は通知を保持して画面へ知らせ復旧を記録する`, async () => {
    let mode = "失敗", stored = "[]";
    const failures = [], recovered = [];
    const factory = load("app/notice.js", "createNotices", {
      bridge: { [command]: async () => { if (mode === "失敗") throw new Error("取得失敗の検査"); return mode === "不正" ? {} : []; } },
      notifyBackgroundFailure: (key, message) => failures.push({ key, message }),
      clearBackgroundFailure: key => recovered.push(key),
      localStorage: { getItem: () => stored, setItem: (_key, value) => { stored = value; } },
      window: { addEventListener() {}, PicoFrameScheduler: { create: callback => ({ schedule: callback }) } },
    });
    const notices = factory({ panel: el("div"), button: el("button"), status: { flash() {} } });
    notices.push({ kind: "通知", title: "既存の通知" });
    await notices[method]();
    assert.match(failures[0]?.message || "", /取得失敗の検査/);
    assert.equal(JSON.parse(stored)[0].title, "既存の通知");
    mode = "不正"; await notices[method]();
    assert.match(failures[1]?.message || "", /形式が不正/);
    assert.equal(JSON.parse(stored)[0].title, "既存の通知");
    mode = "成功"; await notices[method]();
    assert.deepEqual(recovered, [failures[0].key]);
  });
}

function relationshipHarness({ deleteMoment = async () => { throw new Error("保存失敗"); } } = {}) {
  const snapshot = deferred();
  const card = el("div"), button = el("button");
  card.hidden = true;
  let closeFocus = 0;
  const factory = load("app/relationship.js", "createRelationship", {
    bridge: { relationshipState: () => snapshot.promise,
      relationshipEvent: async () => ({ conversations: 2, recentMoments: [] }),
      deleteRelationshipMoment: deleteMoment },
  });
  const relationship = factory({ card });
  const close = card.children[0];
  close.focus = () => { closeFocus += 1; };
  card.querySelectorAll = selector => {
    if (selector !== "button") return [];
    const found = [];
    const walk = node => { if (node.tagName === "BUTTON") found.push(node); for (const child of node.children || []) walk(child); };
    walk(card);
    return found;
  };
  return { snapshot, card, button, close, relationship, get closeFocus() { return closeFocus; } };
}

test("記録カードを読込中に閉じると遅延応答で非表示ボタンへフォーカスしない", async () => {
  const env = relationshipHarness();
  const loading = env.relationship.toggle(env.button);
  env.relationship.hide();
  assert.equal(env.button.disabled, false, "閉じた時点で再度開く操作を戻す");
  env.snapshot.resolve({ conversations: 1, recentMoments: [] });
  await loading;
  assert.equal(env.relationship.open, false);
  assert.equal(env.closeFocus, 0);
  assert.equal(env.button.disabled, false);
});

test("古い記録カード読込は読込中に確定した新しい記録を巻き戻さない", async () => {
  const env = relationshipHarness();
  const loading = env.relationship.toggle(env.button);
  await env.relationship.record("conversation");
  assert.match(env.card.querySelector(".relationship-stats")?.textContent || env.card.children[3].textContent, /会話 2回/);
  env.snapshot.resolve({ conversations: 1, recentMoments: [] });
  await loading;
  assert.match(env.card.children[3].textContent, /会話 2回/);
});

test("記録の削除失敗後は削除中表示を戻しカードを閉じられる", async () => {
  const env = relationshipHarness();
  env.relationship.render({ recentMoments: [{ id: "1", text: "記録" }] });
  const remove = env.card.querySelectorAll("button").find(node => node.textContent === "削除");
  remove.dispatch("click");
  remove.dispatch("click");
  await turn();
  assert.equal(remove.textContent, "削除");
  assert.equal(env.close.disabled, false);
});

test("記録削除の応答待ち中もカードを閉じられる", async () => {
  const deletion = deferred();
  const env = relationshipHarness({ deleteMoment: () => deletion.promise });
  env.relationship.render({ recentMoments: [{ id: "1", text: "記録" }] });
  const remove = env.card.querySelectorAll("button").find(node => node.textContent === "削除");
  remove.dispatch("click");
  remove.dispatch("click");
  assert.equal(remove.disabled, true);
  assert.equal(env.close.disabled, false);
  env.close.dispatch("click");
  assert.equal(env.relationship.open, false);
  deletion.resolve({ recentMoments: [] });
  await turn();
});
