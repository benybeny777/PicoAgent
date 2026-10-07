import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { installFakeDom } from "./lib/fake-dom.mjs";
import { uiFile } from "./lib/ui-source.mjs";

const dom = installFakeDom();
const { el } = await import("../../ui/app/util.js");

function load(file, name, dependencies) {
  const context = vm.createContext(dependencies);
  vm.runInContext(uiFile(file).replace(/^import .*;\r?$/gm, "").replace("export function", "function") + `\nthis.factory = ${name};`, context);
  return context.factory;
}

function contextMenu({ copy = async () => true, paste = async () => "貼り付け" } = {}) {
  const listeners = {};
  const alerts = [];
  const field = dom.createElement("textarea");
  field.value = "元の入力";
  field.selectionStart = 0;
  field.selectionEnd = 2;
  field.closest = selector => selector.startsWith("textarea") ? field : null;
  field.setRangeText = (text, start, end) => { field.value = field.value.slice(0, start) + text + field.value.slice(end); };
  field.dispatchEvent = () => {};
  const factory = load("app/context-menu.js", "createContextMenu", {
    document: { addEventListener: (name, callback) => { listeners[name] = callback; } },
    window: {}, Element: field.constructor, Event: class {},
    navigator: { clipboard: { readText: paste } },
    bridge: { menu: async () => {}, on() {} },
    copyText: copy, toast: text => alerts.push(text), logApp() {},
    errorDetail: String, errorMessage: String, preview: value => value,
  });
  const menu = factory({ onCharacterAction() {}, openExternalUrl() {} });
  listeners.contextmenu({ target: field, preventDefault() {}, clientX: 0, clientY: 0 });
  return { field, alerts, perform: menu.perform };
}

for (const mode of ["false", "throw"]) {
  test(`クリップボードへのコピーが${mode}で失敗しても切り取りは入力を消さない`, async () => {
    const fixture = contextMenu({ copy: async () => {
      if (mode === "throw") throw new Error("クリップボード使用不可");
      return false;
    } });
    await fixture.perform({ action: "cut" });
    assert.equal(fixture.field.value, "元の入力");
    assert.equal(fixture.alerts.length, 1);
  });
}

test("コピーに成功したときだけ元の選択範囲を切り取る", async () => {
  const fixture = contextMenu();
  await fixture.perform({ action: "cut" });
  assert.equal(fixture.field.value, "入力");
});

for (const action of ["cut", "paste"]) {
  test(`${action}のクリップボード待機中に変わった入力を上書きしない`, async () => {
    let resolve;
    const pending = new Promise(done => { resolve = done; });
    const fixture = contextMenu({ copy: () => pending, paste: () => pending });
    const operation = fixture.perform({ action });
    fixture.field.value = "待っている間の新しい入力";
    resolve(action === "cut" ? true : "貼り付け");
    await operation;
    assert.equal(fixture.field.value, "待っている間の新しい入力");
    assert.match(fixture.alerts[0], /入力が変わった/);
  });
}

test("上端メニューとウィンドウ操作の失敗は理由を画面へ出して再操作できる", async () => {
  const alerts = [];
  const reject = async () => { throw new Error("検査用の操作拒否"); };
  const menu = dom.createElement("div");
  const factory = load("app/window-chrome.js", "createWindowChrome", {
    el, Element: menu.constructor, toast: text => alerts.push(text), logApp() {},
    ResizeObserver: class { observe() {} disconnect() {} },
    MutationObserver: class { observe() {} disconnect() {} },
    window: { addEventListener() {} },
    document: { addEventListener() {}, documentElement: { addEventListener() {} } },
    errorDetail: String, errorMessage: error => error.message, preview: value => value,
    bridge: { layoutMenu: reject, characterMenu: reject, openSettings: reject,
      minimizeWindow: reject, toggleMaximizeWindow: reject, closeWindow: reject, startWindowResize: reject },
  });
  const appElement = { querySelector: () => null, classList: { add() {}, remove() {}, toggle() {}, contains: () => false } };
  factory({ menu, appElement, stageElement: dom.createElement("div"), onEditLayout() {}, onGame() {}, listCharacters: async () => [] });
  const items = menu.children[1].children;
  // 📌（常に表示）は失敗しうる外部操作を持たないので、失敗通知の対象から外す。
  const controls = [...menu.children[3].children].filter(button => button.className !== "window-pin");
  for (const button of [items[0], items[2], items[4], ...controls]) {
    const previous = alerts.length;
    await button.listeners.click[0]({ preventDefault() {}, stopPropagation() {} });
    await Promise.resolve();
    assert.equal(alerts.length, previous + 1);
    assert.equal(Boolean(button.disabled), false);
    assert.match(alerts.at(-1), /検査用の操作拒否|一覧を取得できません/);
  }
});
