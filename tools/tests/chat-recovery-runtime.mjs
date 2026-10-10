import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";
import { installFakeDom } from "./lib/fake-dom.mjs";

const turn = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

function senderHarness({ send, cancel }) {
  const timers = new Map(), messages = [], replies = [], spoken = [], choices = [], outcomes = [];
  let id = 0, stopCount = 0;
  const noop = () => {};
  const button = () => ({ addEventListener() {} });
  const context = vm.createContext({
    bridge: { listen: async () => noop, send, cancel, quit: () => { throw new Error("終了してはいけません"); } },
    window: { PicoTaskRun: { createController: () => ({ start: noop, finish: state => outcomes.push(state) }) } },
    setTimeout: (callback, delay) => { timers.set(++id, { callback, delay }); return id; },
    clearTimeout: key => timers.delete(key), cancelAnimationFrame: noop,
    logApp: noop, errorDetail: String, errorMessage: String,
  });
  vm.runInContext(uiFile("chat/send.js").replace(/^import .*;\r?$/gm, "").replace("export function", "function")
    + "\nthis.factory = createSender;", context);
  const sender = context.factory({
    chat: {
      composer: { hasContent: () => true, take: () => ({ text: "質問", images: [], files: [] }),
        close: noop, clearAfterReset: noop, setBlocked: noop, onSubmit: noop, setChoices: value => choices.push(value) },
      status: { clear: noop, set: text => messages.push(text), setBusy: noop, flash: text => messages.push(text) },
      buttons: { regenerateButton: button(), cancelSendButton: button() },
      history: { add: (role, text) => replies.push({ role, text }), clear: () => { replies.length = 0; } },
      setThinking: noop, clearThinking: noop, renderReply: noop, showError: noop,
    },
    voice: { stop: () => { stopCount += 1; }, beginStream: noop, cancelStream: noop,
      finishStream: () => false, speak: text => spoken.push(text) },
    motion: { setBusy: noop, react: noop }, notices: { upsertJob: noop },
    engine: { blockedReason: () => "" }, relationship: {},
    ambient: { onUserActivity: noop, afterConversation: noop }, mic: { scheduleContinuous: noop },
  });
  const release = () => {
    const entry = [...timers].find(([, timer]) => timer.delay === 8000);
    assert.ok(entry, "停止要求が失敗しても待機解除タイマーを保持する");
    timers.delete(entry[0]);
    entry[1].callback();
  };
  return { sender, timers, messages, replies, spoken, choices, outcomes, release, get stopCount() { return stopCount; } };
}

test("停止コマンドが拒否されても応答待ちを8秒で解除して再送できる", async () => {
  let requests = 0;
  const env = senderHarness({ send: () => ++requests === 1 ? new Promise(() => {}) : Promise.resolve({ reply: "次の回答" }),
    cancel: async () => { throw new Error("接続切断"); } });
  const pending = env.sender.submit();
  await turn();
  await env.sender.cancel();
  env.release();
  await pending;
  assert.equal(env.sender.busy, false);
  assert.deepEqual(env.outcomes, ["cancelled"]);
  await env.sender.submit();
  assert.equal(requests, 2);
  assert.deepEqual(env.outcomes, ["cancelled", "completed"]);
  assert.equal(env.replies.at(-1).text, "次の回答");
});

test("旧送信の遅れた停止失敗は次の送信の停止タイマーと状態表示を変更しない", async () => {
  const cancelFirst = deferred(), cancelSecond = deferred();
  let cancelCount = 0;
  const env = senderHarness({ send: () => new Promise(() => {}),
    cancel: () => (++cancelCount === 1 ? cancelFirst.promise : cancelSecond.promise) });
  const first = env.sender.submit();
  await turn();
  const cancellingFirst = env.sender.cancel();
  env.release();
  await first;
  const second = env.sender.submit();
  await turn();
  const cancellingSecond = env.sender.cancel();
  const count = env.messages.length;
  cancelFirst.reject(new Error("古い失敗"));
  await cancellingFirst;
  assert.equal(env.messages.length, count);
  env.release();
  cancelSecond.resolve();
  await cancellingSecond;
  await second;
  assert.equal(env.sender.busy, false);
});

