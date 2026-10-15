// 起動の契約: 準備中は起動画面を見せ、仮のキャラ・読込途中の描画を見せない。
// フロントは ui/app/main.js（起動の手順）と ui/app/boot.js（起動画面と表示の確定）、
// バックエンドは boot_screen.rs / layout.rs が持つ。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { mainScript } from "./lib/ui-source.mjs";

const main = (await readFile(new URL("../../ui/app/main.js", import.meta.url), "utf8")).replace(/\r\n/g, "\n");
const boot = await readFile(new URL("../../ui/app/boot.js", import.meta.url), "utf8");
const engine = await readFile(new URL("../../ui/app/engine.js", import.meta.url), "utf8");
const managedEngine = await readFile(new URL("../../src-tauri/src/engine.rs", import.meta.url), "utf8");
const allMain = mainScript();
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const layout = await readFile(new URL("../../src-tauri/src/layout.rs", import.meta.url), "utf8");
const bootScreen = await readFile(new URL("../../src-tauri/src/boot_screen.rs", import.meta.url), "utf8");
const windowSurface = await readFile(new URL("../../src-tauri/src/window_surface.rs", import.meta.url), "utf8");
const lib = await readFile(new URL("../../src-tauri/src/lib.rs", import.meta.url), "utf8");
const styles = await readFile(new URL("../../ui/components/boot.css", import.meta.url), "utf8");
const windowStyles = await readFile(new URL("../../ui/components/window.css", import.meta.url), "utf8");
const indexHtml = await readFile(new URL("../../ui/index.html", import.meta.url), "utf8");
const startupStart = main.indexOf("(async () => {\n  // 起動画面のDOMを組み立ててから窓を表示する。");
const startupEnd = main.indexOf("\n})().catch(error => {", startupStart);
assert.ok(startupStart >= 0 && startupEnd > startupStart, "起動シーケンスを抽出できません");
const startup = main.slice(startupStart, startupEnd);

test("初回もAI・記憶検索・音声の準備完了まで起動画面を維持する", () => {
  assert.match(engine, /required = \(info\.providerId \|\| "local"\) === "local";/);
  assert.match(startup, /status\.clear\("boot"\);\s*const state = await engine\.waitUntilSettled\(null, \{ includeMemory: true \}\);/);
  assert.match(startup, /status\.set\("声を準備中…", "boot"\);[\s\S]*?await voice\.prepareModel\(\);\s*\n\s*if \(startupGreeting\) preparedGreeting = await voice\.prepare\(startupGreeting\);/);
  // 初期化前・ローカルAIの準備前は送信させない。
  assert.match(engine, /if \(!initialized\) return "初期化完了後に送信できます";/);
  assert.match(engine, /if \(required && !ready\) return "ローカルAIの準備完了後に送信できます";/);
});

