// alarm-runtime.js - 目覚まし画面処理の副作用を持たない状態更新。

(function registerAlarmRuntime(root) {
  "use strict";

  function markFired(firedKeys, key, maxEntries = 50) {
    if (firedKeys.has(key)) return false;
    const limit = Math.max(1, Number(maxEntries) || 50);
    // 現在のキーを追加してから全消去すると、同じ分の次回ポーリングで二重発火する。
    if (firedKeys.size >= limit) firedKeys.clear();
    firedKeys.add(key);
    return true;
  }

  const WEEKDAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
  // 分ちょうどを逃したときに拾い直す猶予。スリープ復帰・起動直後・前の発火が長引いた場合を拾う。
  const DEFAULT_GRACE_MS = 5 * 60 * 1000;

  function pad2(value) {
    return String(value).padStart(2, "0");
  }

  function parseHhmm(time) {
    const match = /^(\d{1,2}):(\d{2})$/.exec(String(time || "").trim());
    if (!match) return null;
    const hours = Number(match[1]);
    const minutes = Number(match[2]);
    if (hours > 23 || minutes > 59) return null;
    return { hours, minutes };
  }

  function occurrenceKey(date) {
    return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())} `
      + `${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
  }

  /// `now` 以前で最も近い予定時刻（同日、無ければ前日）を返す。
  function latestOccurrence(time, now) {
    const parsed = parseHhmm(time);
    if (!parsed) return null;
    const candidate = new Date(now);
    candidate.setHours(parsed.hours, parsed.minutes, 0, 0);
    if (candidate.getTime() > now.getTime()) candidate.setDate(candidate.getDate() - 1);
    return candidate;
  }

  /// いま鳴らすべき目覚ましを1件返す（無ければ null）。
  ///
  /// 「その分ちょうど」だけを見ると、スリープ復帰・起動直後・直前の発火が長引いた場合に
  /// その日の目覚ましが黙って鳴らない。直近 `graceMs` 以内に来た予定まで拾う。
  /// 曜日は「いまの曜日」ではなく**予定時刻の曜日**で判定する（日またぎで取り違えない）。
  function dueAlarms(alarms, now = new Date(), graceMs = DEFAULT_GRACE_MS) {
    if (!Array.isArray(alarms)) return [];
    const limit = Math.max(0, Number(graceMs) || 0);
    const due = [];
    for (const alarm of alarms) {
      if (!alarm || alarm.enabled === false) continue;
      const scheduled = latestOccurrence(alarm.time, now);
      if (!scheduled) continue;
      const lateMs = now.getTime() - scheduled.getTime();
      if (lateMs < 0 || lateMs > limit) continue;
      const days = Array.isArray(alarm.days) ? alarm.days : [];
      if (days.length && !days.includes(WEEKDAYS[scheduled.getDay()])) continue;
      due.push({
        alarm,
        scheduledKey: occurrenceKey(scheduled),
        scheduledTime: `${pad2(scheduled.getHours())}:${pad2(scheduled.getMinutes())}`,
        lateMinutes: Math.floor(lateMs / 60000),
      });
    }
    return due;
  }

  function dueAlarm(alarms, now = new Date(), graceMs = DEFAULT_GRACE_MS) {
    return dueAlarms(alarms, now, graceMs)[0] || null;
  }

  root.PicoAlarmRuntime = Object.freeze({ markFired, dueAlarm, dueAlarms, DEFAULT_GRACE_MS });
})(typeof window === "undefined" ? globalThis : window);
