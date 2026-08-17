import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
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

test("任意キャラクターのI/O待ちはすべて有界化する", () => {
  assert.match(renderer, /fetchJsonWithTimeout/);
  assert.match(renderer, /new AbortController\(\)/);
  assert.match(renderer, /loadScriptOnce[\s\S]*STARTUP_RESOURCE_TIMEOUT_MS/);
  assert.match(renderer, /probeRigLayers[\s\S]*STARTUP_RESOURCE_TIMEOUT_MS/);
  assert.match(renderer, /loadLive2DModelWithTimeout/);
  assert.match(renderer, /destroyDetachedLive2DModel\(model\)/, "遅延完了したモデルを破棄する");
  assert.match(renderer, /function displayPreloadedFrame[\s\S]*image\.naturalWidth < 1/, "未読込フレームを表示しない");
  assert.doesNotMatch(renderer, /petSprite\.src = frameUrl\(animationName, frame\)/, "連番を先読み前に直接表示しない");
});

test("選択キャラ失敗時は既定PNGへ一度だけ切り替えて起動完了する", () => {
  assert.match(startup, /await loadCharacter\("mugi"/);
  assert.match(startup, /const characterLoadPromise = loadStartupCharacter\(\)/);
  assert.match(startup, /finally \{[\s\S]*await window\.picoagent\.finishBoot\(\)/);
});

test("プロバイダ疎通は起動時の短い上限でfalseへ正規化する", () => {
  assert.match(commands, /STARTUP_PROVIDER_PROBE_TIMEOUT[\s\S]*from_secs\(3\)/);
  assert.match(commands, /probe_with_timeout\(p\.is_up/);
  assert.match(commands, /timeout\(timeout, probe\)\.await\.unwrap_or\(false\)/);
  // 起動経路は短い上限を保つ。設定保存後の確認は利用者が結果を待つ場面なので、
  // 同じ3秒だと「遅いが生きている」リモートプロバイダを停止中と誤表示する。
  assert.match(commands, /current_up\(&state, STARTUP_PROVIDER_PROBE_TIMEOUT\)/);
  assert.match(commands, /SETTINGS_PROVIDER_PROBE_TIMEOUT: std::time::Duration =\s*std::time::Duration::from_secs\(10\)/);
  assert.match(commands, /current_up\(&state, SETTINGS_PROVIDER_PROBE_TIMEOUT\)/);
});
