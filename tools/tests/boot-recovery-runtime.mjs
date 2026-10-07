// 起動完了と画面表示を別々に失敗させ、実際の再試行と後始末を確かめる。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { runInNewContext } from "node:vm";
import test from "node:test";

const source = (await readFile(new URL("../../ui/app/boot.js", import.meta.url), "utf8"))
  .replace(/^import .*;\r?\n/gm, "")
  .replace("export function createBoot", "function createBoot");
const statusSource = (await readFile(new URL("../../ui/app/status.js", import.meta.url), "utf8"))
  .replace("export function createStatus", "function createStatus");

function setup({ finishFailures = 0, revealFailures = 0 } = {}) {
  const timers = new Map();
  const unload = [];
  const calls = { finish: 0, reveal: 0, capture: 0 };
  let timerId = 0;
  let booting = true;
  const bridge = {
    bootProgress: async () => {},
    finishBoot: async () => {
      if (calls.finish++ < finishFailures) throw new Error("位置復元の失敗");
    },
    revealMain: async () => {
      if (calls.reveal++ < revealFailures) throw new Error("表示の失敗");
    },
  };
  const createBoot = runInNewContext(`${source}\ncreateBoot`, {
    bridge,
    el: () => ({ children: [], append() {} }),
    errorDetail: String,
    logApp() {},
    setInterval: () => ++timerId,
    clearInterval() {},
    setTimeout: (callback, delay) => { const id = ++timerId; timers.set(id, { callback, delay }); return id; },
    clearTimeout: id => timers.delete(id),
    window: { addEventListener: (_, callback) => unload.push(callback) },
  });
  const boot = createBoot({
    app: { classList: { contains: () => booting, remove: () => { booting = false; } } },
    screen: { replaceChildren() {} },
    beforeReveal: () => calls.capture++,
  });
  const createStatus = runInNewContext(`${statusSource}\ncreateStatus`, { clearTimeout() {} });
  const status = createStatus({ textContent: "", classList: { toggle() {} } });
  return {
    boot, calls, timers, unload,
    status,
    async retry() {
      assert.equal(timers.size, 1, "再試行は常に1本だけ予約する");
      const [id, timer] = [...timers][0];
      timers.delete(id);
      await timer.callback();
      return timer.delay;
    },
  };
}

test("位置復元成功後に表示が失敗しても表示だけ再試行し描画を確定する", async () => {
  const state = setup({ revealFailures: 2 });
  await state.boot.release(state.status);
  assert.equal(state.boot.booting, false);
  assert.equal(state.calls.finish, 1);
  assert.equal(state.calls.reveal, 1);
  assert.equal(await state.retry(), 1500);
  assert.equal(await state.retry(), 3000);
  assert.deepEqual(state.calls, { finish: 1, reveal: 3, capture: 3 });
  assert.equal(state.timers.size, 0);
  assert.equal(state.status.text, "");
});

test("位置復元と表示が順に失敗しても両工程を復旧する", async () => {
  const state = setup({ finishFailures: 1, revealFailures: 1 });
  await state.boot.release(state.status);
  assert.deepEqual(state.calls, { finish: 1, reveal: 0, capture: 0 });
  await state.retry();
  await state.retry();
  assert.deepEqual(state.calls, { finish: 2, reveal: 2, capture: 2 });
  assert.equal(state.timers.size, 0);
});

test("表示の復旧待ちで再度解放しても予約を重複せず終了時に回収する", async () => {
  const state = setup({ revealFailures: 5 });
  await state.boot.release(state.status);
  await state.boot.release(state.status);
  assert.equal(state.calls.finish, 1);
  assert.equal(state.timers.size, 1);
  state.unload.forEach(callback => callback());
  assert.equal(state.timers.size, 0);
});

test("表示復旧中に追加された常設エラーを復旧成功後も残す", async () => {
  const state = setup({ revealFailures: 1 });
  await state.boot.release(state.status);
  assert.equal(state.status.owner, "boot-recovery");
  state.status.setSticky("音声モデルの準備に失敗しました", "error");
  await state.retry();
  assert.equal(state.status.text, "音声モデルの準備に失敗しました");
});

test("既存の常設エラーを復旧進捗で隠さない", async () => {
  const state = setup({ revealFailures: 1 });
  state.status.setSticky("ローカルAIの準備に失敗しました", "error");
  await state.boot.release(state.status);
  assert.equal(state.status.text, "ローカルAIの準備に失敗しました");
  await state.retry();
  assert.equal(state.status.text, "ローカルAIの準備に失敗しました");
});

test("復旧待ちで出た新しい操作状態を再試行と成功のどちらでも上書きしない", async () => {
  const state = setup({ revealFailures: 2 });
  await state.boot.release(state.status);
  state.status.set("読み上げ中…", "voice");
  await state.retry();
  assert.equal(state.status.owner, "voice");
  assert.equal(state.status.text, "読み上げ中…");
  await state.retry();
  assert.equal(state.status.owner, "voice");
  assert.equal(state.status.text, "読み上げ中…");
});
