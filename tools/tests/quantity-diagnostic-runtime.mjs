import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
const source = readFileSync(new URL("../../ui/chat/panel.js", import.meta.url), "utf8");
function node() {
  const classes = new Set();
  return { children: [], hidden: false, handlers: {}, append(...children) { this.children.push(...children); }, addEventListener(name, handler) { this.handlers[name] = handler; }, classList: { toggle(name, value) { value ? classes.add(name) : classes.delete(name); }, contains: name => classes.has(name) } };
}
function fixture() {
  const element = node(), reply = node(), body = node(); element.append(body);
  const applied = [], opened = [];
  const context = vm.createContext({ element, reply, el: (_tag, props) => Object.assign(node(), props), context: { openSettings: tab => opened.push(tab) }, fontSize: { setSize() {} }, applyFontPx: value => applied.push(value), applyStreamFontPx: value => applied.push(value) });
  const state = source.slice(source.indexOf('  let variant = "bubble";'), source.indexOf('  let lastAssistantText ='));
  const methods = source.slice(source.indexOf('    setFontPx(px) {'), source.indexOf('    update({ options, expanded }) {'));
  vm.runInContext(`${state}\nthis.chat={${methods}};this.changeVariant=value=>{variant=value;refreshFontDiagnostic()};this.diagnostic=fontDiagnostic;this.button=fontSettings;`, context);
  return { context, element, reply, body, applied, opened };
}
test("文字サイズ不正は該当表示と入力を診断へ止め設定導線を残し正常保存で戻す", () => {
  const f = fixture();
  f.context.chat.setFontError("bubble", "文字サイズが範囲外です");
  assert.equal(f.context.diagnostic.hidden, false);
  assert.equal(f.body.inert, true);
  assert.equal(f.reply.classList.contains("quantity-hidden"), true);
  assert.equal(f.context.diagnostic.children[0].textContent, "文字サイズが範囲外です");
  f.context.button.handlers.click(); assert.deepEqual(f.opened, ["display"]);
  f.context.changeVariant("column"); assert.equal(f.context.diagnostic.hidden, true);
  f.context.chat.setFontError("column", "配信文字が不正です"); assert.equal(f.body.inert, true);
  f.context.chat.setFontError("column", null); f.context.chat.setStreamFontPx(31);
  assert.equal(f.body.inert, false); assert.deepEqual(f.applied, [31]);
  assert.throws(() => f.context.chat.setStreamFontPx("不正"), /整数/);
  assert.deepEqual(f.applied, [31]);
});

test("接続確認の失敗は状態不明と理由を保持し正常な再確認で復旧する", () => {
  const engine = readFileSync(new URL("../../ui/app/engine.js", import.meta.url), "utf8").replace(/^import .*;\r?$/gm, "").replace("export function", "function");
  const statuses = [], online = [];
  const context = vm.createContext({ status: { setSticky: value => statuses.push(value) }, onChange() {} });
  vm.runInContext(`${engine}\nthis.engine=createEngine({status,onChange});`, context);
  context.engine.init({ providerId: "cloud", up: null, providerError: "ポート値が不正です" }, value => online.push(value));
  assert.match(context.engine.blockedReason(), /接続状態不明.*ポート値/);
  assert.deepEqual(online, [null]);
  context.engine.applyProviderStatus({ up: true, providerError: null });
  assert.equal(context.engine.blockedReason(), "");
  assert.deepEqual(online, [null, true]);
  assert.equal(statuses.at(-1), "");
});

test("会話欄の文字操作は共通宣言の範囲と標準値だけを使い保存値を丸めない", () => {
  const control = readFileSync(new URL("../../ui/chat/font-size.js", import.meta.url), "utf8").replace(/^import .*;\r?$/gm, "").replace(/export function/g, "function");
  const panel = node(), applied = [];
  const context = vm.createContext({ panel, document: node(), el: (_tag, props) => Object.assign(node(), props), apply: (...args) => applied.push(args), setTimeout: () => 1, clearTimeout() {} });
  vm.runInContext(`${control}\nthis.control=createFontSizeControl({panel,variant:()=>"bubble",apply});`, context);
  context.control.setNumberSettings({ PICOAGENT_BUBBLE_FONT_PX: { range: ["10", "28"], default: "13" }, PICOAGENT_STREAM_CHAT_FONT_PX: { range: ["18", "48"], default: "22" } });
  context.control.setSize("bubble", 14);
  panel.handlers.keydown({ ctrlKey: true, key: "+", preventDefault() {} });
  assert.deepEqual(applied, [["bubble", 15]]);
  panel.handlers.keydown({ ctrlKey: true, key: "0", preventDefault() {} });
  assert.deepEqual(applied.at(-1), ["bubble", 13]);
  assert.throws(() => context.control.setSize("bubble", 14.5), /宣言範囲/);
  assert.throws(() => context.control.setSize("bubble", 29), /宣言範囲/);
});
