import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const layout = await readFile(new URL("../../src-tauri/src/layout.rs", import.meta.url), "utf8");
const startupStart = renderer.indexOf("(async () => {\n  startBootLogTicker();");
const startupEnd = renderer.indexOf("\n})().catch(error => {", startupStart);
assert.ok(startupStart >= 0 && startupEnd > startupStart, "起動シーケンスを抽出できません");
const startup = renderer.slice(startupStart, startupEnd);

test("local選択時もローカルAIの準備待ちで起動画面を塞がない", () => {
  assert.match(startup, /localEngineRequired = \(info\.providerId \|\| "local"\) === "local"/);
  assert.match(startup, /if \(localEngineRequired\) \{[\s\S]*?waitForEngineReady\(\)\.catch/);
  assert.doesNotMatch(startup, /await waitForEngineReady\(\)/);
  assert.doesNotMatch(startup, /await prepareSelectedVoiceModel\(/);
  assert.match(startup, /startBackgroundStartupServices\(info, startupGreeting\)/);
  assert.match(renderer, /const STARTUP_UI_DEADLINE_MS = 15000/);
  assert.match(renderer, /!frontendInitialized \|\| \(localEngineRequired && !localEngineReady\)/);
});

test("15秒解除後に初期化が戻っても送信禁止と保存形式を正しく同期する", () => {
  assert.match(renderer, /let frontendInitialized = false/);
  assert.match(renderer, /if \(!frontendInitialized\) \{[\s\S]*?完了すると送信できます/);
  assert.match(startup, /frontendInitialized = true;[\s\S]*?updateComposerState\(\)/);
  assert.match(startup, /if \(bootUiReleased\) applyLayout\(startupLayout\);[\s\S]*?else applyLayout\("stream"\)/);
  assert.match(renderer, /let bootBackendFinished = false/);
  assert.match(renderer, /let bootBackendFinishPromise = null/);
  assert.match(renderer, /bootBackendFinishPromise = window\.picoagent\.finishBoot\(\)[\s\S]*?bootBackendFinished = true/);
  assert.match(renderer, /finally\(\(\) => \{ bootBackendFinishPromise = null; \}\)/);
  assert.match(renderer, /function scheduleBootBackendRetry\(\)/);
  assert.match(renderer, /bootBackendRetryDelayMs = Math\.min\(bootBackendRetryDelayMs \* 2, BOOT_BACKEND_RETRY_MAX_MS\)/);
  assert.match(renderer, /位置・寸法の復旧中/);
  assert.match(renderer, /catch \(error\) \{[\s\S]*?scheduleBootBackendRetry\(\)/);
  assert.doesNotMatch(renderer, /if \(bootUiReleased\) return/);
});

test("finishBoot失敗後も位置意図を保持して再試行できる", () => {
  const finishStart = layout.indexOf("pub fn finish_boot");
  const finishEnd = layout.indexOf("\n}\n\n/// 保存中", finishStart);
  const finish = layout.slice(finishStart, finishEnd);
  assert.match(finish, /boot_window_placement\(app\)/);
  assert.ok(finish.indexOf("apply(app, id)?") < finish.indexOf("clear_boot_window_placement(app)"));
  assert.ok(finish.indexOf("clear_boot_window_placement(app)") < finish.indexOf("BOOT_FINISHED.store(true"));
  assert.doesNotMatch(finish, /take_boot_window_placement/);
});

test("WebView再読込の起動リセットは完了処理と同じロック内で直列化する", () => {
  const restartStart = layout.indexOf("pub fn restart_boot");
  const restartEnd = layout.indexOf("\n}", restartStart);
  const restart = layout.slice(restartStart, restartEnd);
  assert.match(restart, /BOOT_TRANSITION[\s\S]*?lock\(\)/);
  assert.ok(restart.indexOf("BOOT_TRANSITION") < restart.indexOf("BOOT_FINISHED.store(false"));
  assert.match(restart, /apply_boot_locked\(app\)/);
  assert.doesNotMatch(restart, /apply_boot\(app\)/);
});

test("ローカルAI監視開始後すぐ保存形式へ復帰し音声準備を始める", () => {
  const engine = startup.indexOf("waitForEngineReady().catch");
  const finish = startup.indexOf("await releaseBootUi()");
  const background = startup.indexOf("startBackgroundStartupServices(info, startupGreeting)");
  assert.ok(engine >= 0 && finish > engine, "ローカルAIの状態監視を始めずに起動画面を終了しています");
  assert.ok(background > finish, "起動完了より先に音声モデルの準備を始めています");
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
  assert.match(startup, /finally \{[\s\S]*await releaseBootUi\(\)/);
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