test("停止後に返った通常回答は読み上げと選択肢と履歴と終了操作へ進まない", async () => {
  const result = deferred();
  const env = senderHarness({ send: () => result.promise, cancel: async () => {} });
  const pending = env.sender.submit();
  await turn();
  const previousStops = env.stopCount;
  await env.sender.cancel();
  assert.ok(env.stopCount > previousStops, "停止操作で先行読み上げも直ちに止める");
  result.resolve({ reply: "停止後の古い回答", choices: [{ label: "続ける" }], quitApp: true });
  await pending;
  assert.deepEqual(env.spoken, []);
  assert.deepEqual(env.choices, []);
  assert.equal(env.replies.filter(row => row.role === "assistant").length, 0);
  assert.deepEqual(env.outcomes, ["cancelled"]);
  assert.equal(env.timers.size, 0);
});

test("会話リセット中の通信が戻らなくても停止を要求し8秒後に新しい会話を送れる", async () => {
  let cancellations = 0;
  const env = senderHarness({ send: () => new Promise(() => {}), cancel: async () => { cancellations += 1; } });
  const pending = env.sender.submit();
  await turn();
  env.sender.reset();
  assert.equal(cancellations, 1);
  assert.equal(env.sender.hasLastRequest(), false);
  env.release();
  await pending;
  assert.equal(env.sender.busy, false);
  assert.deepEqual(env.replies, []);
  assert.deepEqual(env.outcomes, ["cancelled"]);
});

const document = installFakeDom();
await import("../../ui/composer-draft.js");
await import("../../ui/quick-actions.js");
const { createComposer } = await import("../../ui/chat/composer.js");

function composerHarness() {
  const readers = [];
  globalThis.FileReader = class { readAsDataURL() { readers.push(this); } };
  const node = tag => document.createElement(tag);
  const input = node("textarea"), form = node("form"), sendButton = node("button");
  const composer = createComposer({ form, input, sendButton, preview: node("div"), suggestions: node("div"), quickActions: node("div"), diagnostics: node("div"),
    status: { set() {}, clear() {}, flash() {} } });
  let submitted = 0;
  composer.onSubmit(() => { submitted += 1; });
  composer.setValue("資料について質問");
  return { composer, input, form, sendButton, readers, get submitted() { return submitted; } };
}

test("添付読み込み中はEnterでも送信せず全添付の確定後に送れる", async () => {
  const env = composerHarness();
  const first = env.composer.addFile({ name: "資料.txt", type: "text/plain" });
  const second = env.composer.addFile({ name: "画像.png", type: "image/png" });
  assert.equal(env.sendButton.disabled, true);
  assert.equal(env.composer.hasContent(), false);
  env.form.requestSubmit();
  assert.equal(env.submitted, 0);
  env.readers[0].onload({ target: { result: "資料データ" } });
  await first;
  assert.equal(env.sendButton.disabled, true);
  env.readers[1].onload({ target: { result: "画像データ" } });
  await second;
  assert.equal(env.sendButton.disabled, false);
  env.form.requestSubmit();
  assert.equal(env.submitted, 1);
  assert.equal(env.composer.files.length, 1);
  assert.equal(env.composer.images.length, 1);
});

test("添付読み込み失敗後は下書きを失わず送信を再開できる", async () => {
  const env = composerHarness();
  const pending = env.composer.addFile({ name: "壊れた資料.txt", type: "text/plain" });
  assert.equal(env.sendButton.disabled, true);
  env.readers[0].error = new Error("読込失敗");
  env.readers[0].onerror();
  await pending;
  assert.equal(env.sendButton.disabled, false);
  assert.equal(env.input.value, "資料について質問");
  assert.equal(env.composer.files.length, 0);
});

test("ネイティブ添付とFileReaderの待機は共通化し二重解除でも残りの添付を送らない", async () => {
  const env = composerHarness();
  const release = env.composer.beginAttachmentLoad();
  const loading = env.composer.addFile({ name: "資料.txt", type: "text/plain" });
  env.composer.addLoaded({ kind: "image", dataUrl: "ネイティブ画像" });
  release();
  release();
  assert.equal(env.sendButton.disabled, true);
  env.form.requestSubmit();
  assert.equal(env.submitted, 0);
  env.readers[0].onload({ target: { result: "本文" } });
  await loading;
  assert.equal(env.sendButton.disabled, false);
  assert.equal(env.composer.images.length, 1);
  assert.equal(env.composer.files.length, 1);
});
