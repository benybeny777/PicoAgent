import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const revision = value => value === undefined ? "missing" : JSON.stringify(value);
const revisions = values => Object.fromEntries(Object.entries(values).map(([key, value]) => [key, revision(value)]));

function fixture(responseScope, options = {}) {
  const initial = { PICOAGENT_THEME: "light", PICOAGENT_ALARMS: JSON.stringify([{ id: "A", label: "元の目覚まし" }]), ...options.values };
  const latest = structuredClone(initial);
  const draft = structuredClone(initial);
  const requests = [];
  const restartRequests = [];
  const rendered = [];
  const confirmations = [...(options.confirmations || [])];
  const restartFailures = [...(options.restartFailures || [])];
  let confirmationCount = 0;
  let save;
  let rejected = false;
  const context = vm.createContext({
    saveBtn: { addEventListener(_event, listener) { save = listener; } },
    settingsMutationBusy: false, savedSnapshot: structuredClone(initial),
    data: { values: structuredClone(initial), valueRevisions: revisions(initial), pendingRestart: options.pendingRestart || null, active: { providerId: "実行中のAI" } },
    SETTINGS_RUNTIME_KEYS: new Set(), PRESERVE_UNREADABLE_SECRET: "秘密値維持",
    modelLoadGeneration: 0, statusEl: {},
    validateSettingsNumberFields: () => true,
    setSettingsMutationBusy(value) { context.settingsMutationBusy = value; },
    collect: () => structuredClone(draft), clearNumberErrors() {}, updateDirtyState() {},
    async render() {
      // DOM構築だけを置き換え、応答分岐・再読込・原本と草稿の載せ直しは実コードを通す。
      rendered.push(structuredClone(context.data.values));
      for (const key of Object.keys(draft)) delete draft[key];
      Object.assign(draft, structuredClone(context.data.values));
    },
    logApp() {}, errorDetail: String, errorMessage: String,
    confirmRestart: async () => { confirmationCount++; return confirmations.shift() ?? true; },
    window: { picoagent: {
      async restartApp(restartEngine, restartAivis) {
        restartRequests.push({ restartEngine, restartAivis });
        const failure = restartFailures.shift();
        if (failure) throw new Error(failure);
      },
      settings: {
      async load() {
        if (options.reloadFailure) throw new Error(options.reloadFailure);
        return { values: structuredClone(latest), valueRevisions: revisions(latest), pendingRestart: options.pendingRestart || null, active: { providerId: "実行中のAI" } };
      },
      async save(patch, baseRevisions) {
      const conflicts = Object.keys(baseRevisions).filter(key => baseRevisions[key] !== revision(latest[key]));
      requests.push({ patch: structuredClone(patch), baseRevisions: structuredClone(baseRevisions), conflicts });
      if (conflicts.length) {
        rejected = true;
        return { conflicts };
      }
      Object.assign(latest, patch);
      return { up: true, valueRevisions: revisions(responseScope === "all" ? latest : patch), ...options.restartPlan };
    } } } },
  });
  vm.runInContext(source.slice(source.indexOf("function cloneSettings("), source.indexOf("function updateDirtyStateNow(")), context);
  const restartStart = source.indexOf("async function applyPendingSettingsRestart(");
  if (restartStart >= 0) vm.runInContext(source.slice(restartStart, source.indexOf("function clearNumberErrors(", restartStart)), context);
  vm.runInContext(source.slice(source.indexOf('saveBtn.addEventListener("click"'), source.indexOf("resetChangesBtn?.addEventListener")), context);
  return { context, draft, latest, requests, restartRequests, rendered, confirmationCount: () => confirmationCount, save: () => save(), rejected: () => rejected };
}

