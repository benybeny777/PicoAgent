// ambient.js - 利用者の操作を待たずに起きること（自発雑談・リマインダー・目覚まし・居眠り）の持ち主。

import * as bridge from "./bridge.js";
import { clearBackgroundFailure, notifyBackgroundFailure, toast } from "./toast.js";
import { errorDetail, logApp, waitWithLimit } from "./util.js";

const POLL_MS = 20000;
// 目覚ましの起こし文の生成と音声合成は遅れることがある。「鳴らなかった」がいちばん困るので、待つ上限を決めて文字を先に出す。
const ALARM_WAKE_TIMEOUT_MS = 20000;
const ALARM_SPEECH_TIMEOUT_MS = 20000;
const ALARM_FALLBACK_LINE = "おはよう…そろそろ起きる時間だよ。ゆっくりでいいからね。";

function snoozeMinutes(alarm) {
  const value = Number(alarm?.snooze_minutes ?? alarm?.snoozeMinutes ?? 5);
  return Number.isFinite(value) ? Math.min(60, Math.max(1, Math.round(value))) : 5;
}

export function createAmbient({ chat, voice, host, motion, isBusy, isExpanded }) {
  const { status, history, composer } = chat;
  const chatter = { enabled: false, minSecs: 90, maxSecs: 240 };
  const queue = [];
  const firedAlarms = new Set();
  let chatterTimer = null;
  let dailyTimer = null;
  let alarmTimer = null;
  let napTimer = null;
  let napMinutes = 15;
  let lastActivityAt = Date.now();
  let revision = 0;
  let snooze = null;
  let alarmCheckInFlight = false;

  const canShow = () => !isBusy() && !composer.isOpen && !voice.speaking && !isExpanded();

  function wake() {
    if (!host.sleeping) return;
    host.setSleeping(false);
    status.clear("nap");
    motion.rest();
  }

  function markActivity() {
    lastActivityAt = Date.now();
    wake();
  }

  function fallAsleep() {
    // 全画面で履歴を読んでいる間（マウス操作なしでも閲覧中）は寝ない。
    if (host.sleeping || !canShow()) return;
    if (host.setSleeping(true)) status.set("すやすや……", "nap");
  }

  /** キャラから話しかける（雑談・リマインダー・初回のあいさつ）。今は話せなければ順番待ちにする。 */
  async function say(line, { enqueue = true } = {}) {
    const text = String(line ?? "").trim();
    if (!text) return;
    wake(); // 寝たまま話さない（無操作タイマーは進めない: 雑談で永遠に寝られなくなるため）
    const requestRevision = revision;
    if (!canShow()) {
      if (enqueue) { queue.push(text); if (queue.length > 6) queue.shift(); }
      return;
    }
    const prepared = await voice.prepare(text);
    if (requestRevision !== revision) return;
    if (!canShow()) {
      if (enqueue) { queue.push(text); if (queue.length > 6) queue.shift(); }
      return;
    }
    chat.showAnswer(text, { paginate: false });
    history.add("assistant", text);
    if (prepared) voice.playPrepared(prepared);
  }

  async function flushNotice() {
    if (!queue.length || !canShow()) return;
    await say(queue.shift(), { enqueue: false });
  }

  function scheduleChatter() {
    if (chatterTimer) { clearTimeout(chatterTimer); chatterTimer = null; }
    if (!chatter.enabled) return;
    chatterTimer = setTimeout(fireChatter, (chatter.minSecs + Math.random() * (chatter.maxSecs - chatter.minSecs)) * 1000);
  }

  async function fireChatter() {
    chatterTimer = null;
    // 入力中・応答中・読み上げ中・全画面表示中・居眠り中は割り込まず、次の機会に回す。
    if (!canShow() || host.sleeping) { scheduleChatter(); return; }
    try {
      const result = await bridge.idleChatter();
      if (result?.enabled === false) { chatter.enabled = false; return; }
      await say(result?.reply || "");
    } catch (error) {
      logApp("warn", "自発雑談の取得に失敗しました", errorDetail(error));
    } finally {
      scheduleChatter();
    }
  }

  async function checkDaily() {
    let result;
    try { result = await bridge.dailyEvents(); }
    catch (error) {
      notifyBackgroundFailure("daily-events", "リマインダーを確認できないため通知が出ません。設定の「リマインダー」を確認してください", error);
      return;
    }
    clearBackgroundFailure("daily-events");
    // 日時を読めない予定は永久に発火しない。バックエンドが1件につき1度だけ返すので、届いたぶんを知らせる。
    for (const item of Array.isArray(result?.broken) ? result.broken : []) {
      const label = String(item?.text || item?.label || "").trim() || "登録した通知";
      logApp("warn", "通知の日時を解析できません", `${label} / ${item?.due_at ?? ""}`);
      toast(`「${label}」は日時を読めないため鳴りません。登録し直してください`, "error");
    }
    const events = Array.isArray(result?.events) ? result.events : [];
    if (events.length) await say(events.map(event => String(event?.text || "時間だよ。").trim()).join("\n"));
  }

  function clearSnooze(stop) {
    snooze = null;
    chat.clearAlarm();
    if (stop) voice.stop();
    status.clear("alarm");
  }

  function armSnooze(alarm) {
    const minutes = snoozeMinutes(alarm);
    snooze = { alarm: { ...alarm, snooze_minutes: minutes }, dueAt: Date.now() + minutes * 60 * 1000 };
    status.set(`「おきたよ！」を押すまで${minutes}分おきに起こします`, "alarm");
  }

  async function fireAlarm(alarm, snoozeState = null, due = null) {
    const wakeRequest = bridge.alarmWake(alarm.id).catch(error => {
      logApp("warn", "目覚まし起こし文の生成に失敗しました", errorDetail(error));
      return null;
    });
    const wakeResult = await waitWithLimit(wakeRequest, ALARM_WAKE_TIMEOUT_MS);
    if (wakeResult.timedOut) logApp("warn", "目覚ましの起こし文が時間内に届きませんでした", `${ALARM_WAKE_TIMEOUT_MS}ms`);
    let line = (wakeResult.value?.reply || "").trim() || ALARM_FALLBACK_LINE;
    // 遅れて鳴らす場合は予定時刻を添える（いまの時刻の目覚ましだと誤解させない）。
    if (due && due.lateMinutes >= 1) line = `（${due.scheduledTime}の目覚ましだよ）${line}`;
    markActivity(); // 居眠り中でも目覚ましでは起きる
    // 声より先に文字を出す。操作ボタンをページ送りで消さない。
    chat.showAlarm(line, () => clearSnooze(true));
    history.add("assistant", line);
    // 生成中に「おきたよ！」で解除された場合は、完了後にスヌーズを復活させない。
    const shouldArm = !snoozeState || snooze === snoozeState;
    if (shouldArm) armSnooze(alarm);
    const armed = shouldArm ? snooze : null;
    const speechRequest = voice.prepare(line).catch(error => {
      logApp("warn", "目覚ましの音声合成に失敗しました", errorDetail(error));
      return null;
    });
    const speech = await waitWithLimit(speechRequest, ALARM_SPEECH_TIMEOUT_MS);
    // 数分後に突然鳴るほうが困るため、遅れて届いた音声は鳴らさない。
    if (speech.timedOut) logApp("warn", "目覚ましの音声が時間内に用意できませんでした", `${ALARM_SPEECH_TIMEOUT_MS}ms`);
    if (armed && !speech.timedOut && speech.value && snooze === armed) voice.playPrepared(speech.value);
  }

  async function checkAlarms() {
    if (alarmCheckInFlight) return;
    alarmCheckInFlight = true;
    try {
      const now = new Date();
      if (snooze && Date.now() >= snooze.dueAt) {
        const state = snooze;
        // 遅い音声合成を待つ間に次のポーリングが同じスヌーズを発火しないよう、先に期限を進める。
        snooze.dueAt = Date.now() + snoozeMinutes(state.alarm) * 60 * 1000;
        await fireAlarm(state.alarm, state);
        return;
      }
      let result;
      try { result = await bridge.alarms(); }
      catch (error) {
        notifyBackgroundFailure("alarms", "目覚ましの一覧を読めないため鳴りません。設定の「目覚まし時計」を確認してください", error);
        return;
      }
      clearBackgroundFailure("alarms");
      const alarms = result?.alarms;
      if (!Array.isArray(alarms) || !alarms.length) return;
      // 分ちょうどだけを見ると、スリープ復帰・起動直後に鳴らない。直近5分以内に来た予定まで拾う。
      for (const due of window.PicoAlarmRuntime.dueAlarms(alarms, now)) {
        if (!window.PicoAlarmRuntime.markFired(firedAlarms, `${due.alarm.id}@${due.scheduledKey}`)) continue;
        await fireAlarm(due.alarm, null, due);
        break; // 1ティックにつき1件だけ鳴らす
      }
    } finally {
      alarmCheckInFlight = false;
    }
  }

  ["pointerdown", "keydown"].forEach(type => document.addEventListener(type, markActivity));
  window.addEventListener("beforeunload", () => {
    for (const timer of [chatterTimer]) if (timer) clearTimeout(timer);
    for (const timer of [dailyTimer, alarmTimer, napTimer]) if (timer) clearInterval(timer);
  });

  return {
    say,
    flushNotice,
    markActivity,
    onUserActivity() {
      clearSnooze(false);
      markActivity();
    },
    afterConversation() {
      scheduleChatter(); // 会話後はアイドル計測をやり直す
      setTimeout(flushNotice, 0);
    },
    applyChatter(config) {
      if (!config) return;
      const seconds = (value, fallback) => {
        const numeric = parseFloat(value);
        return Number.isFinite(numeric) ? Math.min(3600, Math.max(30, numeric)) : fallback;
      };
      chatter.enabled = Boolean(config.chatterEnabled);
      chatter.minSecs = seconds(config.chatterMinSecs, 90);
      chatter.maxSecs = Math.max(chatter.minSecs, seconds(config.chatterMaxSecs, 240));
      scheduleChatter();
    },
    start({ napMinutes: nap }) {
      napMinutes = Number(nap) > 0 ? Number(nap) : 0;
      markActivity();
      dailyTimer = setInterval(checkDaily, POLL_MS);
      checkDaily();
      alarmTimer = setInterval(checkAlarms, POLL_MS);
      checkAlarms();
      napTimer = setInterval(() => {
        if (napMinutes > 0 && !host.sleeping && Date.now() - lastActivityAt > napMinutes * 60 * 1000) fallAsleep();
      }, 30 * 1000);
    },
    reset() {
      revision += 1;
      queue.length = 0;
      scheduleChatter();
    },
    get alarmActive() { return Boolean(snooze); },
  };
}
