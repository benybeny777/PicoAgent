// 明示したCLI検出から草稿・既存保存へ渡す境界。実GUIと実CLIは使わない。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import { installFakeDom } from "./lib/fake-dom.mjs";

const source = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const declaration = readFileSync(new URL("../../src-tauri/src/tools/cli_detection.rs", import.meta.url), "utf8");
const key = declaration.match(/MAGICK_SETTING_KEY: &str = "([^"]+)"/)[1];
const section = (start, end) => source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));

function fixture() {
  const document = installFakeDom();
  const draft = { [key]: "旧設定" };
  let requests = 0;
  let saves = 0;
  let resolve;
  let reject;
  let saveFailure = false;
  const saveBtn = document.createElement("button");
  const context = vm.createContext({
    document, settingsMutationBusy: false, errorMessage: String, errorDetail: String,
    savedSnapshot: { ...draft }, data: { values: { ...draft }, valueRevisions: { [key]: "旧revision" } },
    SETTINGS_RUNTIME_KEYS: new Set(), PRESERVE_UNREADABLE_SECRET: "秘密値維持", modelLoadGeneration: 0,
    saveBtn, statusEl: { textContent: "" },
    collect: () => ({ ...draft }), validateSettingsNumberFields: () => true,
    clearNumberErrors() {}, updateDirtyState() {}, logApp() {},
    setSettingsMutationBusy(value) { context.settingsMutationBusy = value; },
    window: { picoagent: { settings: { async save(patch, revisions) {
      saves += 1;
      assert.equal(patch[key], "C:/ImageMagick/convert.exe");
      assert.equal(revisions[key], "旧revision");
      if (saveFailure) throw new Error("保存先が書込不可");
      return { up: true, valueRevisions: { [key]: "新revision" } };
    } } } },
  });
  vm.runInContext(section("function el(", "const { buildAlarmEditor"), context);
  vm.runInContext(section("function cloneSettings(", "function updateDirtyStateNow("), context);
  vm.runInContext(section("function buildCliDetection(", "function componentPresentation("), context);
  vm.runInContext(section('saveBtn.addEventListener("click",', 'resetChangesBtn?.addEventListener('), context);
  const element = context.buildCliDetection(id => {
    assert.equal(id, "imagemagick");
    requests += 1;
    return new Promise((yes, no) => { resolve = yes; reject = no; });
  }, (settingKey, path) => { assert.equal(settingKey, key); draft[key] = path; });
  const button = element.querySelector(".cli-detect");
  const choices = element.querySelector(".cli-candidates");
  const apply = element.querySelector(".cli-apply");
  const note = element.querySelector(".cli-diagnostic");
  return {
    context, draft, button, choices, applyButton: apply, note,
    click: () => button.listeners.click[0](),
    choose(path) { choices.value = path; choices.listeners.change[0](); },
    apply: () => apply.listeners.click[0](),
    save: () => saveBtn.listeners.click[0](),
    resolve: value => resolve(value), reject: value => reject(value),
    failSave(value) { saveFailure = value; },
    requests: () => requests, saves: () => saves,
  };
}
const report = () => ({ settingKey: key, candidates: [{ path: "C:/ImageMagick/convert.exe", version: "6.9.13-38" }], warnings: [] });

test("検出と候補選択は保存ゼロで明示保存だけ一回実行する", async () => {
  const f = fixture();
  assert.equal(f.requests(), 0);
  const pending = f.click();
  assert.equal(f.button.disabled, true);
  await f.click();
  assert.equal(f.requests(), 1);
  f.resolve(report());
  await pending;
  assert.equal(f.button.disabled, false);
  assert.equal(f.draft[key], "旧設定");
  assert.equal(f.saves(), 0);
  assert.equal(f.choices.value, "");
  f.choose("C:/ImageMagick/convert.exe");
  assert.equal(f.applyButton.disabled, false);
  f.apply();
  assert.equal(f.draft[key], "C:/ImageMagick/convert.exe");
  assert.equal(f.saves(), 0);
  await f.save();
  assert.equal(f.saves(), 1);
  assert.equal(f.context.savedSnapshot[key], f.draft[key]);
});

test("保存失敗では草稿と旧設定を保持して理由を表示する", async () => {
  const f = fixture();
  const pending = f.click();
  f.resolve(report());
  await pending;
  f.choose("C:/ImageMagick/convert.exe");
  f.apply();
  f.failSave(true);
  await f.save();
  assert.equal(f.saves(), 1);
  assert.equal(f.draft[key], "C:/ImageMagick/convert.exe");
  assert.equal(f.context.savedSnapshot[key], "旧設定");
  assert.equal(f.context.data.values[key], "旧設定");
  assert.match(f.context.statusEl.textContent, /保存先が書込不可/);
  assert.equal(f.context.settingsMutationBusy, false);
});

test("未発見と検出失敗を別に知らせて何度でも再操作できる", async () => {
  const f = fixture();
  let pending = f.click();
  f.resolve({ settingKey: key, candidates: [], warnings: [] });
  await pending;
  assert.match(f.note.textContent, /見つかりません/);
  assert.equal(f.choices.disabled, true);
  pending = f.click();
  f.reject(new Error("別製品の名乗り"));
  await pending;
  assert.match(f.note.textContent, /検出できませんでした.*別製品/);
  assert.equal(f.button.disabled, false);
  assert.equal(f.saves(), 0);
  pending = f.click();
  f.resolve(report());
  await pending;
  assert.equal(f.requests(), 3);
  f.choose("C:/forged.exe");
  assert.equal(f.applyButton.disabled, true);
  f.apply();
  assert.equal(f.draft[key], "旧設定");
});

test("破損した検出結果は選択へ進まず橋とinvokeは固定IDだけを渡す", async () => {
  const f = fixture();
  const pending = f.click();
  f.resolve({ settingKey: key, candidates: [{ path: 3 }], warnings: [] });
  await pending;
  assert.match(f.note.textContent, /形式が不正/);
  assert.equal(f.applyButton.disabled, true);
  const bridge = readFileSync(new URL("../../ui/picoagent.js", import.meta.url), "utf8");
  const commands = readFileSync(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
  const entry = readFileSync(new URL("../../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(bridge, /detectCli: \(id\) => call\("settings_detect_cli", \{ id \}\)/);
  assert.match(commands, /settings_detect_cli\(id: String\)/);
  assert.match(entry, /commands::settings_detect_cli,/);
  assert.match(source, /item.id === "tools"/);
  assert.match(source, /input.value = path;\s*input.dispatchEvent/);
});

test("長い検出パスと失敗理由は全文を隠さず確定したカード幅で折り返す", () => {
  const css = readFileSync(new URL("../../ui/settings.css", import.meta.url), "utf8");
  const declaration = selector => {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    return css.match(new RegExp(escaped + "\\s*\\{([^}]+)\\}"))?.[1] || "";
  };
  assert.match(declaration(".cli-detection"), /overflow-wrap\s*:\s*anywhere/);
  for (const selector of [".cli-detection", ".cli-detection select"]) {
    assert.match(declaration(selector), /min-width\s*:\s*0/);
    assert.match(declaration(selector), /max-width\s*:\s*100%/);
  }
  assert.match(source, /className: "field cli-detection"/);
  assert.match(source, /licenseConsent,\s*cliDetection,/);
});
