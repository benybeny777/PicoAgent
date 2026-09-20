import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const bridge = readFileSync(new URL("../../ui/picoagent.js", import.meta.url), "utf8");
const between = (start, end) => renderer.slice(renderer.indexOf(start), renderer.indexOf(end, renderer.indexOf(start)));

function keyboard() {
  const input = {
    value: "下書き", selectionStart: 0, selectionEnd: 0,
    addEventListener(_, callback) { this.keydown = callback; },
    setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; },
  };
  const context = vm.createContext({
    input, status: {}, setTimeout() {},
    setInputValue(value) { input.value = value; },
    composer: { requestSubmit() { throw new Error("意図しない送信"); } },
  });
  vm.runInContext('const promptHistory = ["前の入力", "直前の入力"]; let promptHistoryIndex = null; let promptDraft = "";', context);
  vm.runInContext(between("function recallPromptHistory(direction)", "function openComposer()"), context);
  vm.runInContext(between("function isImeComposing(event)", "let historySearchTimer"), context);
  return input;
}

function press(input, key, extra = {}) {
  let prevented = false;
  input.keydown({ key, preventDefault() { prevented = true; }, ...extra });
  return prevented;
}

test("上下キーは未定義処理を呼ばず履歴と下書きを往復する", () => {
  const input = keyboard();
  assert.equal(press(input, "ArrowUp"), true);
  assert.equal(input.value, "直前の入力");
  assert.equal(press(input, "ArrowDown"), true);
  assert.equal(input.value, "下書き");
});

test("文中と選択中の上下キーは通常のカーソル操作を保つ", () => {
  const input = keyboard();
  input.selectionStart = input.selectionEnd = 1;
  assert.equal(press(input, "ArrowUp"), false);
  assert.equal(press(input, "ArrowDown"), false);
  input.selectionStart = 0;
  assert.equal(press(input, "ArrowUp"), false);
  assert.equal(input.value, "下書き");
});

test("IME候補操作と変換確定は履歴・送信へ流さない", () => {
  for (const extra of [{ isComposing: true }, { keyCode: 229 }]) {
    const input = keyboard();
    for (const key of ["ArrowUp", "ArrowDown", "Enter", "Escape"]) assert.equal(press(input, key, extra), false);
    assert.equal(input.value, "下書き");
  }
});

test("コードコピーは成功・拒否・非対応を区別し再操作できる", async () => {
  for (const outcome of [true, false, new Error("拒否")]) {
    let finish;
    let calls = 0;
    const context = vm.createContext({
      async copyText() { calls++; if (outcome instanceof Error) throw outcome; return outcome; },
      logApp() {}, errorDetail: String,
      setTimeout(callback) { finish = callback; },
    });
    vm.runInContext(between("async function copyCodeBlock(", "function renderAlarmReply("), context);
    const button = { textContent: "コピー", disabled: false };
    await context.copyCodeBlock(button, "code");
    assert.equal(button.textContent, outcome === true ? "コピーしました" : "コピーできませんでした");
    await context.copyCodeBlock(button, "code");
    assert.equal(calls, 1);
    finish();
    assert.equal(button.disabled, false);
    assert.equal(button.textContent, "コピー");
  }
});

test("設定のJSONが不正でも保存失敗後に入力と復元を操作できる", async () => {
  const button = () => ({ disabled: false, classList: { add() {}, toggle() {} }, addEventListener(_, callback) { this.click = callback; } });
  const saveBtn = button();
  const resetChangesBtn = button();
  const form = { inert: false, setAttribute() {} };
  let saves = 0;
  const context = vm.createContext({
    saveBtn, resetChangesBtn, form, dirtySummary: button(), statusEl: {},
    collect() { throw new Error("ツール引数は正しいJSONを入力してください"); },
    logApp() {}, errorDetail: String,
    window: { picoagent: { settings: { save() { saves++; } } } },
  });
  vm.runInContext("let renderingSettings = false; let savedSnapshot = {}; let settingsMutationBusy = false;", context);
  vm.runInContext(settings.slice(settings.indexOf("function updateDirtyStateNow()"), settings.indexOf("// collect()と全設定")), context);
  vm.runInContext("const dirtyStateScheduler = { flush: updateDirtyStateNow };", context);
  vm.runInContext(settings.slice(settings.indexOf("function setSettingsMutationBusy("), settings.indexOf("function nodeSearchText(")), context);
  vm.runInContext(settings.slice(settings.indexOf('saveBtn.addEventListener("click"'), settings.indexOf('resetChangesBtn?.addEventListener("click"')), context);
  context.updateDirtyStateNow();
  assert.equal(resetChangesBtn.disabled, false);
  await saveBtn.click();
  assert.equal(saves, 0);
  assert.equal(form.inert, false);
  assert.equal(saveBtn.disabled, false);
  assert.equal(resetChangesBtn.disabled, false);
  assert.match(context.statusEl.textContent, /保存に失敗.*JSON/);
  await saveBtn.click();
  assert.equal(form.inert, false);
});

