import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const turn = () => new Promise(resolve => setImmediate(resolve));
function element(_tag, attributes = {}, children = []) {
  return { hidden: true, dataset: {}, children, ...attributes, classList: { toggle() {} }, addEventListener() {},
    replaceChildren(...nodes) { this.children = nodes; }, append(...nodes) { this.children.push(...nodes); } };
}
function load(file, factory, values) {
  const context = vm.createContext({ logApp() {}, errorDetail: String, errorMessage: String, ...values });
  vm.runInContext(uiFile(file).replace(/^import .*;\r?$/gm, "").replace("export function", "function") + `\nthis.factory = ${factory};`, context);
  return context.factory;
}

for (const phase of ["file", "screen", "subscription"]) {
  test(`送信の${phase}待ちで停止すると待機を解除し遅延結果でもAIへ送信しない`, async () => {
    const pending = deferred();
    const timers = new Map();
    let id = 0, sends = 0, released = 0;
    const factory = load("chat/send.js", "createSender", {
      bridge: {
        listen: () => phase === "subscription" ? pending.promise.then(() => () => { released += 1; }) : Promise.resolve(() => {}),
        readFileContent: () => pending.promise, captureScreen: () => pending.promise,
        send: async () => { sends += 1; return { reply: "返答" }; }, cancel: async () => {},
      },
      window: { PicoTaskRun: { createController: () => ({ start() {}, finish() {} }) } },
      setTimeout: callback => { timers.set(++id, callback); return id; }, clearTimeout: value => timers.delete(value),
      cancelAnimationFrame() {},
    });
    const composer = { hasContent: () => true, take: () => ({ text: phase === "screen" ? "画面を見て" : "質問", images: [], files: phase === "file" ? [{ name: "資料.txt" }] : [] }),
      close() {}, setBlocked() {}, onSubmit() {}, setChoices() {} };
    const noop = () => {};
    const sender = factory({
      chat: { composer, element: element(), status: { clear: noop, set: noop, setBusy: noop, flash: noop },
        buttons: { regenerateButton: element(), cancelSendButton: element() },
        history: { add: noop }, setThinking: noop, clearThinking: noop },
      voice: { stop: noop, beginStream: noop, cancelStream: noop },
      motion: { setBusy: noop }, notices: { upsertJob: noop }, engine: { blockedReason: () => "" },
      relationship: {}, ambient: { onUserActivity: noop, afterConversation: noop }, mic: { scheduleContinuous: noop },
    });
    const submission = sender.submit();
    await turn();
    await sender.cancel();
    [...timers.values()][0]();
    await submission;
    assert.equal(sender.busy, false);
    pending.resolve(phase === "screen" ? { dataUrl: "画像" } : "本文");
    await turn();
    assert.equal(sends, 0);
    if (phase === "subscription") assert.equal(released, 1);
  });
}

test("永続ジョブは古い進捗と一覧で完了を巻き戻さず新しい再試行は受け取る", async () => {
  const snapshot = deferred();
  let stored = "[]";
  const factory = load("app/notice.js", "createNotices", {
    bridge: { durableJobs: () => snapshot.promise },
    localStorage: { getItem: () => stored, setItem: (_key, text) => { stored = text; } },
    el: element, window: { addEventListener() {}, PicoFrameScheduler: { create: callback => ({ schedule: callback }) } },
  });
  const notices = factory({ panel: element(), button: element(), status: { flash() {} } });
  const event = (state, at) => ({ kind: "durable_job", jobId: "1", state, at });
  notices.push(event("running", "2026-10-03T10:00:00+09:00"));
  const loading = notices.loadDurableJobs();
  notices.push(event("completed", "2026-10-03T10:00:02+09:00"));
  notices.push(event("running", "2026-10-03T10:00:02+09:00"));
  assert.equal(JSON.parse(stored)[0].state, "completed");
  notices.push(event("running", "2026-10-03T10:00:01+09:00"));
  notices.push({ ...event("completed", "2026-10-03T09:59:00+09:00"), jobId: "2" });
  assert.equal(JSON.parse(stored)[0].state, "completed");
  snapshot.resolve([{ id: "1", state: "running", updated_at: "2026-10-03T10:00:00+09:00" }]);
  await loading;
  assert.equal(JSON.parse(stored)[0].state, "completed");
  assert.equal(JSON.parse(stored).find(entry => entry.jobId === "2").state, "completed");
  notices.push(event("queued", "2026-10-03T10:00:03+09:00"));
  notices.push(event("running", "2026-10-03T10:00:04+09:00"));
  assert.equal(JSON.parse(stored)[0].state, "running");
  notices.push(event("completed", "2026-10-03T10:00:05+09:00"));
  // queued の購読前に再試行が始まった場合も、新しい状態へ復旧できる。
  notices.push(event("running", "2026-10-03T10:00:06+09:00"));
  assert.equal(JSON.parse(stored)[0].state, "running");
});

test("会話ジョブを繰り返してもアクションセンターは直近50件に収まる", () => {
  let list;
  const factory = load("app/notice.js", "createNotices", {
    bridge: {}, localStorage: { getItem: () => "[]", setItem() {} },
    el: (tag, attributes, children) => {
      const node = element(tag, attributes, children);
      if (attributes?.className === "notice-list") list = node;
      return node;
    },
    window: { addEventListener() {}, PicoFrameScheduler: { create: callback => ({ schedule: callback }) } },
  });
  const panel = element();
  panel.hidden = false;
  const notices = factory({ panel, button: element(), status: { flash() {} } });
  for (let index = 0; index < 60; index += 1) notices.upsertJob(String(index), { status: "completed" });
  assert.equal(list.children.length, 50);
});

test("起動で非表示になったキャラは有効なキャラの読込成功後に再表示する", async () => {
  const source = uiFile("app/main.js");
  const body = source.slice(source.indexOf("async function loadCharacter("), source.indexOf("function localStorageVariant("));
  const characterSource = { element: { style: { visibility: "hidden" } } };
  const context = vm.createContext({
    loadedSignature: "", signatureOf: id => id, characterOverrides: {}, sdkOverrides: {}, characterName: "",
    voice: { stop() {} }, host: { load: async () => ({ pack: { id: "mugi", name: "むぎ", manifest: {} }, greeting: "" }) },
    stage: { setCharacter() {} }, chrome: { setCharacterName() {} }, characterSource,
    renderCharacterOptions() {}, motion: { rest() {} }, personaText: () => "人格",
    bridge: { setCharacterProfile: async () => ({}) }, applyStreamAppearance() {},
  });
  vm.runInContext(body + "\nthis.load = loadCharacter;", context);
  await context.load("mugi");
  assert.equal(characterSource.element.style.visibility, "");
  characterSource.element.style.visibility = "hidden";
  context.host.load = async () => { throw new Error("読込失敗"); };
  await assert.rejects(context.load("missing"), /読込失敗/);
  assert.equal(characterSource.element.style.visibility, "hidden");
});