test("起動した管理エンジンの疎通失敗は子を回収してから診断する", () => {
  const polling = managedEngine.slice(managedEngine.indexOf("let mut failed = None;"), managedEngine.indexOf("fn managed_child_exit_status"));
  assert.match(polling, /Err\(error\) => \{\s*terminate_managed_child\(&app\);\s*set_status\([\s\S]*?return;/);
});

test("初回音声の導入は通常画面へ戻す前に待ち、案内は準備後に出す", () => {
  assert.doesNotMatch(startup, /engine\.required && info\.onboardingDone|engine\.waitUntilSettled\(\)\.catch/);
  assert.ok(startup.indexOf("await onboarding.installVoice()") < startup.indexOf("await voice.prepareModel()"));
  assert.ok(startup.indexOf("await voice.prepareModel()") < startup.indexOf("await boot.release(status)"));
  assert.ok(startup.indexOf("await boot.release(status)") < startup.indexOf("onboarding.show()"));
  assert.match(commands, /status\["memoryEmbedding"\][\s\S]*?memory_embedding::status\(\)/);
});

test("準備完了後に配信画面へ切り替え、起動音声はその後に再生する", () => {
  const engineAt = startup.indexOf("await engine.waitUntilSettled(");
  const voiceAt = startup.indexOf("await voice.prepareModel()");
  const speechAt = startup.indexOf("preparedGreeting = await voice.prepare(startupGreeting)");
  const releaseAt = startup.indexOf("await boot.release(status)");
  const playAt = main.indexOf("voice.playPrepared(preparedGreeting)");
  assert.ok(engineAt >= 0 && voiceAt > engineAt && speechAt > voiceAt && releaseAt > speechAt && playAt > startupStart + releaseAt,
    "AI・音声準備・画面復帰・再生の順序が崩れています");
  // 失敗しても起動画面は必ず外す（finally）。
  assert.match(startup, /\} finally \{[\s\S]*?await boot\.release\(status\);/);
  // 保存した位置・寸法の復元（finishBoot）は1本に束ね、失敗したら間隔を伸ばして裏で再試行する。
  assert.match(boot, /finishPromise = bridge\.finishBoot\(\)\s*\n\s*\.then\(\(\) => \{ backendFinished = true; \}\)\s*\n\s*\.finally\(\(\) => \{ finishPromise = null; \}\);/);
  assert.match(boot, /function scheduleRetry\(status\)/);
  assert.match(boot, /retryDelay = Math\.min\(retryDelay \* 2, RETRY_MAX_MS\);/);
  assert.match(boot, /位置・寸法の復旧中/);
});

test("ローカルAIがエラーなら音声準備へ進まず元のエラーを保つ", () => {
  assert.match(startup, /if \(engine\.required && state\?\.phase === "error"\) startupError = state\.message/);
  assert.match(startup, /if \(!startupError && voice\.enabled\) \{[\s\S]*?prepareModel/);
});

test("長いダウンロード中は進捗通知を続け通常画面で停止する", () => {
  assert.match(boot, /function startProgress\(intervalMs\)/);
  assert.match(boot, /startProgress\(INITIAL_PROGRESS_INTERVAL_MS\);/);
  // 固定値ではなく監視しきい値から決めた間隔を使う（短いしきい値でも通知が間に合う）。
  assert.match(startup, /boot\.setProgressInterval\(info\.bootProgressIntervalMs\);/);
  assert.match(commands, /"bootProgressIntervalMs": crate::boot_screen::progress_interval_ms/);
  assert.match(boot, /async release\(status\) \{\s*\n\s*if \(progressTimer\) clearInterval\(progressTimer\);/);
  assert.match(commands, /pub fn pet_boot_progress[\s\S]*?boot_screen::mark_progress/);
});

test("起動画面を先に表示し仮キャラを隠したまま最終DOMへ切り替える", async () => {
  const config = JSON.parse(await readFile(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  assert.equal(config.app.windows.find(w => w.label === "main").visible, false);
  assert.ok(main.indexOf("const boot = createBoot(") < main.indexOf("await bridge.frontendReady();"));
  const frontendReady = commands.slice(commands.indexOf("pub fn pet_frontend_ready"), commands.indexOf("pub fn pet_boot_progress"));
  assert.ok(frontendReady.indexOf("crate::layout::restart_boot(&app)") < frontendReady.indexOf("window.show()"));
  assert.match(styles, /\.app\.is-booting \.boot-screen \{ display: grid; \}/);
  assert.match(windowStyles, /\.app\.is-booting \.top-window-menu \{ display: none; \}/);
  assert.match(styles, /\.boot-screen \{[\s\S]*?inset: 0;/);
  assert.match(frontendReady, /window\.set_always_on_top\(false\)/);
  const release = boot.slice(boot.indexOf("async release(status) {"));
  assert.ok(release.indexOf('app.classList.remove("is-booting")') < release.indexOf("await reveal()"));
  assert.match(release, /if \(backendFinished\) await reveal\(\);/);
  // 表示前に最終の配置を確定させる。
  assert.ok(startup.indexOf("stage.layoutNow();") < startup.indexOf("await boot.release(status);"));
});

test("初回案内中はブラウザを操作でき、案内を閉じると最前面設定が戻る", () => {
  const complete = commands.slice(commands.indexOf("pub fn pet_complete_onboarding"), commands.indexOf("pub fn pet_show_onboarding"));
  const show = commands.slice(commands.indexOf("pub fn pet_show_onboarding"), commands.indexOf("pub fn settings_components_status"));
  const finish = commands.slice(commands.indexOf("pub fn pet_finish_boot"), commands.indexOf("pub fn pet_reveal_main"));
  assert.match(show, /main\.set_always_on_top\(false\)/);
  assert.match(complete, /main\.set_always_on_top\(state\.config\.get_bool\("PICOAGENT_ALWAYS_ON_TOP", true\)\)/);
  assert.match(finish, /if onboarding_done \{[\s\S]*?window\.set_always_on_top\(always_on_top\)/);
});

test("通常画面の表示入口を1つに保ち再試行経路でも描画を確定させる", () => {
  // 未描画canvasのまま窓を出さない保証を、初回・再試行のどちらでも通す。
  const reveal = boot.slice(boot.indexOf("async function reveal()"), boot.indexOf("async function finishBackend()"));
  assert.match(reveal, /try \{ beforeReveal\?\.\(\); \}/);
  // 描画確定の失敗で表示まで止めない（窓が出ないと操作手段が無くなる）。
  assert.ok(reveal.indexOf("catch (error)") < reveal.indexOf("await bridge.revealMain()"));
  assert.equal(allMain.match(/bridge\.revealMain\(\)/g).length, 1, "表示の入口は reveal() だけにする");
  assert.match(boot, /if \(released\) await reveal\(\);/);
  assert.match(main, /createBoot\(\{ app, screen: document\.getElementById\("bootScreen"\), beforeReveal: \(\) => host\.capturePreview\(\) \}\)/);
  assert.match(commands, /pub fn pet_reveal_main[\s\S]*?boot_screen::is_finished/);
});

test("再読込のたびに停止監視を張り直し準備中でも窓を呼び戻せる", () => {
  // 設定反映後の再初期化（restart_app → pet:reload）も新しい起動として監視する。
  // 張り直さないと、2回目以降の停止で窓も案内ページも出ず戻す手段が無くなる。
  assert.match(commands, /pub fn pet_frontend_ready[\s\S]*?boot_screen::mark_started/);
  assert.match(bootScreen, /pub fn mark_started[\s\S]*?finished\.store\(false[\s\S]*?shown\.store\(false[\s\S]*?watch\(app\.clone\(\)\)/);
  // 前回の完了状態は再読込イベントより先に消す。開始通知の中だけで戻すと、
  // JavaScriptを読み込めない故障時に白い窓を呼び戻せてしまい、停止監視も始まらない。
  const restart = commands.slice(commands.indexOf("pub fn restart_app"), commands.indexOf("pub fn engine_status"));
  assert.ok(restart.indexOf("boot_screen::mark_reloading") < restart.indexOf('win.emit("pet:reload"'),
    "再読込前に起動状態を未開始へ戻してください");
  assert.ok(restart.indexOf("layout::restart_boot") < restart.indexOf('win.emit("pet:reload"'),
    "再読込前に未完成のメイン窓を非表示へ戻してください");
  assert.match(bootScreen, /pub fn mark_reloading[\s\S]*?started\.store\(false[\s\S]*?finished\.store\(false[\s\S]*?last_progress[\s\S]*?None[\s\S]*?watch\(app\.clone\(\)\)/);
  // 古い監視が新しい起動を停止扱いにしないよう世代で分ける。
  assert.match(bootScreen, /generation: AtomicU64/);
  assert.match(bootScreen, /fn outdated\(app: &AppHandle, generation: u64\)/);
  assert.match(bootScreen, /generation\s*\.fetch_add\(1, Ordering::SeqCst\)/);
  // 呼び出しホットキーと多重起動は準備中でも応答する（窓を隠したまま閉じ込めない）。
  // ただしフロントが動き出す前は中身の無い白い窓になるので出さない。
  assert.match(windowSurface, /if !crate::boot_screen::is_started\(app\) && !crate::boot_screen::is_finished\(app\)/);
  assert.match(bootScreen, /pub fn is_started\(app: &AppHandle\) -> bool/);
  // 案内ページのURL規則は boot_screen へ集約する。
  assert.match(bootScreen, /pub fn is_page_url/);
  assert.match(lib, /boot_screen::is_page_url\(payload\.url\(\)\.scheme\(\), payload\.url\(\)\.host_str\(\)\)/);
  assert.doesNotMatch(lib, /picoboot\.localhost/);
});

test("起動画面はキャラクター枠を持たない", () => {
  assert.doesNotMatch(indexHtml, /boot-character|boot-shadow/);
  assert.doesNotMatch(styles + boot, /boot-character|boot-shadow|data-boot-artwork/);
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

test("再初期化は通常画面の位置を保存してから起動用の配置へ移る", () => {
  const start = commands.indexOf("pub fn restart_app(");
  const restart = commands.slice(start, commands.indexOf("\n}", start));
  const remember = restart.indexOf("crate::remember_window_position(&app)?");
  const characterLock = restart.indexOf("state.character.lock()");
  const commitLock = restart.indexOf("state.conversation_commit.lock()");
  const epoch = restart.indexOf("state.conversation_epoch.fetch_add(");
  assert.ok(remember >= 0 && remember < characterLock);
  assert.ok(characterLock < commitLock && commitLock < epoch);
  assert.ok(epoch < restart.indexOf("state.rebuild_from_character("));
  assert.ok(remember < restart.indexOf("crate::layout::restart_boot(&app)"));
  const finishStart = layout.indexOf("pub fn finish_boot(");
  const finish = layout.slice(finishStart, layout.indexOf("\n}", finishStart));
  assert.ok(finish.indexOf("apply(app, id)?") < finish.indexOf("remember_window_position(app)?"));
  const saveStart = lib.indexOf("pub(crate) fn remember_window_position(");
  const save = lib.slice(saveStart, lib.indexOf("\n}", saveStart));
  for (const guard of ["boot_size_active()", "is_minimized()", "is_fullscreen()", "position.x <= -30000"]) {
    assert.ok(save.indexOf(guard) >= 0 && save.indexOf(guard) < save.indexOf("save_window_position("));
  }
});

test("任意キャラクターのI/O待ちはすべて有界化する", async () => {
  const util = await readFile(new URL("../../ui/app/util.js", import.meta.url), "utf8");
  const manifest = await readFile(new URL("../../ui/character/manifest.js", import.meta.url), "utf8");
  const mesh = await readFile(new URL("../../ui/character/mesh.js", import.meta.url), "utf8");
  const live2d = await readFile(new URL("../../ui/character/live2d.js", import.meta.url), "utf8");
  const image = await readFile(new URL("../../ui/character/image.js", import.meta.url), "utf8");
  assert.match(util, /export async function fetchJson\(url, label, timeoutMs = 4000\)[\s\S]*?new AbortController\(\)/);
  assert.match(util, /export function loadScriptOnce\(src, timeoutMs = 8000\)/);
  assert.match(manifest, /fetchJson\(`\$\{entry\.root\}\/character\.json/);
  assert.match(mesh, /const rig = await fetchJson\(packUrl\(pack, rigPath\), "rig\.json", 5000\);/);
  // 描画器の中から素の fetch を使わない（上限の無い待ちを作らない）。
  for (const [name, source] of [["mesh", mesh], ["live2d", live2d], ["image", image], ["manifest", manifest]]) {
    assert.doesNotMatch(source, /[^.\w]fetch\(/, `${name} が上限の無い fetch を使っています`);
  }
  assert.match(live2d, /const MODEL_TIMEOUT_MS = 8000;/);
  assert.match(live2d, /if \(finished\) \{\s*\n\s*try \{ model\?\.destroy\?\.\(\); \}/, "遅延完了したモデルを破棄する");
  assert.match(image, /const timer = setTimeout\(\(\) => finish\(false\), RESOURCE_TIMEOUT_MS\);/);
});

test("選択キャラ失敗時は別キャラを出さず診断を表示する", () => {
  assert.match(startup, /characterSource\.element\.style\.visibility = "hidden";/);
  assert.match(startup, /characterError = `選択したキャラを読み込めません: /);
  assert.match(main, /if \(characterError\) status\.set\(characterError, "character-load"\);/);
  assert.match(main, /status\.clear\("character-load"\);/);
  assert.ok(startup.indexOf("chat.history.hydrate(info.history)") < startup.indexOf("await loadCharacter("), "履歴は描画失敗前に復元する");
  assert.equal(startup.split("chat.history.hydrate(info.history)").length, 2, "履歴の復元は一度だけ行う");
  assert.doesNotMatch(startup, /loadCharacter\("mugi"/);
});

test("プロバイダ疎通は短い上限を保ち確認不能の理由を停止状態と区別する", () => {
  assert.match(commands, /STARTUP_PROVIDER_PROBE_TIMEOUT[\s\S]*from_secs\(3\)/);
  assert.match(commands, /probe_with_timeout\(p\.is_up/);
  assert.match(commands, /timeout\(timeout, probe\)\.await\.map_err/);
  assert.doesNotMatch(commands, /timeout\(timeout, probe\)\.await\.unwrap_or\(false\)/);
  assert.match(commands, /Err\(error\) => \(None, Some\(error\)\)/);
  assert.match(commands, /"providerError": provider_error/);
  assert.match(engine, /info\.up === null/);
  assert.match(engine, /接続状態不明/);
  // 起動経路は短い上限を保つ。設定保存後の確認は利用者が結果を待つ場面なので、
  // 同じ3秒だと「遅いが生きている」リモートプロバイダを停止中と誤表示する。
  assert.match(commands, /current_up\(&state, STARTUP_PROVIDER_PROBE_TIMEOUT\)/);
  assert.match(commands, /SETTINGS_PROVIDER_PROBE_TIMEOUT: std::time::Duration =\s*std::time::Duration::from_secs\(10\)/);
  assert.match(commands, /current_up\(&state, SETTINGS_PROVIDER_PROBE_TIMEOUT\)/);
});

test("起動画面を作ってから起動の合図を送り、重い部品は使う時まで読まない", () => {
  assert.ok(main.indexOf("const boot = createBoot(") < main.indexOf("await bridge.frontendReady();"));
  assert.ok(startup.indexOf("await bridge.frontendReady();") < startup.indexOf("await Promise.all([bridge.init()"));
  // index.html は描画ライブラリ（PIXI・Live2D・MMD）を直接読み込まない（キャラの方式が決まってから読む）。
  assert.doesNotMatch(indexHtml, /vendor\//);
  assert.doesNotMatch(indexHtml, /moveable/);
});
