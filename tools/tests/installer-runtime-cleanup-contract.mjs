// 通常版と関係者版の同梱物回収・利用者データ保護を検査する。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../../", import.meta.url);
const hooks = await Promise.all([
  "src-tauri/installer-hooks-lite.nsh",
  "src-tauri/installer-hooks.nsh",
].map(path => readFile(new URL(path, root), "utf8")));
const config = JSON.parse(await readFile(new URL("src-tauri/tauri.conf.json", root), "utf8"));

test("通常版インストーラは日本語とアプリ共通のアイコンを使う", () => {
  assert.deepEqual(config.bundle.windows.nsis.languages, ["Japanese"]);
  assert.equal(config.bundle.windows.nsis.installerIcon, "icons/icon.ico");
});

test("同梱ランタイムは個別削除の前にディレクトリ単位で削除する", () => {
  for (const hook of hooks) {
    const pre = hook.slice(hook.indexOf("!macro NSIS_HOOK_PREUNINSTALL"), hook.indexOf("!macro NSIS_HOOK_POSTUNINSTALL"));
    assert.match(pre, /RMDir \/r "\$INSTDIR\\resources\\bundled\\aivis"/);
    assert.match(pre, /SetDetailsPrint none/);
    assert.match(hook.slice(hook.indexOf("!macro NSIS_HOOK_POSTUNINSTALL")), /SetDetailsPrint both/);
  }
});

test("利用者データの削除はサイレントと既定のいいえで回避する", () => {
  for (const hook of hooks) {
    const post = hook.slice(hook.indexOf("!macro NSIS_HOOK_POSTUNINSTALL"));
    const silent = post.indexOf("IfSilent picoagent_keep_user_data 0");
    const deletion = post.indexOf('RMDir /r "$APPDATA\\PicoAgent"');
    assert.ok(silent >= 0 && deletion > silent);
    assert.match(post, /MessageBox MB_YESNO\|MB_ICONQUESTION[^\n]*IDNO picoagent_keep_user_data/);
  }
});
