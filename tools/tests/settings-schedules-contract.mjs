import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/settings-schedules.js");

const schedules = globalThis.PicoSettingsSchedules;
const settingsHtml = await readFile(new URL("../../ui/settings.html", import.meta.url), "utf8");
const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");

function stubElement(tag, props = {}, children = []) {
  return Object.assign({
    tag,
    children: [...children],
    handlers: {},
    append(...items) { this.children.push(...items); },
    addEventListener(type, handler) { this.handlers[type] = handler; },
    removeChild(child) { this.children = this.children.filter(item => item !== child); },
  }, props);
}

test("保存一覧は配列だけを受け付ける", () => {
  assert.deepEqual(schedules.parseStoredList('[{"id":"a"}]'), {
    items: [{ id: "a" }],
    broken: false,
    raw: '[{"id":"a"}]',
  });
  assert.equal(schedules.parseStoredList("{}").broken, true);
  assert.equal(schedules.parseStoredList("壊れたJSON").broken, true);
});

test("壊れた目覚ましとスケジュールを未編集では上書きしない", () => {
  let alarmState;
  let scheduleState;
  const editors = schedules.create({
    el: stubElement,
    setAlarmEditorState: state => { alarmState = state; },
    setScheduleEditorState: state => { scheduleState = state; },
  });
  editors.buildAlarmEditor("壊れた目覚まし");
  editors.buildScheduleEditor("壊れたスケジュール");
  assert.equal(alarmState.getJson(), "壊れた目覚まし");
  assert.equal(scheduleState.getJson(), "壊れたスケジュール");
});

test("専用モジュールを設定本体より先に読み込む", () => {
  assert.ok(
    settingsHtml.indexOf('<script src="settings-schedules.js"')
      < settingsHtml.indexOf('<script src="settings.js'),
  );
  assert.match(settings, /PicoSettingsSchedules\.create/);
  assert.doesNotMatch(settings, /function buildAlarmEditor/);
  assert.doesNotMatch(settings, /function buildScheduleEditor/);
});
