// 診断の実モジュールを評価し、遅い操作結果が最新のキャラや診断を上書きしないことを確認する。
import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

function element(tag, props = {}, children = [], attributes = {}) {
  return {
    tag, hidden: false, disabled: false, textContent: "", children: [...children],
    attributes: { ...attributes }, listeners: {}, ...props,
    append(child) { this.children.push(child); },
    addEventListener(name, callback) { this.listeners[name] = callback; },
    setAttribute(name, value) { this.attributes[name] = value; },
    removeAttribute(name) { delete this.attributes[name]; },
    remove() { this.removed = true; },
    click() { assert.equal(this.disabled, false, "操作を再試行できません"); return this.listeners.click(); },
  };
}

function harness({ retry = async () => {}, useImage = async () => {} } = {}) {
  const context = vm.createContext({ el: element, errorMessage: error => error.message || String(error) });
  const source = uiFile("app/character-diagnostic.js")
    .replace(/^import .*;\r?$/gm, "")
    .replace("export function createCharacterDiagnostic", "function createCharacterDiagnostic");
  vm.runInContext(`${source}\nthis.factory = createCharacterDiagnostic;`, context);
  const container = element("div");
  const diagnostic = context.factory({ container, retry, useImage });
  const panel = container.children[0];
  const [title, reason, actions] = panel.children;
  const [retryButton, imageButton] = actions.children;
  return { diagnostic, panel, title, reason, retryButton, imageButton };
}

function deferred() {
  let reject;
  const promise = new Promise((_resolve, fail) => { reject = fail; });
  return { promise, reject };
}

test("診断は対象IDと理由を表示し画像非対応では2D操作を隠す", () => {
  const env = harness();
  env.diagnostic.show("追加キャラ", new Error("Live2Dのモデルを読めません"));
  assert.equal(env.panel.hidden, false);
  assert.equal(env.title.textContent, "追加キャラを表示できません");
  assert.equal(env.reason.textContent, "Live2Dのモデルを読めません");
  assert.equal(env.imageButton.hidden, true);
  assert.equal(env.retryButton.disabled, false);
});

test("再試行は診断中のキャラIDを渡し操作完了後にボタンを戻す", async () => {
  const called = [];
  const env = harness({ retry: async id => { called.push(id); } });
  env.diagnostic.show("試行対象", new Error("読込失敗"));
  const running = env.retryButton.click();
  assert.equal(env.retryButton.disabled, true);
  assert.equal(env.panel.attributes["aria-busy"], "true");
  await running;
  assert.deepEqual(called, ["試行対象"]);
  assert.equal(env.retryButton.disabled, false);
  assert.equal(env.panel.attributes["aria-busy"], undefined);
});

test("2D保存の失敗理由を残し同じキャラで再試行できる", async () => {
  const called = [];
  const env = harness({ useImage: async id => {
    called.push(id);
    if (called.length === 1) throw new Error("描画の選択を保存できません");
  } });
  env.diagnostic.show("2D対象", Object.assign(new Error("WebGLの初期化失敗"), { imageAvailable: true }));
  assert.equal(env.imageButton.hidden, false);
  await env.imageButton.click();
  assert.equal(env.panel.hidden, false);
  assert.equal(env.reason.textContent, "描画の選択を保存できません");
  assert.equal(env.imageButton.disabled, false);
  assert.equal(env.retryButton.disabled, false);
  await env.imageButton.click();
  assert.deepEqual(called, ["2D対象", "2D対象"]);
});

test("古い2D操作の拒否は新しい診断の理由と操作を上書きしない", async () => {
  const old = deferred();
  const called = [];
  const env = harness({ useImage: id => { called.push(id); return old.promise; } });
  env.diagnostic.show("旧キャラ", Object.assign(new Error("旧理由"), { imageAvailable: true }));
  const running = env.imageButton.click();
  env.diagnostic.show("新キャラ", new Error("新キャラのモデル欠損"));
  old.reject(new Error("旧キャラの保存失敗"));
  await running;
  assert.deepEqual(called, ["旧キャラ"]);
  assert.equal(env.title.textContent, "新キャラを表示できません");
  assert.equal(env.reason.textContent, "新キャラのモデル欠損");
  assert.equal(env.imageButton.hidden, true);
  assert.equal(env.retryButton.disabled, false);
});

test("古い操作の完了は最新の実行中操作を解除しない", async () => {
  const old = deferred(), latest = deferred();
  const env = harness({ retry: id => id === "旧" ? old.promise : latest.promise });
  env.diagnostic.show("旧", new Error("旧理由"));
  const oldRun = env.retryButton.click();
  env.diagnostic.show("新", new Error("新理由"));
  const latestRun = env.retryButton.click();
  old.reject(new Error("旧操作失敗"));
  await oldRun;
  assert.equal(env.reason.textContent, "新理由");
  assert.equal(env.retryButton.disabled, true);
  assert.equal(env.panel.attributes["aria-busy"], "true");
  latest.reject(new Error("最新の再試行失敗"));
  await latestRun;
  assert.equal(env.reason.textContent, "最新の再試行失敗");
  assert.equal(env.retryButton.disabled, false);
});

test("診断を消した後の古い拒否は再表示も理由の変更もしない", async () => {
  const pending = deferred();
  const env = harness({ retry: () => pending.promise });
  env.diagnostic.show("対象", new Error("表示中の理由"));
  const running = env.retryButton.click();
  env.diagnostic.clear();
  pending.reject(new Error("消した後の失敗"));
  await running;
  assert.equal(env.panel.hidden, true);
  assert.equal(env.reason.textContent, "表示中の理由");
});
