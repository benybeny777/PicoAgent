import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/alarm-runtime.js");

const index = await readFile(new URL("../../ui/index.html", import.meta.url), "utf8");
const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");

test("上限到達時も現在の発火記録を消さない", () => {
  const fired = new Set(Array.from({ length: 50 }, (_, index) => `old-${index}`));
  const key = "alarm-1@2026-08-18 07:00";

  assert.equal(globalThis.PicoAlarmRuntime.markFired(fired, key), true);
  assert.equal(fired.size, 1);
  assert.equal(fired.has(key), true);
  assert.equal(globalThis.PicoAlarmRuntime.markFired(fired, key), false);
});

test("実画面の目覚まし判定が共通の発火記録を使う", () => {
  assert.ok(index.indexOf('<script src="alarm-runtime.js') < index.indexOf('<script src="renderer.js'));
  assert.match(renderer, /PicoAlarmRuntime\.markFired\(firedAlarms, key\)/);
});

const at = (text) => new Date(text);

test("分ちょうどを逃しても直近5分以内なら拾う", () => {
  const alarms = [{ id: "a1", time: "07:00", enabled: true, days: [] }];
  // ちょうど。
  const exact = globalThis.PicoAlarmRuntime.dueAlarm(alarms, at("2026-09-02T07:00:10"));
  assert.equal(exact?.alarm.id, "a1");
  assert.equal(exact?.scheduledTime, "07:00");
  assert.equal(exact?.lateMinutes, 0);
  // スリープ復帰などで3分遅れ。予定時刻を添えられるよう遅れ分を返す。
  const late = globalThis.PicoAlarmRuntime.dueAlarm(alarms, at("2026-09-02T07:03:30"));
  assert.equal(late?.lateMinutes, 3);
  assert.equal(late?.scheduledKey, "2026-09-02 07:00");
  // 5分を超えたら拾わない（何時間も前の目覚ましを起動時に鳴らさない）。
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm(alarms, at("2026-09-02T07:06:00")), null);
  // 予定より前は鳴らさない。
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm(alarms, at("2026-09-02T06:59:59")), null);
});

test("曜日は予定時刻の曜日で判定する", () => {
  // 2026-09-02 は水曜。前日 2026-09-01 は火曜。
  const wednesday = [{ id: "a2", time: "07:00", enabled: true, days: ["wed"] }];
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm(wednesday, at("2026-09-02T07:02:00"))?.alarm.id, "a2");
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm(wednesday, at("2026-09-03T07:02:00")), null);

  // 日またぎ: 23:58の目覚ましを翌00:01に拾うとき、曜日は前日（火曜）で見る。
  const tuesdayNight = [{ id: "a3", time: "23:58", enabled: true, days: ["tue"] }];
  const rolled = globalThis.PicoAlarmRuntime.dueAlarm(tuesdayNight, at("2026-09-02T00:01:00"));
  assert.equal(rolled?.alarm.id, "a3");
  assert.equal(rolled?.scheduledKey, "2026-09-01 23:58");
  // 今日（水曜）指定なら、前日の予定は拾わない。
  const wednesdayNight = [{ id: "a4", time: "23:58", enabled: true, days: ["wed"] }];
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm(wednesdayNight, at("2026-09-02T00:01:00")), null);
});

test("無効・壊れた指定は鳴らさない", () => {
  const now = at("2026-09-02T07:01:00");
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm([{ id: "off", time: "07:00", enabled: false }], now), null);
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm([{ id: "broken", time: "25:99", enabled: true }], now), null);
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm([{ id: "empty", time: "", enabled: true }], now), null);
  assert.equal(globalThis.PicoAlarmRuntime.dueAlarm(null, now), null);
});

test("同じ予定は発火記録で二重に鳴らない", () => {
  const alarms = [{ id: "a5", time: "07:00", enabled: true, days: [] }];
  const fired = new Set();
  const first = globalThis.PicoAlarmRuntime.dueAlarm(alarms, at("2026-09-02T07:00:05"));
  const second = globalThis.PicoAlarmRuntime.dueAlarm(alarms, at("2026-09-02T07:02:05"));
  // 遅れて拾い直しても予定時刻が同じなら記録キーも同じ。
  assert.equal(first.scheduledKey, second.scheduledKey);
  assert.equal(globalThis.PicoAlarmRuntime.markFired(fired, `${first.alarm.id}@${first.scheduledKey}`), true);
  assert.equal(globalThis.PicoAlarmRuntime.markFired(fired, `${second.alarm.id}@${second.scheduledKey}`), false);
});

test("同時刻の先頭が発火済みでも次の目覚ましを選べる", () => {
  const alarms = [
    { id: "first", time: "07:00", enabled: true, days: [] },
    { id: "second", time: "07:00", enabled: true, days: [] },
  ];
  const dueItems = globalThis.PicoAlarmRuntime.dueAlarms(alarms, at("2026-09-02T07:01:00"));
  assert.deepEqual(dueItems.map((item) => item.alarm.id), ["first", "second"]);

  const fired = new Set([`${dueItems[0].alarm.id}@${dueItems[0].scheduledKey}`]);
  const next = dueItems.find((item) => !fired.has(`${item.alarm.id}@${item.scheduledKey}`));
  assert.equal(next?.alarm.id, "second");
});

test("実画面は共通の判定と発火記録を使う", () => {
  assert.match(renderer, /PicoAlarmRuntime\.dueAlarms\(alarms, now\)/);
  assert.match(renderer, /PicoAlarmRuntime\.markFired\(firedAlarms, key\)/);
  // 遅れて鳴らすときは予定時刻を添える（いまの時刻の目覚ましだと誤解させない）。
  assert.match(renderer, /due\.lateMinutes >= 1/);
});