for (const scope of ["all", "patch"]) {
  test(`別操作で追加した目覚ましは無関係な保存を挟んでも競合で守る（${scope}）`, async () => {
    const f = fixture(scope);
    const external = [...JSON.parse(f.latest.PICOAGENT_ALARMS), { id: "B", label: "別操作で追加" }];
    f.latest.PICOAGENT_ALARMS = JSON.stringify(external);
    f.draft.PICOAGENT_THEME = "dark";
    await f.save();
    f.draft.PICOAGENT_ALARMS = JSON.stringify([{ id: "A", label: "画面で編集" }]);
    await f.save();
    assert.equal(f.rejected(), true);
    assert.deepEqual(f.requests[1].conflicts, ["PICOAGENT_ALARMS"]);
    assert.equal(f.latest.PICOAGENT_ALARMS, JSON.stringify(external));
  });
}

test("保存したキーの版だけを更新して続く同項目の保存を受け付ける", async () => {
  const f = fixture("patch");
  const initialAlarmRevision = f.context.data.valueRevisions.PICOAGENT_ALARMS;
  f.draft.PICOAGENT_THEME = "dark";
  await f.save();
  assert.equal(f.context.data.valueRevisions.PICOAGENT_ALARMS, initialAlarmRevision);
  f.draft.PICOAGENT_THEME = "system";
  await f.save();
  assert.equal(f.rejected(), false);
  assert.equal(f.latest.PICOAGENT_THEME, "system");
});

test("競合応答は最新原本と非表示項目の草稿を分けて再表示し確認後だけ再保存する", async () => {
  const pendingRestart = { restartEngine: true, restartAivis: false };
  const f = fixture("all", {
    values: { PICOAGENT_PROVIDER: "local", OPENAI_MODEL: "旧モデル" },
    pendingRestart, confirmations: [false],
  });
  f.draft.PICOAGENT_THEME = "dark";
  f.draft.OPENAI_MODEL = "非表示AIの編集モデル";
  f.latest.PICOAGENT_THEME = "system";
  f.latest.OPENAI_MODEL = "別操作のモデル";
  f.latest.LATEST_ONLY = "別操作で追加した項目";
  const latestBefore = structuredClone(f.latest);
  await f.save();
  assert.equal(f.requests.length, 1, "競合から勝手に再保存しない");
  assert.equal(f.rendered.length, 2);
  assert.deepEqual(f.rendered[0], latestBefore, "最初の描画は最新原本だけ");
  assert.deepEqual(structuredClone(f.context.savedSnapshot), latestBefore);
  assert.equal(f.draft.PICOAGENT_THEME, "dark");
  assert.equal(f.draft.OPENAI_MODEL, "非表示AIの編集モデル");
  assert.equal(f.draft.LATEST_ONLY, latestBefore.LATEST_ONLY);
  assert.deepEqual(f.context.data.valueRevisions, revisions(latestBefore));
  assert.equal(f.context.data.active.providerId, "実行中のAI");
  assert.deepEqual(f.context.data.pendingRestart, pendingRestart);
  assert.equal(f.restartRequests.length, 0);
  assert.match(f.context.statusEl.textContent, /入力内容を確認して、もう一度保存/);
  await f.save();
  assert.deepEqual(f.requests[1].conflicts, []);
  assert.deepEqual(f.requests[1].patch, { PICOAGENT_THEME: "dark", OPENAI_MODEL: "非表示AIの編集モデル" });
  assert.equal(f.requests[1].baseRevisions.OPENAI_MODEL, revision(latestBefore.OPENAI_MODEL));
  assert.equal(f.latest.LATEST_ONLY, latestBefore.LATEST_ONLY);
  assert.equal(f.context.data.active.providerId, "実行中のAI");
  assert.deepEqual(f.context.data.pendingRestart, pendingRestart, "延期した反映待ちは保存成功で消さない");
  assert.equal(f.context.settingsMutationBusy, false);
});

test("競合後の再読込失敗では元の原本と草稿と版を保持して再試行できる", async () => {
  const f = fixture("patch", { reloadFailure: "設定を再取得できません" });
  const baseline = structuredClone(f.context.savedSnapshot);
  const versions = structuredClone(f.context.data.valueRevisions);
  f.draft.PICOAGENT_THEME = "dark";
  f.latest.PICOAGENT_THEME = "system";
  await f.save();
  assert.deepEqual(f.context.savedSnapshot, baseline);
  assert.deepEqual(f.context.data.valueRevisions, versions);
  assert.equal(f.draft.PICOAGENT_THEME, "dark");
  assert.equal(f.latest.PICOAGENT_THEME, "system");
  assert.equal(f.rendered.length, 0);
  assert.equal(f.context.settingsMutationBusy, false);
  assert.match(f.context.statusEl.textContent, /再取得できません/);
});

