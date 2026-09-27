// 設定画面の入力・保存の「静かに壊れる」契約。メイン画面の同種の契約は main-input-feedback-contract.mjs にある。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const bridge = readFileSync(new URL("../../ui/picoagent.js", import.meta.url), "utf8");

test("設定のJSONが不正でも保存失敗後に入力と復元を操作できる", async () => {
  const button = () => ({ disabled: false, classList: { add() {}, toggle() {} }, addEventListener(_, callback) { this.click = callback; } });
  const saveBtn = button();
  const resetChangesBtn = button();
  const form = { inert: false, setAttribute() {} };
  let saves = 0;
  const context = vm.createContext({
    saveBtn, resetChangesBtn, form, dirtySummary: button(), statusEl: {},
    collect() { throw new Error("ツール引数は正しいJSONを入力してください"); },
    logApp() {}, errorDetail: String, errorMessage: error => error?.message || String(error),
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
      updateDirtyState() {}, logApp() {}, errorDetail: String, errorMessage: error => error?.message || String(error),
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

test("設定検索は購読1本で1フレーム1回だけ絞り込む", () => {
  // 検索は全設定項目の文面を走査する。1打鍵ごとに走らせると入力が引っかかるため、
  // 変更印の更新と同じ共通部品でまとめる。消去ボタンの出し入れだけは待たせない。
  assert.equal(settings.match(/settingsSearch\?\.addEventListener\("input"/g)?.length, 1);
  assert.match(settings, /settingsSearchScheduler = window\.PicoFrameScheduler\.create\(applySettingsSearch\)/);
  assert.match(settings, /settingsSearch\?\.addEventListener\("input", \(\) => \{[\s\S]*?clearSettingsSearch\.hidden = !settingsSearch\.value;[\s\S]*?settingsSearchScheduler\.schedule\(\);/);
  assert.match(settings, /settingsSearchScheduler\?\.cancel\(\);/);
});
