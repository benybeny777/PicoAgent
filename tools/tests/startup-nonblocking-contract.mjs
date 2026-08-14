import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const startupStart = renderer.indexOf("(async () => {\n  startBootLogTicker();");
const startupEnd = renderer.indexOf("\n})().catch(error => {", startupStart);
assert.ok(startupStart >= 0 && startupEnd > startupStart, "起動シーケンスを抽出できません");
const startup = renderer.slice(startupStart, startupEnd);

test("画面起動はローカルAIと音声モデルの準備を待たない", () => {
  assert.doesNotMatch(startup, /await waitForEngineReady\(/);
  assert.doesNotMatch(startup, /await prepareSelectedVoiceModel\(/);
  assert.match(startup, /startBackgroundStartupServices\(info, startupGreeting\)/);
});

test("保存形式への復帰後にバックグラウンド準備を始める", () => {
  const finish = startup.indexOf("await window.picoagent.finishBoot()");
  const background = startup.indexOf("startBackgroundStartupServices(info, startupGreeting)");
  assert.ok(finish >= 0 && background > finish, "起動完了より先に追加機能を待っています");
});
