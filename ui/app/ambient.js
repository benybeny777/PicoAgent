// ambient.js - 利用者の操作を待たずに起きること（自発雑談・リマインダー・目覚まし・居眠り）の持ち主。

import * as bridge from "./bridge.js";
import { clearBackgroundFailure, notifyBackgroundFailure, toast } from "./toast.js";
import { errorDetail, errorMessage, logApp, waitWithLimit, readIntegerSetting } from "./util.js";

const POLL_MS = 20000;
// 会話ターン待機と生成上限はバックエンドが管理する。音声は文字表示後に上限まで待つ。
const ALARM_SPEECH_TIMEOUT_MS = 20000;

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
  let napMinutes = null;
  let lastActivityAt = Date.now();
  let revision = 0;
  let snooze = null;
  let alarmCheckInFlight = false;
  let alarmJob = null;
  const isCurrent = (requestRevision, result = null) => requestRevision === revision && !result?.stale;

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
    if (!isCurrent(requestRevision)) return;
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
    // 音声の準備待ちで入力が開いた場合も、未表示の通知を再び待機列へ戻す。
    await say(queue.shift());
  }

  function scheduleChatter() {
    if (chatterTimer) { clearTimeout(chatterTimer); chatterTimer = null; }
    if (!chatter.enabled) return;
    chatterTimer = setTimeout(fireChatter, (chatter.minSecs + Math.random() * (chatter.maxSecs - chatter.minSecs)) * 1000);
  }

  async function fireChatter() {
    chatterTimer = null;
    const requestRevision = revision;
    // 入力中・応答中・読み上げ中・全画面表示中・居眠り中は割り込まず、次の機会に回す。
    if (!canShow() || host.sleeping) { scheduleChatter(); return; }
    try {
      const result = await bridge.idleChatter();
      if (!isCurrent(requestRevision, result)) return;
      if (result?.busy) return;
      if (result?.enabled === false) { chatter.enabled = false; return; }
      await say(result?.reply || "");
    } catch (error) {
      if (isCurrent(requestRevision)) logApp("warn", "自発雑談の取得に失敗しました", errorDetail(error));
    } finally {
      if (isCurrent(requestRevision)) scheduleChatter();
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

  function cancelAlarmJob() {
    if (alarmJob) {
      alarmJob.cancelled = true;
      alarmJob.releaseRetry?.();
    }
  }

  function clearSnooze(stop) {
    cancelAlarmJob();
    snooze = null;
    chat.clearAlarm();
    if (stop) voice.stop();
    status.clear("alarm");
  }

  function armSnooze(alarm, consumed) {
    const minutes = snoozeMinutes(alarm);
    snooze = { alarm: { ...alarm, snooze_minutes: minutes }, consumed, dueAt: Date.now() + minutes * 60 * 1000 };
    status.set(`「おきたよ！」を押すまで${minutes}分おきに起こします`, "alarm");
  }

  const isAlarmActive = job => alarmJob === job && !job.cancelled && (!job.snoozeState || snooze === job.snoozeState);

  async function alarmStillRegistered(job) {
    if (!isAlarmActive(job)) return false;
    const result = await bridge.alarms();
    if (!Array.isArray(result?.alarms)) throw new Error("目覚まし一覧の応答が不正です");
    const current = result.alarms.find(alarm => alarm.id === job.alarm.id);
    // 成功応答が消費を確定した一度きりだけ、自動無効化後も同じジョブを続ける。
    if (!isAlarmActive(job)) return false;
    if (!current || (current.enabled === false && (!job.consumed || current.repeat !== false))) {
      clearSnooze(false);
      return false;
    }
    return true;
  }

  function reportAlarmFailure(job, error) {
    if (!isAlarmActive(job)) return;
    if (job.snoozeState) clearSnooze(false);
    notifyBackgroundFailure("alarm-wake", `目覚ましの声かけに失敗しました: ${errorMessage(error)}。AI設定を確認し、目覚ましを再設定してください`, error);
  }

  async function currentAlarmLine(job) {
    let retryMs = 1000;
    while (isAlarmActive(job)) {
      const requestRevision = revision;
      try {
        const result = await bridge.alarmWake(job.alarm.id, job.consumed);
        if (!isAlarmActive(job)) return null;
        if (result?.cancelled) {
          clearSnooze(false);
          return null;
        }
        // 画面だけが古くなった成功も自動消費を失わない。backend staleは消費していない。
        if (!result?.stale && result?.consumed === true) job.consumed = true;
        if (!await alarmStillRegistered(job)) return null;
        if (isCurrent(requestRevision, result)) {
          const line = String(result?.reply ?? "").trim();
          if (!line) throw new Error("目覚ましの起こし文が空です");
          clearBackgroundFailure("alarm-wake");
          return { line, requestRevision };
        }
      } catch (error) {
        if (!isAlarmActive(job)) return null;
        if (isCurrent(requestRevision)) {
          try {
            if (!await alarmStillRegistered(job)) return null;
          } catch (readError) {
            reportAlarmFailure(job, readError);
            return null;
          }
          reportAlarmFailure(job, error);
          return null;
        }
      }
      // キャラ切替が続いても同じ発火を失わず、即時ループで推論や通信を増やさない。
      await new Promise(resolve => {
        const timer = setTimeout(resolve, retryMs);
        job.releaseRetry = () => { clearTimeout(timer); resolve(); };
      });
      job.releaseRetry?.();
      job.releaseRetry = null;
      retryMs = Math.min(POLL_MS, retryMs * 2);
      if (!isAlarmActive(job)) return null;
      try {
        if (!await alarmStillRegistered(job)) return null;
      } catch (error) {
        reportAlarmFailure(job, error);
        return null;
      }
    }
    return null;
  }

  async function fireAlarm(alarm, snoozeState = null, due = null) {
    const job = { alarm, snoozeState, consumed: snoozeState?.consumed === true, cancelled: false, releaseRetry: null };
    alarmJob = job;
    try {
      let current;
      do {
        current = await currentAlarmLine(job);
        if (!current || !isAlarmActive(job)) return;
        // 待機の継続が次のmicrotaskへ移る間の切替も、本文の公開前に照合する。
      } while (!isCurrent(current.requestRevision));
      const { requestRevision } = current;
      let { line } = current;
      // 遅れて鳴らす場合は予定時刻を添える（いまの時刻の目覚ましだと誤解させない）。
      if (due && due.lateMinutes >= 1) line = `（${due.scheduledTime}の目覚ましだよ）${line}`;
      markActivity(); // 居眠り中でも目覚ましでは起きる
      // 声より先に文字を出す。操作ボタンをページ送りで消さない。
      chat.showAlarm(line, () => clearSnooze(true));
      history.add("assistant", line);
      // 生成中に「おきたよ！」で解除された場合は、完了後にスヌーズを復活させない。
      const shouldArm = !snoozeState || snooze === snoozeState;
      if (shouldArm) armSnooze(alarm, job.consumed);
      const armed = shouldArm ? snooze : null;
      job.snoozeState = armed;
      const speechRequest = voice.prepare(line).catch(error => {
        logApp("warn", "目覚ましの音声合成に失敗しました", errorDetail(error));
        return null;
      });
      const speech = await waitWithLimit(speechRequest, ALARM_SPEECH_TIMEOUT_MS);
      // 数分後に突然鳴るほうが困るため、遅れて届いた音声は鳴らさない。
      if (speech.timedOut) logApp("warn", "目覚ましの音声が時間内に用意できませんでした", `${ALARM_SPEECH_TIMEOUT_MS}ms`);
      if (isCurrent(requestRevision) && isAlarmActive(job) && armed && !speech.timedOut && speech.value && snooze === armed) {
        try {
          if (await alarmStillRegistered(job) && isCurrent(requestRevision)) voice.playPrepared(speech.value);
        } catch (error) { reportAlarmFailure(job, error); }
      }
    } finally {
      job.releaseRetry?.();
      if (alarmJob === job) alarmJob = null;
    }
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
    cancelAlarmJob();
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
      if (!config?.numberSettings) return;
      if (Object.hasOwn(config, "chatterMinSecs") || Object.hasOwn(config, "chatterMaxSecs")) {
        const minimum = readIntegerSetting(config, "PICOAGENT_CHATTER_MIN_SECS", "chatterMinSecs");
        const maximum = readIntegerSetting(config, "PICOAGENT_CHATTER_MAX_SECS", "chatterMaxSecs");
        const error = minimum.error || maximum.error || (minimum.value > maximum.value ? "雑談の最短間隔は最長間隔以下にしてください" : null);
        chatter.enabled = Boolean(config.chatterEnabled) && !error;
        if (error) notifyBackgroundFailure("integer-chatter", `自発会話を停止しました: ${error}`, error);
        else { chatter.minSecs = minimum.value; chatter.maxSecs = maximum.value; clearBackgroundFailure("integer-chatter"); }
      }
      if (Object.hasOwn(config, "napMinutes")) {
        const nap = readIntegerSetting(config, "PICOAGENT_NAP_MINUTES", "napMinutes");
        napMinutes = nap.value;
        if (nap.error) notifyBackgroundFailure("integer-nap", `居眠りを停止しました: ${nap.error}`, nap.error);
        else clearBackgroundFailure("integer-nap");
      }
      scheduleChatter();
    },
    start(config) {
      this.applyChatter(config);
      markActivity();
      dailyTimer = setInterval(checkDaily, POLL_MS);
      checkDaily();
      alarmTimer = setInterval(checkAlarms, POLL_MS);
      checkAlarms();
      napTimer = setInterval(() => {
        if (napMinutes > 0 && !host.sleeping && BigInt(Date.now() - lastActivityAt) > BigInt(napMinutes) * 60000n) fallAsleep();
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
