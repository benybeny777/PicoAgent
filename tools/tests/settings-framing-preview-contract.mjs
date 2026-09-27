import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");

test("旧構図エディタは保存せずメイン画面での調整を案内する", () => {
  const start = settings.indexOf("function buildCharacterFramingEditor()");
  const end = settings.indexOf("function diagnosticRepairButton(", start);
  const editor = settings.slice(start, end);
  assert.match(editor, /メイン画面/);
  assert.doesNotMatch(editor, /zoom|pointermove|characterFraming\[/);
  assert.doesNotMatch(settings, /patch\.PICOAGENT_CHARACTER_FRAMING\s*=/);
});

test("メイン画面の見た目をglassとpastelから選べる", () => {
  const html = readFileSync(new URL("../../ui/settings.html", import.meta.url), "utf8");
  const css = readFileSync(new URL("../../ui/settings.css", import.meta.url), "utf8");
  assert.match(settings, /selectField\("PICOAGENT_UI_THEME"/);
  assert.match(settings, /value: "glass"/);
  assert.match(settings, /value: "pastel"/);
  assert.match(settings, /applySettingsTheme\(v\.PICOAGENT_THEME \|\| "system", v\.PICOAGENT_UI_THEME \|\| "glass"\)/);
  assert.match(html, /themes\/glass\/tokens\.css/);
  assert.match(html, /themes\/pastel\/tokens\.css/);
  assert.match(css, /--pink: var\(--accent\)/);
});
