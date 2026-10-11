import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const revision = value => value === undefined ? "missing" : JSON.stringify(value);
const revisions = values => Object.fromEntries(Object.entries(values).map(([key, value]) => [key, revision(value)]));

function fixture(responseScope) {
  const initial = { PICOAGENT_THEME: "light", PICOAGENT_ALARMS: JSON.stringify([{ id: "A", label: "元の目覚まし" }]) };
  const latest = structuredClone(initial);
  const draft = structuredClone(initial);
  const requests = [];
  let save;
  let rejected = false;
  const context = vm.createContext({
    saveBtn: { addEventListener(_event, listener) { save = listener; } },
    settingsMutationBusy: false, savedSnapshot: structuredClone(initial),
    data: { values: structuredClone(initial), valueRevisions: revisions(initial) },
    SETTINGS_RUNTIME_KEYS: new Set(), PRESERVE_UNREADABLE_SECRET: "秘密値維持",
    modelLoadGeneration: 0, statusEl: {},
    validateSettingsNumberFields: () => true, setSettingsMutationBusy() {},
    collect: () => structuredClone(draft), clearNumberErrors() {}, updateDirtyState() {},
    logApp() {}, errorDetail: String, errorMessage: String,
    window: { picoagent: { settings: { async save(patch, baseRevisions) {
      const conflicts = Object.keys(baseRevisions).filter(key => baseRevisions[key] !== revision(latest[key]));
      requests.push({ patch: structuredClone(patch), baseRevisions: structuredClone(baseRevisions), conflicts });
      if (conflicts.length) {
        rejected = true;
        // この検査は再描画ではなく保存境界を対象とし、競合した原本には触れない。
        throw new Error("別の操作で更新されています");
      }
      Object.assign(latest, patch);
      return { up: true, valueRevisions: revisions(responseScope === "all" ? latest : patch) };
    } } } },
  });
  vm.runInContext(source.slice(source.indexOf("function cloneSettings("), source.indexOf("function updateDirtyStateNow(")), context);
  vm.runInContext(source.slice(source.indexOf('saveBtn.addEventListener("click"'), source.indexOf("resetChangesBtn?.addEventListener")), context);
  return { context, draft, latest, requests, save: () => save(), rejected: () => rejected };
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
