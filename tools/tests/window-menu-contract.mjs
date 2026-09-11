import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("上端メニューのボタン操作をウィンドウドラッグが奪わない", async () => {
  const [index, renderer, menu, bridge, commands, styles, config] = await Promise.all([
    read("ui/index.html"),
    read("ui/renderer.js"),
    read("ui/top-menu.js"),
    read("ui/picoagent.js"),
    read("src-tauri/src/commands.rs"),
    read("ui/styles.css"),
    read("src-tauri/tauri.conf.json"),
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
  for (const id of ["windowMinimize", "windowMaximize", "windowClose"]) {
    assert.match(index, new RegExp(`id="${id}"`));
  }
  assert.match(menu, /windowMinimize[\s\S]*?minimizeWindow/);
  assert.match(menu, /windowMaximize[\s\S]*?toggleMaximizeWindow/);
  assert.match(menu, /windowClose[\s\S]*?closeWindow/);
  assert.match(bridge, /minimizeWindow[\s\S]*?getCurrentWindow\(\)\.minimize\(\)/);
  assert.match(styles, /\.top-window-menu\s*\{[\s\S]*?position:\s*fixed/);
  const menuStyle = styles.match(/\.top-window-menu\s*\{[^}]*\}/)?.[0] || "";
  assert.doesNotMatch(menuStyle, /translateY|opacity:\s*0/);
  assert.equal(JSON.parse(config).app.windows[0].maximizable, true);
});

test("会話リンクは既定ブラウザへ渡し、字幕は選択状態を残さない", async () => {
  const [renderer, bridge, commands, styles] = await Promise.all([
    read("ui/renderer.js"),
    read("ui/picoagent.js"),
    read("src-tauri/src/commands.rs"),
    read("ui/styles.css"),
  ]);
  assert.match(renderer, /function externalLink\(/);
  assert.match(renderer, /function externalLink\(url, text\)[\s\S]*?openExternalUrl\(url\)/);
  assert.doesNotMatch(renderer, /window\.open\(href/);
  assert.match(bridge, /openExternalUrl:\s*\(url\)\s*=>\s*call\("pet_open_external_url"/);
  assert.match(commands, /pub fn pet_open_external_url/);
  assert.match(styles, /\.reply\s*\{[\s\S]*?user-select:\s*none/);
});
