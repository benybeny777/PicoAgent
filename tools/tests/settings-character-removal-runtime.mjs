import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const key = "PICOAGENT_CHARACTER";

function fixture({ selected, saved, response, failure }) {
  let remove;
  let removedCard = false;
  const switched = [];
  const busy = [];
  const context = vm.createContext({
    removeButton: { addEventListener(_event, handler) { remove = handler; } },
    removeConfirmation: { request: () => true }, removeStatus: {}, statusEl: {},
    character: { id: "deleted", name: "削除するキャラ" },
    characterSelect: { value: selected, options: ["deleted", "mugi", "kept", "draft"].map(value => ({ value, remove() {} })) },
    characterCatalog: { default: "mugi", characters: [{ id: "deleted" }, { id: "mugi" }, { id: "kept" }, { id: "draft" }] },
    card: { remove() { removedCard = true; } },
    savedSnapshot: { [key]: saved, OTHER: "未変更" },
    data: { values: { [key]: saved, OTHER: "未変更" }, valueRevisions: { [key]: "古い版", OTHER: "別項目の版" } },
    characterManifestCache: new Map([["deleted", {}]]), characterDefaultPersonas: { deleted: "人格" }, characterDefaultVoices: { deleted: "声" },
    setSettingsMutationBusy: value => busy.push(value), queueCharacterSwitch: async id => switched.push(id),
    applyCharacterFilters() {}, errorMessage: error => error.message,
    SETTINGS_RUNTIME_KEYS: new Set(), PRESERVE_UNREADABLE_SECRET: "秘密値維持",
    window: { picoagent: { settings: { async removeCharacter(id) {
      assert.equal(id, "deleted");
      if (failure) throw new Error(failure);
      return response;
    } } } },
  });
  vm.runInContext(source.slice(source.indexOf("function cloneSettings("), source.indexOf("function updateDirtyStateNow(")), context);
  const start = source.indexOf('removeButton.addEventListener("click", async () => {');
  vm.runInContext(source.slice(start, source.indexOf("texts.push(removeStatus, removeButton);", start)), context);
  return { context, switched, busy, removedCard: () => removedCard, remove: () => remove() };
}

test("削除後の保存選択と版を同期し実行中キャラや別の草稿へ読み替えない", async () => {
  for (const [selected, saved, savedAfter, selectedAfter, expectedSwitch] of [
    ["deleted", "deleted", "mugi", "mugi", ["mugi"]],
    ["deleted", "kept", "kept", "kept", ["kept"]],
    ["draft", "deleted", "mugi", "draft", []],
    ["draft", "kept", "kept", "draft", []],
  ]) {
    const f = fixture({ selected, saved, response: { activeCharacter: "runtime", savedCharacter: savedAfter, valueRevisions: { [key]: "削除後の版" } } });
    await f.remove();
    assert.equal(f.removedCard(), true);
    assert.equal(f.context.characterSelect.value, selectedAfter);
    assert.equal(f.context.savedSnapshot[key], savedAfter);
    assert.equal(f.context.data.values[key], savedAfter);
    assert.equal(f.context.data.valueRevisions[key], "削除後の版");
    assert.equal(f.context.data.valueRevisions.OTHER, "別項目の版");
    assert.deepEqual(f.switched, expectedSwitch);
    const request = f.context.settingsChangeRequest(
      { [key]: selectedAfter, OTHER: "未変更" }, f.context.savedSnapshot, f.context.data.valueRevisions,
    );
    assert.deepEqual({ ...request.patch }, selectedAfter === savedAfter ? {} : { [key]: selectedAfter });
    if (selectedAfter !== savedAfter) assert.equal(request.baseRevisions[key], "削除後の版");
    assert.deepEqual(f.busy, [true, false]);
  }
});

test("キャラ削除の失敗では保存選択と版と草稿と一覧を保持する", async () => {
  const f = fixture({ selected: "draft", saved: "deleted", failure: "ゴミ箱へ移動できません" });
  await f.remove();
  assert.equal(f.removedCard(), false);
  assert.equal(f.context.characterSelect.value, "draft");
  assert.equal(f.context.savedSnapshot[key], "deleted");
  assert.equal(f.context.data.values[key], "deleted");
  assert.equal(f.context.data.valueRevisions[key], "古い版");
  assert.deepEqual(f.switched, []);
  assert.match(f.context.removeStatus.textContent, /削除できませんでした.*ゴミ箱/);
  assert.equal(f.context.removeButton.disabled, false);
  assert.deepEqual(f.busy, [true, false]);
});

test("表示中キャラの削除後は未変更でも再初期化を実行できる状態を引き継ぐ", async () => {
  const pendingRestart = { restartEngine: false, restartAivis: false };
  const f = fixture({ selected: "deleted", saved: "deleted", response: {
    savedCharacter: "mugi", valueRevisions: { [key]: "削除後の版" }, pendingRestart,
  } });
  await f.remove();
  assert.deepEqual(f.context.data.pendingRestart, pendingRestart);
  assert.match(f.context.statusEl.textContent, /保存済みの設定を反映/);
});