test("保存応答の反映は確定キーだけを深いコピーで受け取り草稿と原本を共有しない", () => {
  const f = fixture("all", { values: { UNEDITED: { value: "原本" } } });
  f.context.data.values.UNEDITED.value = "未保存の草稿";
  const originalRevision = f.context.data.valueRevisions.UNEDITED;
  const patch = { EDITED: { value: "保存済み" } };
  f.context.acceptSavedSettings(patch, { EDITED: "新しい版", UNEDITED: "他操作の版" });
  patch.EDITED.value = "呼出元の次の編集";
  f.context.data.values.EDITED.value = "画面の次の編集";
  assert.equal(f.context.savedSnapshot.EDITED.value, "保存済み");
  assert.equal(f.context.savedSnapshot.UNEDITED.value, "原本");
  assert.equal(f.context.data.values.UNEDITED.value, "未保存の草稿");
  assert.equal(f.context.data.valueRevisions.UNEDITED, originalRevision);
  assert.equal(f.context.data.valueRevisions.EDITED, "新しい版");
});

test("保存後の再初期化失敗は保存成功と区別し変更なしで再試行できる", async () => {
  const f = fixture("patch", {
    restartPlan: { requiresRestart: true, restartEngine: true, restartAivis: false },
    restartFailures: ["再読込の開始に失敗"],
  });
  f.draft.PICOAGENT_THEME = "dark";
  await f.save();
  assert.equal(f.context.savedSnapshot.PICOAGENT_THEME, "dark");
  assert.match(f.context.statusEl.textContent, /設定は保存済み.*再初期化に失敗.*再試行/);
  assert.equal(f.context.data.pendingRestart.restartEngine, true);
  await f.save();
  assert.equal(f.requests.length, 1, "保存済みの値を再送しない");
  assert.deepEqual(f.restartRequests, [
    { restartEngine: true, restartAivis: false },
    { restartEngine: true, restartAivis: false },
  ]);
  assert.equal(f.context.data.pendingRestart, null);
  assert.match(f.context.statusEl.textContent, /^反映しました/);
  await f.save();
  assert.equal(f.confirmationCount(), 2, "反映成功後は再確認しない");
  assert.equal(f.context.statusEl.textContent, "変更はありません");
});

test("再初期化の延期は次の保存操作で再確認し差分がなくても反映できる", async () => {
  const f = fixture("patch", {
    restartPlan: { requiresRestart: true, restartEngine: false, restartAivis: true },
    confirmations: [false, true],
  });
  f.draft.PICOAGENT_THEME = "dark";
  await f.save();
  assert.equal(f.restartRequests.length, 0);
  assert.match(f.context.statusEl.textContent, /次回の起動時に反映/);
  await f.save();
  assert.equal(f.requests.length, 1);
  assert.deepEqual(f.restartRequests, [{ restartEngine: false, restartAivis: true }]);
});

test("設定画面を開き直して読み込んだ保留要求も変更せず反映できる", async () => {
  const f = fixture("patch", { pendingRestart: { restartEngine: false, restartAivis: false } });
  await f.save();
  assert.equal(f.requests.length, 0);
  assert.deepEqual(f.restartRequests, [{ restartEngine: false, restartAivis: false }]);
});

