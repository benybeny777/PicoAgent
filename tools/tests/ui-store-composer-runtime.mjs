// 保存領域の故障を本番モジュールへ注入し、草稿・秘密情報と再試行の境界を固定する。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";

const source = name => readFileSync(new URL(`../../ui/${name}`, import.meta.url), "utf8");
const storeSource = source("app/ui-store.js").replace(/export /g, "");
function storage(initial = {}) {
  const values = new Map(Object.entries(initial));
  const calls = [];
  const errors = {};
  const api = Object.fromEntries(["getItem", "setItem", "removeItem"].map(operation => [operation, (key, value) => {
    calls.push({ operation, key, value });
    if (errors[operation]) throw errors[operation];
    if (operation === "getItem") return values.has(key) ? values.get(key) : null;
    if (operation === "setItem") values.set(key, value);
    else values.delete(key);
  }]));
  return { api, values, calls, errors };
}
function createStore(cache) {
  const context = { localStorage: cache.api };
  const api = runInNewContext(`${storeSource}; ({ read, readJson, write, remove })`, context);
  return { api, context };
}
const plain = value => JSON.parse(JSON.stringify(value));

test("保存領域の全操作は例外の秘密本文と内部キーを理由へ含めない", () => {
  const secret = "sk-secret-draft-private";
  for (const [operation, method, args] of [["getItem", "read", []], ["getItem", "readJson", [() => true]], ["setItem", "write", [secret]], ["removeItem", "remove", []]]) {
    for (const name of ["SecurityError", "QuotaExceededError", secret]) {
      const cache = storage({ secret });
      cache.errors[operation] = { name, message: secret, stack: secret };
      const { api } = createStore(cache);
      const result = api[method](secret, ...args);
      assert.equal(result.ok, false);
      assert.ok(result.reason.length > 0);
      assert.ok(!JSON.stringify(result).includes(secret));
      assert.equal(cache.values.get("secret"), secret);
    }
  }
  const { api, context } = createStore(storage());
  Object.defineProperty(context, "localStorage", { get() { throw new Error(secret); } });
  for (const result of [api.read(secret), api.readJson(secret, () => true), api.write(secret, secret), api.remove(secret)]) {
    assert.equal(result.ok, false);
    assert.ok(!JSON.stringify(result).includes(secret));
  }
});

test("不在は正常でjsonとschema破損を明示し原本は自動削除しない", () => {
  const cache = storage();
  const { api } = createStore(cache);
  assert.deepEqual(plain(api.readJson("key", () => { throw new Error("不在では検証しない"); })), { ok: true, value: null });
  for (const raw of ["", "{secret", "null", "[]", '{"draft":12}']) {
    cache.values.set("key", raw);
    const result = api.readJson("key", value => value && typeof value.draft === "string");
    assert.equal(result.ok, false);
    assert.equal(result.corrupted, true);
    assert.equal(cache.values.get("key"), raw);
    assert.ok(!result.reason.includes("secret"));
  }
  cache.values.set("key", '{"draft":"本文"}');
  assert.deepEqual(plain(api.readJson("key", value => typeof value.draft === "string")), { ok: true, value: { draft: "本文" } });
  const failure = api.readJson("key", () => { throw new Error("secret本文"); });
  assert.equal(failure.corrupted, true);
  assert.ok(!failure.reason.includes("secret本文"));
  assert.equal(cache.calls.filter(call => call.operation === "removeItem").length, 0);
});

