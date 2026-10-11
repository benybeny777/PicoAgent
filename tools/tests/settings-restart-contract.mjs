import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const commands = readFileSync(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const between = (start, end) => commands.slice(commands.indexOf(start), commands.indexOf(end, commands.indexOf(start)));
const save = between("pub async fn settings_save(", "fn preserve_unreadable_secrets(");
const restart = between("pub fn restart_app(", "pub async fn engine_status(");

test("保存した値の版は差分確定から応答まで同じスナップショットを使う", () => {
  const load = between("pub async fn settings_load(", "pub async fn settings_list_models(");
  assert.match(load, /let mut values = state\.config\.get_all\(\);\s*let value_revisions = Config::revisions_for\(&values\);/);
  assert.match(save, /let saved_revisions = Config::revisions_for\(&prepared_patch\);/);
  assert.match(save.slice(save.lastIndexOf('"ok": true')), /"valueRevisions": saved_revisions/);
});

test("設定保存と再初期化は同じ保留要求の所有者を経由する", () => {
  assert.match(save, /let \(prepared_patch, restart\) = \{\s*let mut restart = state\.settings_restart\.lock\(\)\.unwrap\(\);/);
  const committed = save.slice(save.indexOf("if !conflicts.is_empty()"));
  assert.match(committed, /return Ok\([\s\S]*?let restart = restart\.include\(SettingsRestart \{/);
  for (const [member, condition] of [
    ["engine", "provider_changed \\|\\| local_engine_changed"],
    ["aivis", "tts_runtime_changed"],
    ["character", "character_changed \\|\\| live2d_sdk_changed"],
  ]) assert.match(committed, new RegExp(`${member}: ${condition}`));
  assert.match(save, /let restart_engine = restart\.engine;/);
  assert.match(save, /let restart_aivis = restart\.aivis;/);
  assert.match(save, /let requires_restart = restart\.required\(\);/);
  assert.match(save, /if !requires_restart \{\s*\*state\.provider_id/);
  assert.match(restart, /state\.settings_restart\.lock\(\)\.unwrap\(\)/);
  assert.match(restart, /pending_restart\.apply\(/);
  assert.match(restart, /if restart\.engine \{\s*crate::engine::stop/);
  assert.match(restart, /if restart\.aivis \{\s*aivis::stop/);
});

test("再初期化開始の失敗は要求を消さず設定画面に操作を残す", () => {
  const owner = between("impl SettingsRestart {", "pub struct AppState {");
  assert.match(owner, /let result = apply\(self\.include\(requested\)\)\?;\s*\*self = Self::default\(\);/);
  assert.match(restart, /win\.emit\("pet:reload", \(\)\)\.map_err\([\s\S]*?\)\?;/);
  assert.ok(restart.indexOf('win.emit("pet:reload"') < restart.indexOf("settings.close()"));
});
