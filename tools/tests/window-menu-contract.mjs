import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("上端メニューのボタン操作をウィンドウドラッグが奪わない", async () => {
  const [renderer, menu, commands] = await Promise.all([
    read("ui/renderer.js"),
    read("ui/top-menu.js"),
    read("src-tauri/src/commands.rs"),
  ]);
  assert.match(
    renderer,
    /topWindowMenu\?\.addEventListener\("pointerdown", event => \{\s*if \(event\.target instanceof Element && event\.target\.closest\("button,input,select,textarea,a,\[contenteditable\]"\)\) return;\s*beginWindowDrag\(event\);/,
  );
  assert.doesNotMatch(renderer, /topWindowMenu\?\.querySelectorAll\("\[data-settings-tab\]"\)/);
  assert.match(menu, /menu\.querySelectorAll\("\[data-settings-tab\]"\)/);
  assert.match(menu, /if \(button\.disabled\) return;[\s\S]*?button\.disabled = true;/);
  assert.match(menu, /await window\.picoagent\.openSettings\(tab\)/);
  assert.match(menu, /finally \{\s*button\.disabled = false;/);
  assert.match(commands, /pub fn pet_open_settings[\s\S]*?open_settings_tab\(&app, tab\.as_deref\(\)\);\s*Ok\(\(\)\)/);
});