function element(tag, properties = {}, children = []) {
  const listeners = new Map();
  const node = { tag, children: [...children], ...properties, style: {}, value: properties.value || "", scrollHeight: 44,
    classList: { add() {}, remove() {} }, dataset: {},
    addEventListener(type, callback) { listeners.set(type, callback); },
    fire(type, event = {}) { return listeners.get(type)?.(event); },
    replaceChildren(...next) { this.children = next; }, append(...next) { this.children.push(...next); },
    querySelectorAll(tagName) { return this.children.flatMap(child => typeof child === "object" ? [...(child.tag === tagName ? [child] : []), ...child.querySelectorAll(tagName)] : []); },
    focus() {}, blur() {}, setSelectionRange() {}, requestSubmit() { this.fire("submit", { preventDefault() {} }); },
  };
  return node;
}
function setup({ raw = null, error = null, presence = async () => ({}) } = {}) {
  const key = "picoagent.composerDraft.v1";
  const cache = storage(raw === null ? {} : { [key]: raw });
  if (error) cache.errors.getItem = error;
  const context = { localStorage: cache.api, Date, console, bridge: { configPresence: presence }, el: element,
    toast() {}, errorDetail: String, logApp() { throw new Error("保存値や例外全文をログへ渡してはいけない"); }, preview: String, isImeComposing: () => false,
    setTimeout: () => 1, clearTimeout() {}, reports: [],
    createBackgroundFailureReporter() { return { failure(...args) { context.reports.push(["failure", ...args]); }, recover(operation) { context.reports.push(["recover", operation]); } }; },
  };
  context.window = context;
  runInNewContext(source("composer-draft.js"), context);
  runInNewContext(source("quick-actions.js"), context);
  context.uiStore = runInNewContext(`${storeSource}; ({ read, readJson, write, remove })`, context);
  const createComposer = runInNewContext(`${source("chat/composer.js").replace(/^import .*;$/gm, "").replace(/export /g, "")}; createComposer`, context);
  const form = element("form", { hidden: true }), input = element("textarea"), quickActions = element("div");
  const diagnostics = element("div", {}, [quickActions]);
  form.append(diagnostics);
  const status = { text: "", set(text) { this.text = text; }, clear() { this.text = ""; }, flash(text) { this.text = text; } };
  const composer = createComposer({ form, input, quickActions, diagnostics, status, preview: element("div"), suggestions: element("div"), sendButton: element("button") });
  const button = text => form.querySelectorAll("button").find(node => node.textContent === text);
  const draftRaw = text => context.PicoComposerDraft.serializeDraft(text);
  return { composer, form, input, quickActions, status, cache, context, key, button, draftRaw };
}

test("初回と期限切れは異常にせず保存済み草稿は初期入力へ復元する", () => {
  const first = setup();
  assert.equal(first.input.value, "");
  assert.equal(first.context.reports.some(report => report[0] === "failure"), false);
  const expired = setup({ raw: JSON.stringify({ draft: "期限切れ", draftAt: Date.now() - 24 * 60 * 60 * 1000 }) });
  assert.equal(expired.input.value, "");
  assert.equal(expired.context.reports.some(report => report[0] === "failure"), false);
  assert.equal(expired.cache.calls.filter(call => call.operation === "removeItem").length, 0);
  const restored = setup({ raw: JSON.stringify({ draft: "復元する草稿", draftAt: Date.now() }) });
  assert.equal(restored.input.value, "復元する草稿");
});

test("草稿の読込不能は後続保存ゼロで再読込しても編集中の本文を上書きしない", () => {
  const state = setup({ error: { name: "SecurityError", message: "秘密本文" } });
  state.composer.setValue("いま編集中");
  state.composer.persistDraft();
  state.composer.close();
  assert.equal(state.cache.calls.filter(call => call.operation === "setItem").length, 0);
  state.cache.values.set(state.key, state.draftRaw("古い保存本文"));
  delete state.cache.errors.getItem;
  state.button("書きかけを再読込").fire("click");
  assert.equal(state.input.value, "いま編集中");
  state.composer.persistDraft();
  assert.equal(JSON.parse(state.cache.values.get(state.key)).draft, "いま編集中");
  assert.ok(!JSON.stringify(state.context.reports).includes("秘密本文"));
});

test("編集中に空へ戻した草稿も再読込で古い本文を戻さない", () => {
  const state = setup({ error: new Error("保存領域故障") });
  state.composer.setValue("入力");
  state.composer.setValue("");
  state.cache.values.set(state.key, state.draftRaw("古い本文"));
  delete state.cache.errors.getItem;
  state.button("書きかけを再読込").fire("click");
  assert.equal(state.input.value, "");
});

test("破損草稿は削除成功だけ通知し削除失敗では保存を止める", () => {
  const raw = "{秘密の壊れた本文";
  const state = setup({ raw, error: new Error("一時読込故障") });
  delete state.cache.errors.getItem;
  state.cache.errors.removeItem = { name: "SecurityError", message: "削除の秘密理由" };
  state.button("書きかけを再読込").fire("click");
  state.composer.setValue("編集中");
  state.composer.persistDraft();
  assert.equal(state.cache.values.get(state.key), raw);
  assert.equal(state.cache.calls.filter(call => call.operation === "setItem").length, 0);
  assert.ok(!state.status.text.includes("消しました"));
  delete state.cache.errors.removeItem;
  state.button("書きかけを再読込").fire("click");
  assert.equal(state.cache.values.has(state.key), false);
  assert.ok(state.status.text.includes("壊れていたため消しました"));
  assert.equal(state.input.value, "編集中");
  assert.ok(!JSON.stringify(state.context.reports).includes("秘密理由"));
});

