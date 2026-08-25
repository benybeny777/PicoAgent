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

// 曜日ラベルの組み立てが `document.createTextNode` を使うため、行を1つでも作る検査には
// 最小限の `document` が要る。これが無いと保存値を組み立てる本体を一度も通らない。
globalThis.document ??= { createTextNode: text => ({ tag: "#text", text }) };

function findByClass(node, className) {
  if (node?.className === className) return node;
  for (const child of node?.children || []) {
    const hit = findByClass(child, className);
    if (hit) return hit;
  }
  return null;
}

function createEditors() {
  let alarmState;
  let scheduleState;
  const editors = schedules.create({
    el: stubElement,
    setAlarmEditorState: state => { alarmState = state; },
    setScheduleEditorState: state => { scheduleState = state; },
  });
  return { editors, alarm: () => alarmState, schedule: () => scheduleState };
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

// 保存済みの目覚ましを行へ展開し、そのまま保存し直しても内容が変わらないこと。
// 行の組み立てと `getJson` は設定本体から移設した部分そのもので、ここを通さないと
// 項目名の取り違えや既定値の混入に気づけない。
test("目覚ましは読み込んだ内容を変えずに保存し直す", () => {
  const stored = [
    { id: "al1", time: "06:30", days: ["mon", "fri"], repeat: false, label: "起床", enabled: true, snooze_minutes: 9 },
    { id: "al2", time: "22:00", days: [], repeat: true, label: "", enabled: false, snooze_minutes: 5 },
  ];
  const { editors, alarm } = createEditors();
  editors.buildAlarmEditor(JSON.stringify(stored));
  assert.deepEqual(JSON.parse(alarm().getJson()), stored);
});

// 未設定の目覚ましへ入る既定値。移設で既定が変わると、保存した瞬間に利用者の設定が動く。
test("目覚ましの既定値は時刻07:00・繰り返しあり・有効・5分後", () => {
  const { editors, alarm } = createEditors();
  editors.buildAlarmEditor('[{"id":"al1"}]');
  assert.deepEqual(JSON.parse(alarm().getJson()), [
    { id: "al1", time: "07:00", days: [], repeat: true, label: "", enabled: true, snooze_minutes: 5 },
  ]);
});

// cron を入れた行は時刻欄を空にして保存する（両方が効いて二重に実行されるのを防ぐ）。
test("スケジュールはcronを入れた行の時刻を空にする", () => {
  const { editors, schedule } = createEditors();
  editors.buildScheduleEditor(JSON.stringify([
    { id: "st1", kind: "tool", time: "09:00", cron: "0 9 * * 1", tool_name: "weather", tool_args: {} },
    { id: "st2", kind: "tool", time: "10:30", cron: "", tool_name: "weather", tool_args: {} },
  ]));
  const saved = JSON.parse(schedule().getJson());
  assert.equal(saved[0].cron, "0 9 * * 1");
  assert.equal(saved[0].time, "");
  assert.equal(saved[1].cron, "");
  assert.equal(saved[1].time, "10:30");
});

// tool_args が JSON として読めない行は、黙って空オブジェクトへ落とさず保存を止める。
// 落とすと、利用者が書いた引数が消えたまま保存が成功したように見える。
test("スケジュールの壊れたtool_argsは保存を中止する", () => {
  const { editors, schedule } = createEditors();
  const wrap = editors.buildScheduleEditor(JSON.stringify([
    { id: "st1", kind: "tool", label: "天気", tool_name: "weather", tool_args: {} },
  ]));
  findByClass(wrap, "schedule-json").value = "{ こわれ";
  assert.throws(() => schedule().getJson(), /「天気」の tool_args/);
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
