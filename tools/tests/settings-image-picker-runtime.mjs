import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const start = settings.indexOf("function bindStreamImagePicker(");
const source = settings.slice(start, settings.indexOf("function streamSettingsField(", start));

function harness() {
  let resolvePick;
  let rejectPick;
  let requests = 0;
  let available = true;
  const handlers = {};
  const button = { disabled: false, addEventListener(type, callback) { handlers[type] = callback; } };
  const paths = [];
  const busyStates = [];
  const context = vm.createContext({
    settingsMutationBusy: false,
    statusEl: { textContent: "" },
    window: { picoagent: { settings: { pickStreamImage() {
      requests += 1;
      return new Promise((resolve, reject) => { resolvePick = resolve; rejectPick = reject; });
    } } } },
    setSettingsMutationBusy(busy) { context.settingsMutationBusy = busy; busyStates.push(busy); },
    logApp() {}, errorDetail: String, errorMessage: String,
  });
  vm.runInContext(source, context);
  context.bindStreamImagePicker(button, "background", "キャラ用背景画像", path => paths.push(path), () => available);
  return {
    context, button, paths, busyStates,
    click: () => handlers.click(),
    resolve: value => resolvePick(value),
    reject: error => rejectPick(error),
    setAvailable: value => { available = value; },
    requests: () => requests,
  };
}

test("画像取り込み中は保存・設定復元・キャラ切替を止め、反映後に解除する", async () => {
  const h = harness();
  const pending = h.click();
  assert.equal(h.context.settingsMutationBusy, true);
  assert.equal(h.button.disabled, true);
  await h.click();
  assert.equal(h.requests(), 1);
  h.resolve({ path: "選択した画像.png" });
  await pending;
  assert.deepEqual(h.paths, ["選択した画像.png"]);
  assert.deepEqual(h.busyStates, [true, false]);
  assert.equal(h.button.disabled, false);
});

test("画像選択の取消・失敗でも設定変更ロックを残さない", async () => {
  for (const outcome of ["cancel", "error"]) {
    const h = harness();
    const pending = h.click();
    if (outcome === "cancel") h.resolve(null);
    else h.reject(new Error("画像を取り込めません"));
    await pending;
    assert.equal(h.context.settingsMutationBusy, false);
    assert.equal(h.button.disabled, false);
    assert.deepEqual(h.paths, []);
    assert.match(h.context.statusEl.textContent, outcome === "cancel" ? /キャンセル/ : /取り込めません/);
  }
});

test("別の設定変更処理中やキャラ専用設定OFFでは画像取得を開始しない", async () => {
  const h = harness();
  h.context.settingsMutationBusy = true;
  await h.click();
  assert.equal(h.requests(), 0);
  assert.equal(h.context.settingsMutationBusy, true);
  h.context.settingsMutationBusy = false;
  h.setAvailable(false);
  await h.click();
  assert.equal(h.requests(), 0);
  assert.deepEqual(h.busyStates, []);
});
