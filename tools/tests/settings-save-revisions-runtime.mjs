import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const revision = value => value === undefined ? "missing" : JSON.stringify(value);
const revisions = values => Object.fromEntries(Object.entries(values).map(([key, value]) => [key, revision(value)]));

function fixture(responseScope, options = {}) {
  const initial = { PICOAGENT_THEME: "light", PICOAGENT_ALARMS: JSON.stringify([{ id: "A", label: "元の目覚まし" }]) };
  const latest = structuredClone(initial);
  const draft = structuredClone(initial);
  const requests = [];
  const restartRequests = [];
  const confirmations = [...(options.confirmations || [])];
  const restartFailures = [...(options.restartFailures || [])];
  let confirmationCount = 0;
  let save;
  let rejected = false;
  const context = vm.createContext({
    saveBtn: { addEventListener(_event, listener) { save = listener; } },
    settingsMutationBusy: false, savedSnapshot: structuredClone(initial),
    data: { values: structuredClone(initial), valueRevisions: revisions(initial), pendingRestart: options.pendingRestart || null },
    SETTINGS_RUNTIME_KEYS: new Set(), PRESERVE_UNREADABLE_SECRET: "秘密値維持",
    modelLoadGeneration: 0, statusEl: {},
    validateSettingsNumberFields: () => true, setSettingsMutationBusy() {},
    collect: () => structuredClone(draft), clearNumberErrors() {}, updateDirtyState() {},
    logApp() {}, errorDetail: String, errorMessage: String,
    confirmRestart: async () => { confirmationCount++; return confirmations.shift() ?? true; },
    window: { picoagent: {
      async restartApp(restartEngine, restartAivis) {
        restartRequests.push({ restartEngine, restartAivis });
        const failure = restartFailures.shift();
        if (failure) throw new Error(failure);
      },
      settings: { async save(patch, baseRevisions) {
      const conflicts = Object.keys(baseRevisions).filter(key => baseRevisions[key] !== revision(latest[key]));
      requests.push({ patch: structuredClone(patch), baseRevisions: structuredClone(baseRevisions), conflicts });
      if (conflicts.length) {
        rejected = true;
        // この検査は再描画ではなく保存境界を対象とし、競合した原本には触れない。
        throw new Error("別の操作で更新されています");
      }
      Object.assign(latest, patch);
      return { up: true, valueRevisions: revisions(responseScope === "all" ? latest : patch), ...options.restartPlan };
    } } } },
  });
  vm.runInContext(source.slice(source.indexOf("function cloneSettings("), source.indexOf("function updateDirtyStateNow(")), context);
  const restartStart = source.indexOf("async function applyPendingSettingsRestart(");
  if (restartStart >= 0) vm.runInContext(source.slice(restartStart, source.indexOf("function clearNumberErrors(", restartStart)), context);
  vm.runInContext(source.slice(source.indexOf('saveBtn.addEventListener("click"'), source.indexOf("resetChangesBtn?.addEventListener")), context);
  return { context, draft, latest, requests, restartRequests, confirmationCount: () => confirmationCount, save: () => save(), rejected: () => rejected };
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