test("一般欄と複合欄の変更印は保存差分と一致し同じ値を再比較しない", () => {
  const compositeKeys = ["PICOAGENT_CHARACTER_OVERRIDES", "PICOAGENT_TTS_CHARACTER_OVERRIDES",
    "PICOAGENT_LIVE2D_SDK_OVERRIDES", "PICOAGENT_RENDERER_OVERRIDES", "PICOAGENT_ALARMS", "PICOAGENT_SCHEDULED_TASKS"];
  const values = Object.fromEntries(["NORMAL", "SECRET", "RUNTIME", ...compositeKeys].map(key => [key, ""]));
  const f = fixture("patch", { values });
  const field = () => {
    const flags = new Set();
    return { flags, classList: { toggle(name, enabled) { if (enabled) flags.add(name); else flags.delete(name); } } };
  };
  const wrappers = new Map(Object.keys(f.draft).map(key => [key, field()]));
  const composites = compositeKeys.map(field);
  let comparisons = 0;
  const stringify = f.context.stableStringify;
  f.context.stableStringify = value => { comparisons++; return stringify(value); };
  Object.assign(f.context, {
    renderingSettings: false, resetChangesBtn: {}, inputWrappers: wrappers,
    characterPersona: { closest: () => composites[0] }, characterSpeaker: { closest: () => composites[1] },
    characterLive2dSdkField: composites[2], characterRendererField: composites[3],
    form: { querySelector: selector => ({ closest: () => composites[selector === ".alarm-editor" ? 4 : 5] }), querySelectorAll: () => [] },
    dirtySummary: field(),
  });
  f.context.SETTINGS_RUNTIME_KEYS.add("RUNTIME");
  f.context.saveBtn.classList = field().classList;
  vm.runInContext(source.slice(source.indexOf("function updateDirtyStateNow()"), source.indexOf("// collect()と全設定")), f.context);
  for (const key of ["NORMAL", "RUNTIME", ...compositeKeys]) f.draft[key] = "変更";
  f.draft.SECRET = f.context.PRESERVE_UNREADABLE_SECRET;
  f.context.updateDirtyStateNow();
  assert.equal(comparisons, (Object.keys(f.draft).length - 1) * 2, "各保存対象は原本と草稿の1回ずつだけ比較する");
  for (const [key, wrapper] of wrappers) {
    assert.equal(wrapper.flags.has("changed"), key === "NORMAL" || compositeKeys.includes(key), key);
  }
  assert.ok(composites.every(wrapper => wrapper.flags.has("changed")));
  Object.assign(f.draft, structuredClone(f.context.savedSnapshot));
  f.context.updateDirtyStateNow();
  assert.ok([...wrappers.values(), ...composites].every(wrapper => !wrapper.flags.has("changed")));
});

test("未反映の設定は保存ボタンと状態欄に示し保存ショートカットでも再試行する", () => {
  const f = fixture("patch", { pendingRestart: { restartEngine: false, restartAivis: true } });
  const classes = { toggle() {} };
  let keydown;
  let clicks = 0;
  Object.assign(f.context, {
    renderingSettings: false, resetChangesBtn: {}, inputWrappers: new Map(),
    characterPersona: null, characterSpeaker: null, characterLive2dSdkField: null, characterRendererField: null,
    form: { querySelector: () => null, querySelectorAll: () => [] }, dirtySummary: { classList: classes },
    modalOpen: false, document: { addEventListener(_event, handler) { keydown = handler; } },
  });
  Object.assign(f.context.saveBtn, { classList: classes, click: () => { clicks++; } });
  vm.runInContext(source.slice(source.indexOf("function updateDirtyStateNow()"), source.indexOf("// collect()と全設定")), f.context);
  f.context.updateDirtyStateNow();
  assert.equal(f.context.saveBtn.textContent, "保存済みの設定を反映");
  assert.equal(f.context.dirtySummary.textContent, "保存済み・反映待ち");
  assert.equal(f.context.resetChangesBtn.disabled, true);
  const shortcutStart = source.indexOf('document.addEventListener("keydown", event => {');
  vm.runInContext(source.slice(shortcutStart, source.indexOf("\n\n(async () => {", shortcutStart)), f.context);
  keydown({ key: "s", ctrlKey: true, preventDefault() {} });
  assert.equal(clicks, 1);
  f.context.data.pendingRestart = null;
  f.context.updateDirtyStateNow();
  assert.equal(f.context.dirtySummary.textContent, "保存済み");
  keydown({ key: "s", ctrlKey: true, preventDefault() {} });
  assert.equal(clicks, 1, "保存済みで保留もないときは再実行しない");
});
