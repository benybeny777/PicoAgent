// fake-dom.mjs - 画面モジュールを Node で動かすための最小の DOM。
// 実ブラウザの再現ではなく、要素の生成・親子・クラス・属性・イベント登録だけを持つ。
// 見た目や配置の検査は tools/screenshots/ui-check.mjs（実ブラウザ）で行う。
import { readFileSync } from "node:fs";
import vm from "node:vm";

class FakeNode {
  constructor() {
    this.parentElement = null;
    this.children = [];
  }
  get nextSibling() {
    const siblings = this.parentElement?.children || [];
    return siblings[siblings.indexOf(this) + 1] || null;
  }
  get childNodes() { return this.children; }
  get firstChild() { return this.children[0] || null; }
  get lastChild() { return this.children[this.children.length - 1] || null; }
  remove() {
    if (!this.parentElement) return;
    const list = this.parentElement.children;
    list.splice(list.indexOf(this), 1);
    this.parentElement = null;
  }
}

class FakeText extends FakeNode {
  constructor(text) {
    super();
    this.data = String(text);
  }
  get textContent() { return this.data; }
}

class FakeElement extends FakeNode {
  constructor(tag) {
    super();
    this.tagName = String(tag).toUpperCase();
    this.attributes = new Map();
    this.dataset = {};
    this.style = { setProperty(name, value) { this[name] = value; }, removeProperty(name) { delete this[name]; } };
    this.listeners = {};
    this.hidden = false;
    this.className = "";
    if (["INPUT", "TEXTAREA", "SELECT"].includes(this.tagName)) {
      this.value = "";
      this.selectionStart = 0;
      this.selectionEnd = 0;
    }
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.clientHeight = 0;
    const element = this;
    this.classList = {
      add: (...names) => { const set = element.classSet(); names.forEach(name => set.add(name)); element.className = [...set].join(" "); },
      remove: (...names) => { const set = element.classSet(); names.forEach(name => set.delete(name)); element.className = [...set].join(" "); },
      toggle: (name, force) => {
        const on = force === undefined ? !element.classSet().has(name) : Boolean(force);
        if (on) element.classList.add(name); else element.classList.remove(name);
        return on;
      },
      contains: name => element.classSet().has(name),
    };
  }
  classSet() { return new Set(String(this.className).split(/\s+/).filter(Boolean)); }
  set textContent(value) {
    this.children.forEach(child => { child.parentElement = null; });
    this.children = value === "" || value == null ? [] : [new FakeText(value)];
  }
  get textContent() { return this.children.map(child => child.textContent).join(""); }
  append(...items) {
    for (const item of items) {
      const node = item instanceof FakeNode ? item : new FakeText(item);
      node.remove();
      node.parentElement = this;
      this.children.push(node);
    }
  }
  prepend(...items) {
    const nodes = items.map(item => (item instanceof FakeNode ? item : new FakeText(item)));
    nodes.forEach(node => { node.remove(); node.parentElement = this; });
    this.children.unshift(...nodes);
  }
  replaceChildren(...items) {
    this.children.forEach(child => { child.parentElement = null; });
    this.children = [];
    this.append(...items);
  }
  before(...items) {
    const parent = this.parentElement;
    if (!parent) return;
    for (const item of items) {
      item.remove();
      item.parentElement = parent;
      parent.children.splice(parent.children.indexOf(this), 0, item);
    }
  }
  after(...items) {
    const parent = this.parentElement;
    if (!parent) return;
    let anchor = this;
    for (const item of items) {
      item.remove();
      item.parentElement = parent;
      parent.children.splice(parent.children.indexOf(anchor) + 1, 0, item);
      anchor = item;
    }
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.has(name) ? this.attributes.get(name) : null; }
  removeAttribute(name) { this.attributes.delete(name); }
  hasAttribute(name) { return this.attributes.has(name); }
  addEventListener(type, listener) { (this.listeners[type] ||= []).push(listener); }
  removeEventListener(type, listener) { this.listeners[type] = (this.listeners[type] || []).filter(entry => entry !== listener); }
  dispatch(type, event = {}) { (this.listeners[type] || []).forEach(listener => listener({ preventDefault() {}, stopPropagation() {}, target: this, ...event })); }
  /** `.class` だけに対応する。 */
  querySelectorAll(selector) {
    const wanted = selector.replace(/^\./, "");
    const found = [];
    const walk = node => {
      for (const child of node.children) {
        if (child instanceof FakeElement) {
          if (child.classSet().has(wanted)) found.push(child);
          walk(child);
        }
      }
    };
    walk(this);
    return found;
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  getBoundingClientRect() { return { left: 0, top: 0, width: 0, height: 0, right: 0, bottom: 0 }; }
  focus() {}
  select() {}
  setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; }
  requestSubmit() { this.dispatch("submit"); }
  closest() { return null; }
}

/** globalThis に window / document / Node と、共通の道具（ui/common.js）を用意する。何度呼んでも同じ。 */
export function installFakeDom() {
  if (globalThis.document?.fake) return globalThis.document;
  const body = new FakeElement("body");
  // <script> を足したら読み込み完了として扱う（中身は実行しない。必要な window.* はテスト側で用意する）。
  const head = new FakeElement("head");
  const append = head.append.bind(head);
  head.append = (...items) => {
    append(...items);
    for (const item of items) if (item.tagName === "SCRIPT") setTimeout(() => item.onload?.(), 0);
  };
  const document = {
    fake: true,
    body,
    head,
    createElement: tag => new FakeElement(tag),
    createTextNode: text => new FakeText(text),
    addEventListener() {},
    getElementById: () => null,
    activeElement: null,
    hidden: false,
  };
  Object.assign(globalThis, { document, Node: FakeNode, Element: FakeElement });
  globalThis.window = globalThis;
  globalThis.requestAnimationFrame ||= callback => setTimeout(() => callback(performance.now()), 0);
  globalThis.cancelAnimationFrame ||= id => clearTimeout(id);
  globalThis.picoagent ||= new Proxy({}, { get: () => () => Promise.resolve({}) });
  if (!globalThis.localStorage) {
    const store = new Map();
    globalThis.localStorage = {
      getItem: key => (store.has(key) ? store.get(key) : null),
      setItem: (key, value) => store.set(key, String(value)),
      removeItem: key => store.delete(key),
    };
  }
  if (!globalThis.PicoUI) {
    vm.runInThisContext(readFileSync(new URL("../../../ui/common.js", import.meta.url), "utf8"));
  }
  return document;
}
