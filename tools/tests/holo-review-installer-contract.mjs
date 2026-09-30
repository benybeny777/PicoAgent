// 審査版の削除範囲と初回導線を、通常版の設定から独立したまま保つ。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const read = path => readFile(new URL(path, root), "utf8");
const [builder, liteHook, fullHook, config] = await Promise.all([
  read("scripts/build-holo-indie-review-installer.ps1"),
  read("src-tauri/installer-hooks-lite.nsh"),
  read("src-tauri/installer-hooks.nsh"),
  read("src-tauri/tauri.conf.json"),
]);

test("審査版は初回案内を飛ばし、日本語のインストーラで起動する", () => {
  assert.match(builder, /PICOAGENT_ONBOARDING_DONE\s*=\s*\$true/);
  assert.match(builder, /languages = @\('Japanese'\)/);
  assert.deepEqual(JSON.parse(config).bundle.windows.nsis.languages, ["Japanese"]);
  const hook = builder.slice(builder.indexOf("$hook = [System.Collections.Generic.List[string]]::new()"));
  assert.ok(hook.indexOf("!define PICOAGENT_REVIEW_DATA_ROOT") < hook.indexOf("!include"));
  assert.match(hook, /SetOverwrite off[\s\S]*?File \/oname=config\.json[\s\S]*?SetOverwrite on/);
});

test("審査版の削除確認は専用領域だけを対象にする", () => {
  const post = liteHook.slice(liteHook.indexOf("!macro NSIS_HOOK_POSTUNINSTALL"));
  assert.match(post, /!ifdef PICOAGENT_REVIEW_DATA_ROOT[\s\S]*?RMDir \/r "\$APPDATA\\\$\{PICOAGENT_REVIEW_DATA_ROOT\}"[\s\S]*?!else[\s\S]*?RMDir \/r "\$APPDATA\\PicoAgent"/);
  assert.match(liteHook, /\$APPDATA\\\$\{PICOAGENT_MANAGED_DATA_ROOT\}\\bundled/);
});

test("同梱ランタイムは個別削除の前にディレクトリ単位で削除する", () => {
  for (const hook of [liteHook, fullHook]) {
    const pre = hook.slice(hook.indexOf("!macro NSIS_HOOK_PREUNINSTALL"), hook.indexOf("!macro NSIS_HOOK_POSTUNINSTALL"));
    assert.match(pre, /RMDir \/r "\$INSTDIR\\resources\\bundled\\aivis"/);
    assert.match(pre, /SetDetailsPrint none/);
    assert.match(hook.slice(hook.indexOf("!macro NSIS_HOOK_POSTUNINSTALL")), /SetDetailsPrint both/);
  }
});
