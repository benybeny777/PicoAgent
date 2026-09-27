import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("上端メニューのボタン操作をウィンドウドラッグが奪わない", async () => {
  const [menu, appBridge, bridge, commands, styles, config, capabilities] = await Promise.all([
    read("ui/app/window-chrome.js"),
    read("ui/app/bridge.js"),
    read("ui/picoagent.js"),
    read("src-tauri/src/commands.rs"),
    read("ui/components/window.css"),
    read("src-tauri/tauri.conf.json"),
    read("src-tauri/capabilities/default.json"),
  ]);
  // ボタン・入力欄の上ではウィンドウ移動を始めない（押したボタンが効かなくなる）。
  const drag = menu.slice(menu.indexOf("function beginWindowDrag(event)"), menu.indexOf("/** 押した位置に"));
  assert.match(drag, /if \(event\.target instanceof Element && event\.target\.closest\("button,input,textarea,select,a,\[contenteditable\]"\)\) return;/);
  assert.match(menu, /menu\.addEventListener\("pointerdown", beginWindowDrag\);/);
  // 設定は二度押しで二重に開かない。
  assert.match(menu, /settingsButton\.disabled = true;\s*\n\s*try \{ await bridge\.openSettings\(\); \}/);
  assert.match(menu, /finally \{ settingsButton\.disabled = false; \}/);
  assert.match(commands, /pub async fn pet_open_settings[\s\S]*?spawn_blocking\(move \|\| open_settings_tab\(&app, tab\.as_deref\(\)\)\)\s*\.await/);
  assert.match(commands, /fn open_settings_tab\([^)]*\)[^{]*-> Result<\(\), String>/);
  assert.match(commands, /window\.additional_browser_args\.as_deref\(\)/,
    "メインと設定でWebView2の起動オプションを一致させる");
  assert.match(commands, /builder = builder\.additional_browser_args\(args\)/);
  const newSettings = commands.slice(commands.indexOf('let path = tab'), commands.indexOf('const SETTINGS_WINDOW_WIDTH'));
  assert.doesNotMatch(newSettings, /\.center\(\)/,
    "WebView初期化前の中央化を行わない");
  assert.doesNotMatch(commands, /settings\.outer_size\(\)/,
    "設定画面の初期化前に設定WebViewへ寸法を問い合わせない");
  const existingSettings = commands.slice(commands.indexOf('if let Some(win) = app.get_webview_window("settings")'), commands.indexOf('let path = tab'));
  assert.ok(existingSettings.indexOf("win.show()") < existingSettings.indexOf("center_settings_on_main_monitor"),
    "既存の設定画面は表示を成立させてから右画面へ配置する");
  const windowPermissions = JSON.parse(capabilities).permissions;
  for (const operation of ["close", "minimize", "unminimize", "toggle-maximize"]) {
    assert.ok(windowPermissions.includes(`core:window:allow-${operation}`), `${operation} の権限がありません`);
  }
  assert.match(menu, /\[minimize, bridge\.minimizeWindow, "最小化"\], \[maximize, bridge\.toggleMaximizeWindow, "最大化の切り替え"\], \[close, bridge\.closeWindow, "閉じる"\]/);
  assert.match(appBridge, /export const minimizeWindow = \(\) => api\.minimizeWindow\(\);/);
  assert.match(bridge, /minimizeWindow[\s\S]*?getCurrentWindow\(\)\.minimize\(\)/);
  assert.match(styles, /\.top-window-menu\s*\{[\s\S]*?position:\s*fixed/);
  assert.match(styles, /\.app\.is-booting \.stage,\s*\n\.app\.is-booting \.top-window-menu-items \{ visibility: hidden; \}/,
    "起動中は上端メニューから画面状態を変更できない");
  const menuStyle = styles.match(/\.top-window-menu\s*\{[^}]*\}/)?.[0] || "";
  assert.doesNotMatch(menuStyle, /translateY|opacity:\s*0/);
  assert.equal(JSON.parse(config).app.windows[0].maximizable, true);
});

test("会話リンクは既定ブラウザへ渡す", async () => {
  const [markdown, bridge, commands] = await Promise.all([
    read("ui/chat/markdown.js"),
    read("ui/picoagent.js"),
    read("src-tauri/src/commands.rs"),
  ]);
  assert.match(markdown, /function externalLink\(url, text\)[\s\S]*?openExternalUrl\(url\)/);
  assert.doesNotMatch(markdown, /window\.open\(/);
  assert.match(bridge, /openExternalUrl:\s*\(url\)\s*=>\s*call\("pet_open_external_url"/);
  assert.match(commands, /pub fn pet_open_external_url/);
});
