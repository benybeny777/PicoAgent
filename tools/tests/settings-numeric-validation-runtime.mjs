import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const helper = readFileSync(new URL("../../ui/settings-number.js", import.meta.url), "utf8");
function fixture(schema, value) {
  const handlers = new Map();
  const input = { value, dataset: {}, type: "text", addEventListener: (name, callback) => handlers.set(name, callback) };
  const context = vm.createContext({ window: {} });
  vm.runInContext(helper, context);
  const api = context.window.PicoIntegerInput;
  api.attach(input, schema);
  return { input, handlers, api };
}
const schema = { range: [18, 48], unit: "px", default: "22", label: "文字サイズ" };

test("整数宣言の上下限・小数・指数・auto許可と空欄を同じ判定で検証する", () => {
  for (const [value, valid] of [["18", true], ["48", true], ["17", false], ["49", false], ["22.5", false], ["2e1", false], ["auto", false], ["", true]]) {
    const f = fixture(schema, value);
    assert.equal(f.api.reason(f.input, schema) === "", valid, value);
  }
  const auto = { ...schema, allow_auto: true };
  assert.equal(fixture(auto, "auto").input.type, "text");
  assert.equal(fixture(auto, "auto").api.reason({ value: "auto" }, auto), "");
  const wide = { range: ["0", "9223372036854775807"] };
  const f = fixture(wide, "9223372036854775808");
  assert.match(f.api.reason(f.input, wide), /9223372036854775807/);
});

test("全角ペースト・IMEと不正草稿を保持し有効な整数だけnumberへ戻す", () => {
  const f = fixture(schema, "22");
  assert.equal(f.input.type, "number");
  assert.equal(f.input.min, "18"); assert.equal(f.input.max, "48"); assert.equal(f.input.step, "1");
  f.handlers.get("focus")(); assert.equal(f.input.type, "text");
  f.input.value = "＋３１"; f.handlers.get("compositionend")(); f.handlers.get("blur")();
  assert.equal(f.input.value, "31"); assert.equal(f.input.type, "number");
  f.handlers.get("focus")(); f.input.value = "２２.５"; f.handlers.get("blur")();
  assert.equal(f.input.value, "22.5"); assert.equal(f.input.type, "text");
  assert.match(f.api.reason(f.input, schema), /整数/);
});

function saveFixture(response) {
  let callback;
  const marks = [];
  const context = vm.createContext({
    settingsMutationBusy: false, savedSnapshot: { quantity: "22" }, modelLoadGeneration: 0,
    data: { values: { quantity: "22" }, valueRevisions: {} },
    validateSettingsNumberFields: () => true,
    setSettingsMutationBusy: value => marks.push(["busy", value]), statusEl: {},
    collect: () => ({ quantity: "31", other: "草稿" }),
    settingsChangeRequest: current => ({ patch: current, baseRevisions: {} }),
    window: { picoagent: { settings: { save: async () => response } } },
    showNumberErrors: invalid => marks.push(["invalid", invalid[0].message]),
    clearNumberErrors: () => marks.push(["clear"]),
    cloneSettings: value => structuredClone(value), updateDirtyState: () => marks.push(["dirty"]),
    logApp: () => {}, errorDetail: String, errorMessage: String,
    saveBtn: { addEventListener: (_name, handler) => { callback = handler; } },
  });
  const start = settings.indexOf('saveBtn.addEventListener("click"');
  vm.runInContext(settings.slice(start, settings.indexOf('resetChangesBtn?.addEventListener', start)), context);
  return { context, marks, click: () => callback() };
}

test("バックエンドの不正拒否では全草稿と保存正本を保持し正常再保存で理由を解除する", async () => {
  const rejected = saveFixture({ conflicts: [], invalid: [{ key: "quantity", message: "範囲外です" }] });
  await rejected.click();
  assert.equal(rejected.context.savedSnapshot.quantity, "22");
  assert.equal(rejected.context.data.values.quantity, "22");
  assert.deepEqual(rejected.marks, [["busy", true], ["invalid", "範囲外です"], ["busy", false]]);
  const accepted = saveFixture({ conflicts: [], invalid: [], up: true });
  await accepted.click();
  assert.equal(accepted.context.savedSnapshot.quantity, "31");
  assert.equal(accepted.context.savedSnapshot.other, "草稿");
  assert.ok(accepted.marks.some(([name]) => name === "clear"));
});

test("実数量payload検査はnull・文字列・小数・範囲外を代替値へ置換しない", () => {
  const util = readFileSync(new URL("../../ui/app/util.js", import.meta.url), "utf8");
  const context = vm.createContext({});
  vm.runInContext(util.slice(util.indexOf("export function readIntegerSetting")).replace("export function", "function"), context);
  const payload = { numberSettings: { key: { range: ["18", "48"], unit: "px" } } };
  for (const value of [null, "31", 31.5, 49]) {
    const result = context.readIntegerSetting({ ...payload, font: value }, "key", "font");
    assert.equal(result.value, null); assert.ok(result.error);
  }
  assert.equal(context.readIntegerSetting({ ...payload, font: 31 }, "key", "font").value, 31);
  assert.equal(context.readIntegerSetting({ ...payload, font: null, invalidNumbers: [{ key: "key", message: "原本の整数が不正です" }] }, "key", "font").error, "原本の整数が不正です");
});
