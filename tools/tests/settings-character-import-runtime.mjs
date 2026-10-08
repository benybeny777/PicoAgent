import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const operations = ["installRawLive2d", "installRawMmd", "installCharacterPack"];

function harness(operation) {
  let resolvePick;
  let rejectPick;
  let resolveRegister;
  let resetHandler;
  let requests = 0;
  let renders = 0;
  const states = [];
  const status = { textContent: "" };
  const context = vm.createContext({
    settingsMutationBusy: false,
    savedSnapshot: { PICOAGENT_CHARACTER: "元のキャラ" },
    resetChangesBtn: { disabled: false, addEventListener(type, handler) { resetHandler = handler; } },
    data: { values: {} },
    form: {},
    setSettingsMutationBusy(busy) { context.settingsMutationBusy = busy; states.push(busy); },
    cloneSettings: structuredClone,
    render: async () => { renders += 1; },
    updateDirtyState() {},
    errorMessage: String,
    characterCatalog: { characters: [] },
    termsInput: { checked: true },
    live2dIdInput: { value: "追加キャラ" }, live2dNameInput: { value: "追加キャラ名" },
    mmdIdInput: { value: "追加キャラ" }, mmdNameInput: { value: "追加キャラ名" },
    live2dStatus: status, addStatus: status, statusEl: status,
    window: { picoagent: { settings: {
      pickCharacterPack() {
        requests += 1;
        return new Promise((resolve, reject) => { resolvePick = resolve; rejectPick = reject; });
      },
      installLive2dCharacter: async () => ({ id: "追加キャラ" }),
      installMmdCharacter: async () => ({ id: "追加キャラ" }),
      installCharacter: async () => ({ id: "追加キャラ" }),
    } } },
    registerInstalledCharacter: () => new Promise(resolve => { resolveRegister = resolve; }),
  });
  const start = source.indexOf(`  async function ${operation}(`);
  vm.runInContext(source.slice(start, source.indexOf("\n  }", start) + 4), context);
  const resetStart = source.indexOf('resetChangesBtn?.addEventListener("click"');
  vm.runInContext(source.slice(resetStart, source.indexOf('form.addEventListener("input"', resetStart)), context);
  return {
    context, states, status,
    start: () => context[operation](false), reset: () => resetHandler(),
    resolvePick: value => resolvePick(value), rejectPick: error => rejectPick(error),
    registered: () => Boolean(resolveRegister),
    finishRegister: () => resolveRegister({ id: "追加キャラ", name: "追加キャラ名" }),
    requests: () => requests, renders: () => renders,
  };
}

test("Live2D・MMD・完成パックの追加中に設定復元や重複取り込みを許さない", async () => {
  for (const operation of operations) {
    const h = harness(operation);
    const pending = h.start();
    assert.equal(h.context.settingsMutationBusy, true, operation);
    await h.reset();
    await h.start();
    assert.equal(h.renders(), 0, "元の登録先フォームを再描画しない");
    assert.equal(h.requests(), 1, "同じ選択操作を二重に開始しない");
    h.resolvePick({ path: "選択したモデル.zip" });
    for (let count = 0; !h.registered() && count < 10; count += 1) await Promise.resolve();
    assert.equal(h.registered(), true);
    assert.equal(h.context.settingsMutationBusy, true, "選択後の登録完了まで操作を止める");
    h.finishRegister();
    await pending;
    assert.equal(h.context.settingsMutationBusy, false);
    assert.deepEqual(h.states, [true, false]);
    assert.match(h.status.textContent, /追加して選びました/);
  }
});

test("モデル追加の取消・失敗で設定操作を必ず再開できる", async () => {
  for (const operation of operations) {
    for (const outcome of ["cancel", "error"]) {
      const h = harness(operation);
      const pending = h.start();
      if (outcome === "cancel") h.resolvePick(null);
      else h.rejectPick(new Error("選択できません"));
      await pending;
      assert.equal(h.context.settingsMutationBusy, false, `${operation}/${outcome}`);
      assert.deepEqual(h.states, [true, false]);
      assert.equal(h.registered(), false);
    }
  }
});
