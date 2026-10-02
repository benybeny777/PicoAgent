import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");

test("旧構図エディタは保存せずメイン画面での調整を案内する", () => {
  const start = settings.indexOf("function buildCharacterFramingEditor()");
  const end = settings.indexOf("function diagnosticRepairButton(", start);
  const editor = settings.slice(start, end);
  assert.match(editor, /メイン画面/);
  assert.doesNotMatch(editor, /zoom|pointermove|characterFraming\[/);
  assert.doesNotMatch(settings, /patch\.PICOAGENT_CHARACTER_FRAMING\s*=/);
});

test("UIモードは可愛いを既定に既存のpastelとglassで選べる", () => {
  const html = readFileSync(new URL("../../ui/settings.html", import.meta.url), "utf8");
  const css = readFileSync(new URL("../../ui/settings.css", import.meta.url), "utf8");
  assert.match(settings, /selectField\("PICOAGENT_UI_THEME"/);
  assert.match(settings, /selectField\("PICOAGENT_UI_THEME", "UIモード", v\.PICOAGENT_UI_THEME \|\| "pastel"/);
  assert.match(settings, /value: "glass", label: "かっこいい（シアン色・引き締まった形）"/);
  assert.match(settings, /value: "pastel", label: "可愛い（ローズ色・丸み）"/);
  assert.match(settings, /applySettingsTheme\(v\.PICOAGENT_THEME \|\| "system", v\.PICOAGENT_UI_THEME \|\| "pastel"\)/);
  assert.match(html, /data-ui-theme="pastel"/);
  assert.match(html, /themes\/glass\/tokens\.css/);
  assert.match(html, /themes\/pastel\/tokens\.css/);
  assert.match(css, /--pink: var\(--accent\)/);
});

test("設定のUIモードは未知値を可愛いへ戻し保存済みのかっこいいを尊重する", () => {
  const root = { dataset: {}, removeAttribute(name) { if (name === "data-theme") delete this.dataset.theme; } };
  const context = vm.createContext({ document: { documentElement: root } });
  const start = settings.indexOf("function applySettingsTheme(");
  vm.runInContext(settings.slice(start, settings.indexOf("function section(", start)), context);
  context.applySettingsTheme("system");
  assert.equal(root.dataset.uiTheme, "pastel");
  assert.equal(root.dataset.theme, undefined);
  context.applySettingsTheme("system", "glass");
  assert.equal(root.dataset.uiTheme, "glass");
  assert.equal(root.dataset.theme, "dark");
  context.applySettingsTheme("light");
  assert.equal(root.dataset.uiTheme, "glass");
  assert.equal(root.dataset.theme, "light");
  context.applySettingsTheme("system", "unknown");
  assert.equal(root.dataset.uiTheme, "pastel");
  assert.equal(root.dataset.theme, undefined);
  context.applySettingsTheme("dark", "pastel");
  assert.equal(root.dataset.theme, "dark");
});

test("メイン画面とバックエンドの未設定既定を可愛いへ揃える", () => {
  const html = readFileSync(new URL("../../ui/index.html", import.meta.url), "utf8");
  const main = readFileSync(new URL("../../ui/app/main.js", import.meta.url), "utf8");
  const commands = readFileSync(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
  assert.match(html, /data-ui-theme="pastel"/);
  assert.equal(commands.match(/"uiTheme": state\.config\.get\("PICOAGENT_UI_THEME", "pastel"\)/g)?.length, 2);
  assert.doesNotMatch(commands, /config\.get\("PICOAGENT_UI_THEME", "glass"\)/);
  const root = { dataset: { uiTheme: "pastel" } };
  const context = vm.createContext({ document: { documentElement: root } });
  const start = main.indexOf("const UI_THEMES =");
  vm.runInContext(main.slice(start, main.indexOf("function applyStreamAppearance(", start)), context);
  context.applyTheme("system", "glass");
  assert.equal(root.dataset.uiTheme, "glass");
  context.applyTheme("light", "pastel");
  assert.equal(root.dataset.uiTheme, "pastel");
  assert.equal(root.dataset.theme, "light");
  context.applyTheme(null, "unknown");
  assert.equal(root.dataset.uiTheme, "pastel");
  assert.equal(root.dataset.theme, "light");
});