test("モデル一覧の逆順応答と古い失敗は現在の選択を上書きしない", async () => {
  for (const oldFails of [false, true]) {
    const pending = [];
    const options = [];
    const context = vm.createContext({
      modelInput: { value: "初期値" }, modelOptions: { set innerHTML(_) { options.length = 0; }, append(node) { options.push(node.value); } },
      statusEl: {}, data: { active: { providerId: "b" } },
      providerDefaultModel: id => id, el: (_, value) => value,
      updateDirtyState() {}, logApp() {}, errorDetail: String,
      window: { picoagent: { settings: { listModels() { return new Promise((resolve, reject) => pending.push({ resolve, reject })); } } } },
    });
    vm.runInContext("let modelLoadGeneration = 0; const cliModelSelect = null;", context);
    vm.runInContext(settings.slice(settings.indexOf("async function loadModels("), settings.indexOf("function collect()")), context);
    const old = context.loadModels("a", true);
    const current = context.loadModels("b", true);
    pending[1].resolve(["b"]);
    await current;
    if (oldFails) pending[0].reject(new Error("古い接続失敗"));
    else pending[0].resolve(["a"]);
    await old;
    assert.equal(context.modelInput.value, "b");
    assert.deepEqual(options, ["b"]);
    assert.equal(context.statusEl.textContent, "");
    const manual = context.loadModels("b", true);
    context.modelInput.value = "手入力モデル";
    pending[2].resolve(["b"]);
    await manual;
    assert.equal(context.modelInput.value, "手入力モデル");
  }
});

