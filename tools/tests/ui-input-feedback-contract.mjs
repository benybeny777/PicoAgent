import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
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
    vm.runInContext("let modelLoadRevision = 0;", context);
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
  assert.match(settings, /async function render\(\) \{\s*\+\+modelLoadRevision;\s*providerDraftValues = \{\};/);
});
