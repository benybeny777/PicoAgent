import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const start = settings.indexOf("function validateSettingsNumberFields()");
const source = settings.slice(start, settings.indexOf('saveBtn.addEventListener("click"', start));

test("不正な文字サイズ・音量は検索解除と対象タブ表示の後に警告し保存を止める", () => {
  for (const [key, tabId] of [
    ["PICOAGENT_STREAM_CHAT_FONT_PX", "display"],
    ["PICOAGENT_TTS_VOLUME", "voice"],
  ]) {
    const events = [];
    const context = vm.createContext({
      inputs: new Map([[key, {
        checkValidity: () => false,
        reportValidity() { events.push("警告"); assert.equal(context.settingsSearch.value, ""); },
      }]]),
      settingsSearch: { value: "別の設定" },
      clearSettingsSearch: { hidden: false },
      statusEl: { textContent: "" },
      activateSettingsTab(value) { events.push(value); },
    });
    vm.runInContext(source, context);
    assert.equal(context.validateSettingsNumberFields(), false);
    assert.deepEqual(events, [tabId, "警告"]);
    assert.equal(context.clearSettingsSearch.hidden, true);
    assert.match(context.statusEl.textContent, /入力内容を確認/);
  }
});

test("数値が有効なときは検索・選択タブを変えず保存へ進める", () => {
  const context = vm.createContext({
    inputs: new Map([
      ["PICOAGENT_STREAM_CHAT_FONT_PX", { checkValidity: () => true }],
      ["PICOAGENT_TTS_VOLUME", { checkValidity: () => true }],
    ]),
    settingsSearch: { value: "音声" },
    activateSettingsTab() { assert.fail("有効な入力でタブを変更しない"); },
  });
  vm.runInContext(source, context);
  assert.equal(context.validateSettingsNumberFields(), true);
  assert.equal(context.settingsSearch.value, "音声");
  assert.match(settings, /if \(!validateSettingsNumberFields\(\)\) return;/);
});