test("非表示へ切り替えたAIの入力は保存収集でも保持し復号不能秘密を消さない", () => {
  const inputs = new Map([
    ["API_KEY", { type: "password", value: "編集中のキー" }],
    ["BROKEN_KEY", { type: "password", value: "" }],
  ]);
  const context = vm.createContext({
    inputs, cloneSettings: value => JSON.parse(JSON.stringify(value)),
    data: { values: {}, unavailableSecrets: [{ key: "BROKEN_KEY" }] },
  });
  vm.runInContext(`let providerDraftValues = {}; const savedSnapshot = {};
    const PRESERVE_UNREADABLE_SECRET = "keep";
    const characterSelect = null, characterPersona = null, characterSpeaker = null, characterLive2dSdkSelect = null;
    const characterOverrides = {}, characterVoiceOverrides = {}, characterLive2dSdkOverrides = {}, characterFraming = {}, characterStreamProfiles = {};
    const alarmEditorState = null, scheduleEditorState = null;`, context);
  vm.runInContext(settings.slice(settings.indexOf("function rememberProviderDraft("), settings.indexOf("function cloneSettings(")), context);
  vm.runInContext(settings.slice(settings.indexOf("function collect()"), settings.indexOf("// 再起動の確認画面")), context);
  context.rememberProviderDraft({ querySelectorAll: () => [...inputs.keys()].map(settingKey => ({ dataset: { settingKey } })) });
  inputs.clear();
  const patch = context.collect();
  assert.equal(patch.API_KEY, "編集中のキー");
  assert.equal(patch.BROKEN_KEY, "keep");
  assert.match(settings, /providerDraftValues\[f\.key\] \?\? v\[f\.key\]/);
  assert.match(settings, /async function render\(\) \{\s*\+\+modelLoadGeneration;\s*providerDraftValues = \{\};/);
});

test("設定保存は変更差分だけを送り内部状態を競合対象にしない", () => {
  const context = vm.createContext({});
  vm.runInContext(`const PRESERVE_UNREADABLE_SECRET = "keep";
    const SETTINGS_RUNTIME_KEYS = new Set(["PICOAGENT_WINDOW_LAYOUTS", "PICOAGENT_SCHEDULED_TASK_STATE"]);`, context);
  vm.runInContext(settings.slice(settings.indexOf("function stableStringify("), settings.indexOf("function updateDirtyStateNow(")), context);
  const request = context.settingsChangeRequest(
    { SAME: "x", CHANGE: "new", SECRET: "keep", NEW_KEY: "value", PICOAGENT_WINDOW_LAYOUTS: { pet: {} } },
    { SAME: "x", CHANGE: "old", SECRET: "", PICOAGENT_WINDOW_LAYOUTS: { pet: { width: 1 } } },
    { SAME: "r1", CHANGE: "r2", SECRET: "r3", PICOAGENT_WINDOW_LAYOUTS: "runtime" },
  );
  assert.deepEqual({ ...request.patch }, { CHANGE: "new", NEW_KEY: "value" });
  assert.equal(request.baseRevisions.CHANGE, "r2");
  assert.equal(request.baseRevisions.NEW_KEY, "missing");
  assert.equal(request.baseRevisions.SAME, undefined);
  assert.equal(request.baseRevisions.PICOAGENT_WINDOW_LAYOUTS, undefined);
  assert.match(bridge, /save: \(patch, baseRevisions\) => call\("settings_save", \{ patch, baseRevisions \}\)/);
});

test("設定競合は草稿を捨てず最新値へ載せ直す", () => {
  const saveFlow = settings.slice(
    settings.indexOf('saveBtn.addEventListener("click"'),
    settings.indexOf('resetChangesBtn?.addEventListener("click"'),
  );
  assert.match(saveFlow, /const draft = cloneSettings\(request\.patch\)/);
  assert.match(saveFlow, /const latest = await window\.picoagent\.settings\.load\(\)/);
  assert.match(saveFlow, /data\.values = \{ \.\.\.cloneSettings\(latest\.values\), \.\.\.draft \}/);
  assert.match(saveFlow, /savedSnapshot = latestSnapshot/);
  assert.match(saveFlow, /入力内容を確認して、もう一度保存してください/);
});

test("変更判定のcollectはキャラ草稿を書き換えない", () => {
  const collect = settings.slice(settings.indexOf("function collect()"), settings.indexOf("// 再起動の確認画面"));
  assert.doesNotMatch(collect, /saveCurrentCharacter/);
  assert.match(settings, /saveCurrentCharacterPersona\(displayedCharacterId\)/);
  assert.match(settings, /saveCurrentCharacterVoice\(displayedCharacterId\)/);
  assert.match(settings, /saveCurrentCharacterLive2dSdk\(displayedCharacterId\)/);
});

test("停止要求のあと応答が返らなくても画面の待機を解除できる", () => {
  // 停止を押したあと実行中の送信が返らないと、待機表示が残って入力も再送もできず、
  // 復帰する手段が無くなる。一定時間で画面側だけ中断扱いにして戻す。
  assert.match(renderer, /const CANCEL_RELEASE_MS = \d+/);
  const cancel = renderer.match(/async function requestAgentCancellation\(button\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.ok(cancel.indexOf("cancelReleaseTimer = setTimeout(") < cancel.indexOf("await window.picoagent.cancel()"),
    "cancel IPC自体が返らない場合にも備え、待機解除の見張りを先に開始してください");
  assert.match(cancel, /releasePendingSend\(\{ stale: true, releasedByCancel: true \}\)/);
  // 送信は打ち切りの合図と競走させ、既存の「中断扱い」経路（stale）で後始末する。
  assert.match(renderer, /const releasedByCancel = new Promise\(resolve => \{ releasePendingSend = resolve; \}\)/);
  assert.match(renderer, /Promise\.race\(\[window\.picoagent\.send\(fullText\), releasedByCancel\]\)/);
  assert.match(renderer, /result\?\.releasedByCancel\s*\n?\s*\? "停止しました/);
  // 見張りは送信の後始末で必ず解く（次の送信へ持ち越さない）。
  assert.match(renderer, /clearCancelRelease\(\);\s*\n\s*busy = false;/);
});

test("窓に収まらないフキダシ保存値は表示側で収める", () => {
  // 広い画面で広げたフキダシの保存値を狭い窓へそのまま当てると、入力欄と送信ボタンが
  // 窓の外へ出て操作できなくなる。保存値は変えず、当てる寸法だけ窓へ収める。
  assert.match(renderer, /function fitBubbleSizeToWindow\(width, height\) \{[\s\S]*?Math\.min\(width, maxWidth\)[\s\S]*?Math\.min\(height, maxHeight\)/);
  const apply = renderer.match(/function applyBubbleLayout\(width, height, left, top\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(apply, /bubble\.style\.width = `\$\{fitBubbleSizeToWindow\(w, h\)\.width\}px`/);
  assert.match(apply, /bubble\.style\.height = `\$\{fitBubbleSizeToWindow\(w, h\)\.height\}px`/);
  // 窓が広がれば保存どおりへ戻す。つまみ操作中と拡大中は触らない。
  const refit = renderer.match(/function refitBubbleToWindow\(\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(refit, /bubble\.classList\.contains\("fullscreen"\) \|\| resizeStart/);
  assert.match(refit, /savedBubbleLayout\?\.width/);
  // リサイズ中の再計算は1本の購読へまとめ、描画フレームごとに1回へ間引く。
  const handler = renderer.match(/function handleWindowResize\(\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(handler, /refitBubbleToWindow\(\);/);
  assert.match(handler, /refreshBubblePlacement\(\);/);
  assert.match(handler, /applyCharacterOffset\(characterOffset\.x, characterOffset\.y, \{ clamp: false \}\)/);
  assert.equal(renderer.match(/window\.addEventListener\("resize"/g)?.length, 1);
  assert.match(renderer, /const windowResizeScheduler = window\.PicoFrameScheduler\.create\(handleWindowResize\)/);
  assert.match(renderer, /window\.addEventListener\("resize", \(\) => windowResizeScheduler\.schedule\(\)\)/);
  // 間引き用のフレーム予約は終了時に取り消す（解放漏れを残さない）。
  assert.match(renderer, /windowResizeScheduler\?\.cancel\(\);/);
});

test("ダブルクリックでは全画面へ切り替えず拡大ボタンだけを使う", () => {
  assert.doesNotMatch(renderer, /addEventListener\("dblclick"/);
  assert.match(renderer, /toggleFullscreenButton\?\.addEventListener\("click"[\s\S]*?toggleBubbleFullscreen\(\)/);
});

test("準備中・失敗の理由は入力や読み上げ操作で消さない", () => {
  // SPEC: `error` は診断・修復へ進める理由として常時表示する。入力のたびに状態表示を
  // 空にすると、赤い空欄と無効な送信ボタンだけが残り、送れない理由が分からなくなる。
  assert.match(renderer, /let localEngineStatusText = ""/);
  const clear = renderer.match(/function clearTransientStatus\(\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(clear, /localEngineStatusText && !localEngineReady \? localEngineStatusText : ""/);
  // 文字数表示は消す側の入口を通す（直接空にしない）。
  assert.match(renderer, /if \(len > 100\) status\.textContent = `\$\{len\}文字`;\s*\n\s*else clearTransientStatus\(\);/);
  // 準備中・失敗の文言は applyEngineStatus が控える。
  const apply = renderer.match(/function applyEngineStatus\(s\) \{[\s\S]*?\n\}/)?.[0] || "";
  assert.match(apply, /localEngineStatusText = "";/);
  assert.match(apply, /準備に失敗: \$\{s\.message \|\| "不明なエラー"\}`\);\s*\n\s*localEngineStatusText = status\.textContent;/);
});

test("一時表示を消すすべての経路でローカルAIの失敗理由へ戻す", () => {
  const helperStart = renderer.indexOf("function clearTransientStatus()");
  const helperEnd = renderer.indexOf("\n}", helperStart) + 2;
  assert.ok(helperStart >= 0 && helperEnd > helperStart);
  const withoutHelper = renderer.slice(0, helperStart) + renderer.slice(helperEnd);
  assert.doesNotMatch(withoutHelper, /status\.textContent\s*=\s*""/,
    "状態表示を空にせず clearTransientStatus() で失敗理由へ戻してください");
});

test("フキダシの自動配置は配置基準の枠内へ収める", () => {
  // 位置はフキダシの配置基準（.pet-app）からの座標で決まる。ウィンドウ全体の高さで
  // 上限を計算すると上端メニューぶん（body の padding-top 32px）だけ緩くなり、
  // フキダシが窓の下へはみ出して状態表示が切れる。
  const place = renderer.match(/function placeBubbleAwayFromCharacter\([\s\S]*?\n\}/)?.[0] || "";
  assert.match(place, /const container = bubble\.offsetParent instanceof HTMLElement \? bubble\.offsetParent : document\.documentElement/);
  assert.match(place, /const viewportWidth = container\.clientWidth/);
  assert.match(place, /const viewportHeight = container\.clientHeight/);
  assert.doesNotMatch(place, /document\.documentElement\.clientHeight/);
});

test("生成中の本文描画は最短間隔をあけて組み直す", () => {
  // 1回の描画で本文全体を組み直すため、トークンが毎フレーム届くローカルAIでは
  // 長い回答ほど入力とアニメが引っかかっていた。読める速さは保ったまま間引く。
  assert.match(renderer, /const STREAM_RENDER_MIN_MS = 80;/);
  assert.match(renderer, /if \(performance\.now\(\) - lastStreamRenderAt < STREAM_RENDER_MIN_MS\) \{\s*\n\s*streamRenderFrame = requestAnimationFrame\(paintStream\);\s*\n\s*return;/);
  assert.match(renderer, /lastStreamRenderAt = performance\.now\(\);\s*\n[\s\S]{0,200}?renderMarkdown\(reply, streamBuffer\);/);
  // 送信直後とステップ境界では、最初のチャンクを待たせない。
  assert.equal(renderer.match(/(?<!let )lastStreamRenderAt = 0;/g)?.length, 2);
});
