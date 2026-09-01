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

test("local選択時はAIと音声の準備完了まで起動画面を維持する", () => {
  assert.match(startup, /localEngineRequired = \(info\.providerId \|\| "local"\) === "local"/);
  assert.match(startup, /if \(localEngineRequired\) \{[\s\S]*?await waitForEngineReady\(\)/);
  assert.match(renderer, /prepareSelectedVoiceModel\(\)[\s\S]*?setStatus\("声を準備中…"\)/);
  assert.match(startup, /await prepareSelectedVoiceModel\(\)[\s\S]*?await prepareSpeech\(startupGreeting\)/);
  assert.doesNotMatch(renderer, /STARTUP_UI_DEADLINE_MS/);
  assert.match(renderer, /!frontendInitialized \|\| \(localEngineRequired && !localEngineReady\)/);
});

test("準備完了後に保存形式へ戻し起動音声を再生する", () => {
  assert.match(renderer, /let frontendInitialized = false/);
  assert.match(renderer, /if \(!frontendInitialized\) \{[\s\S]*?完了すると送信できます/);
  assert.match(startup, /frontendInitialized = true;[\s\S]*?updateComposerState\(\)/);
  assert.match(startup, /if \(bootUiReleased\) applyLayout\(startupLayout\);[\s\S]*?else applyLayout\("stream"\)/);
  const engine = startup.indexOf("await waitForEngineReady()");
  const voice = startup.indexOf("await prepareSelectedVoiceModel()");
  const speech = startup.indexOf("await prepareSpeech(startupGreeting)");
  const finish = startup.indexOf("await releaseBootUi()");
  const play = startup.indexOf("playPreparedSpeech(startupPreparedSpeech)");
  assert.ok(engine >= 0 && voice > engine && speech > voice && finish > speech && play > finish,
    "AI・音声準備・画面復帰・再生の順序が崩れています");
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

test("長いダウンロード中は進捗通知を続け通常画面で停止する", () => {
  assert.match(renderer, /bootProgressTimer = setInterval\([\s\S]*?window\.picoagent\.bootProgress\(\)[\s\S]*?10000\)/);
  assert.match(renderer, /async function releaseBootUi[\s\S]*?clearInterval\(bootProgressTimer\)/);
  assert.match(commands, /pub fn pet_boot_progress[\s\S]*?boot_screen::mark_progress/);
});

test("起動画面は初期3キャラの寝姿だけをランダム選択する", () => {
  assert.match(renderer, /STARTUP_SLEEPING_CHARACTER_IDS = Object\.freeze\(\["mugi", "rin", "mai"\]\)/);
  assert.match(renderer, /function chooseStartupSleepingCharacter\(randomValue = Math\.random\(\)\)/);
  assert.match(startup, /await prepareBootCharacter\(chooseStartupSleepingCharacter\(\)\)/);
  assert.ok(startup.indexOf("chooseStartupSleepingCharacter()") < startup.indexOf("loadStartupCharacter()"),
    "寝姿を通常キャラクターより先に読み込んでいません");
  assert.doesNotMatch(startup, /prepareBootCharacter\(info\.characterId/);
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