test("明示破棄は削除成功で保存を再開し編集中の本文を保持する", () => {
  const state = setup({ error: new Error("読込不能") });
  state.composer.setValue("保持する本文");
  state.cache.errors.removeItem = new Error("削除不能");
  state.button("保存済みの書きかけを破棄").fire("click");
  state.composer.persistDraft();
  assert.equal(state.cache.calls.filter(call => call.operation === "setItem").length, 0);
  delete state.cache.errors.removeItem;
  state.button("保存済みの書きかけを破棄").fire("click");
  assert.equal(state.input.value, "保持する本文");
  state.composer.persistDraft();
  assert.equal(JSON.parse(state.cache.values.get(state.key)).draft, "保持する本文");
});

test("草稿の保存失敗は入力と保存原本を保持し成功で共通通知を復旧する", () => {
  const state = setup();
  state.cache.values.set(state.key, state.draftRaw("原本"));
  state.cache.errors.setItem = { name: "QuotaExceededError", message: "秘密" };
  state.composer.setValue("新しい本文");
  state.composer.persistDraft();
  assert.equal(JSON.parse(state.cache.values.get(state.key)).draft, "原本");
  assert.equal(state.input.value, "新しい本文");
  assert.ok(state.context.reports.some(report => report[0] === "failure" && report[1] === "write"));
  delete state.cache.errors.setItem;
  state.composer.persistDraft();
  assert.ok(state.context.reports.some(report => report[0] === "recover" && report[1] === "write"));
  assert.equal(JSON.parse(state.cache.values.get(state.key)).draft, "新しい本文");
});

const textOf = node => [node.textContent || "", ...node.children.map(child => typeof child === "object" ? textOf(child) : String(child))].join(" ");
test("クイック操作の確認失敗は縮小一覧を出さず理由と再試行を示す", async () => {
  let calls = 0;
  const state = setup({ presence: async keys => {
    calls += 1;
    if (calls === 1) throw new Error("sk-secret-config");
    return Object.fromEntries(keys.map(key => [key, true]));
  } });
  assert.ok(textOf(state.quickActions).includes("確認中"));
  assert.equal(state.quickActions.dataset.state, "loading");
  await state.composer.refreshDefaultQuickActions();
  assert.ok(textOf(state.quickActions).includes("準備できません"));
  assert.equal(state.quickActions.dataset.state, "error");
  assert.deepEqual(state.quickActions.querySelectorAll("button").map(button => button.textContent), ["再試行"]);
  assert.ok(!JSON.stringify(state.context.reports).includes("sk-secret-config"));
  await state.quickActions.querySelectorAll("button")[0].fire("click");
  assert.equal(state.quickActions.querySelectorAll("button").length, 8);
  assert.equal(state.quickActions.dataset.state, "ready");
  assert.ok(state.context.reports.some(report => report[0] === "recover" && report[1] === "quick-actions"));
});

test("古いクイック操作の取得結果や不正な存在情報で最新表示を上書きしない", async () => {
  let complete;
  const state = setup({ presence: keys => new Promise(resolve => { complete = value => resolve(value || Object.fromEntries(keys.map(key => [key, true]))); }) });
  const first = state.composer.refreshDefaultQuickActions();
  const firstComplete = complete;
  const second = state.composer.refreshDefaultQuickActions();
  complete({ PICOAGENT_SECRET: "秘密値" });
  await second;
  firstComplete();
  await first;
  assert.deepEqual(state.quickActions.querySelectorAll("button").map(button => button.textContent), ["再試行"]);
  assert.ok(!JSON.stringify(state.context.reports).includes("秘密値"));
});

test("質問の選択肢は確認失敗中も保持し通常一覧と異なる状態で送信する", async () => {
  const state = setup({ presence: async () => { throw new Error("秘密"); } });
  await state.composer.refreshDefaultQuickActions();
  state.composer.setChoices([{ label: "詳しい説明を希望する選択肢", description: "説明" }]);
  assert.equal(state.quickActions.dataset.state, "choices");
  assert.equal(state.quickActions.querySelectorAll("button")[0].textContent, "詳しい説明を希望する選択肢");
  await state.quickActions.querySelectorAll("button")[0].fire("click");
  assert.equal(state.input.value, "詳しい説明を希望する選択肢");
  state.composer.setChoices(null);
  assert.equal(state.quickActions.dataset.state, "error");
});
