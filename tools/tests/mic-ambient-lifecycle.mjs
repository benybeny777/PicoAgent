import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

const integerMetadata = {
  PICOAGENT_CHATTER_MIN_SECS: { range: ["30", "3600"] },
  PICOAGENT_CHATTER_MAX_SECS: { range: ["30", "3600"] },
  PICOAGENT_NAP_MINUTES: { range: ["0", "9223372036854775807"] },
};
function quantities(values) { return { numberSettings: integerMetadata, chatterMinSecs: 90, chatterMaxSecs: 240, napMinutes: 0, ...values }; }

function target() {
  const handlers = new Map();
  return {
    handlers, disabled: false,
    addEventListener(name, callback) { handlers.set(name, callback); },
    setAttribute() {}, classList: { toggle() {} },
  };
}

function environment(file, factory, extra = {}) {
  const timers = new Map();
  const deadlines = new Map();
  let now = 0;
  const epoch = Date.parse("2026-10-11T07:00:00Z");
  class ClockDate extends Date {
    constructor(...args) { super(...(args.length ? args : [epoch + now])); }
    static now() { return epoch + now; }
  }
  let id = 0;
  const schedule = (callback, delay, interval = false) => {
    timers.set(++id, callback);
    deadlines.set(id, { due: now + delay, delay, interval });
    return id;
  };
  const clear = value => { timers.delete(value); deadlines.delete(value); };
  const context = vm.createContext({
    document: { ...target(), hidden: false }, window: target(), Date: ClockDate,
    setTimeout: schedule, setInterval: (callback, delay) => schedule(callback, delay, true),
    clearTimeout: clear, clearInterval: clear,
    performance: { now: () => 0 }, console,
    toast() {}, logApp() {}, clearBackgroundFailure() {}, notifyBackgroundFailure() {},
    errorDetail: String, errorMessage: String, preview: String, ...extra,
  });
  const util = uiFile("app/util.js");
  const wait = util.slice(util.indexOf("export async function waitWithLimit"), util.indexOf("/** 指定時間内に応答しない fetch"));
  vm.runInContext(util.slice(util.indexOf("export function readIntegerSetting")).replace("export function", "function"), context);
  vm.runInContext(wait.replace("export async function", "async function"), context);
  vm.runInContext(uiFile(file).replace(/^import .*;\r?$/gm, "").replace("export function", "function") + `\nthis.factory = ${factory};`, context);
  async function settle() { for (let step = 0; step < 20; step += 1) await Promise.resolve(); }
  async function advance(ms) {
    now += ms;
    for (const [key, timer] of [...deadlines]) {
      if (timer.due > now || !timers.has(key)) continue;
      const callback = timers.get(key);
      if (timer.interval) timer.due = now + timer.delay;
      else clear(key);
      callback();
    }
    await settle();
  }
  return { context, timers, factory: context.factory, advance, settle };
}

for (const event of ["pointerup", "pointercancel"]) {
  test(`マイク許可待ちの長押しは${event}で取り消して遅延取得のトラックを停止する`, async () => {
    let resolveMedia;
    let stopped = 0;
    const env = environment("app/mic.js", "createMic", {
      navigator: { mediaDevices: { getUserMedia: () => new Promise(resolve => { resolveMedia = resolve; }) } },
    });
    const micButton = target();
    const mic = env.factory({
      chat: { buttons: { micButton, voiceLoopButton: target() },
        composer: { value: "", focus() {} }, status: { clear() {} } },
      voice: { stop() {} }, motion: { rest() {} }, isBusy: () => false, canListen: () => true,
    });
    micButton.handlers.get("pointerdown")({ button: 0 });
    const begin = [...env.timers.values()][0]();
    await micButton.handlers.get(event)();
    resolveMedia({ getTracks: () => [{ stop() { stopped += 1; } }] });
    await begin;
    assert.equal(stopped, 1);
    assert.equal(mic.active, false);
    assert.equal(micButton.disabled, false);
  });
}

test("録音開始後の長押し取消は取得済みのトラックと音声処理を解放する", async () => {
  let stopped = 0;
  let closed = 0;
  const env = environment("app/mic.js", "createMic", {
    navigator: { mediaDevices: { getUserMedia: async () => ({ getTracks: () => [{ stop() { stopped += 1; } }] }) } },
  });
  env.context.window.AudioContext = class {
    sampleRate = 16000;
    createMediaStreamSource() { return { connect() {} }; }
    createScriptProcessor() { return { connect() {}, disconnect() {} }; }
    close() { closed += 1; return Promise.resolve(); }
  };
  const micButton = target();
  const mic = env.factory({
    chat: { buttons: { micButton, voiceLoopButton: target() },
      composer: { value: "", focus() {} }, status: { clear() {}, set() {} } },
    voice: { stop() {}, speaking: false }, motion: { rest() {}, react() {} },
    isBusy: () => false, canListen: () => true,
  });
  micButton.handlers.get("pointerdown")({ button: 0 });
  await [...env.timers.values()][0]();
  assert.equal(mic.active, true);
  micButton.handlers.get("pointercancel")();
  assert.equal(mic.active, false);
  assert.equal(stopped, 1);
  assert.equal(closed, 1);
});

test("同時に期限が来た全リマインダーを一つの通知として表示と音声と履歴へ渡す", async () => {
  const displayed = [], prepared = [], history = [];
  const env = environment("app/ambient.js", "createAmbient", {
    bridge: { dailyEvents: async () => ({ events: [{ text: "会議" }, { text: "薬" }] }), alarms: async () => ({ alarms: [] }) },
  });
  const ambient = env.factory({
    chat: { status: {}, composer: { isOpen: false }, history: { add: (_role, text) => history.push(text) },
      showAnswer: text => displayed.push(text) },
    voice: { speaking: false, prepare: async text => { prepared.push(text); return null; } },
    host: { sleeping: false }, motion: {}, isBusy: () => false, isExpanded: () => false,
  });
  ambient.start(quantities({ napMinutes: 0 }));
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(displayed, ["会議\n薬"]);
  assert.deepEqual(prepared, displayed);
  assert.deepEqual(history, displayed);
});

test("通知の音声準備中に入力を開いても待機通知を失わず次の機会に表示する", async () => {
  let resolveSpeech;
  let first = true;
  const displayed = [];
  const composer = { isOpen: true };
  const env = environment("app/ambient.js", "createAmbient");
  const ambient = env.factory({
    chat: { composer, status: { clear() {} }, history: { add() {} }, showAnswer: text => displayed.push(text) },
    voice: { speaking: false, prepare: () => first ? new Promise(resolve => { resolveSpeech = resolve; first = false; }) : Promise.resolve(null) },
    host: { sleeping: false }, motion: {}, isBusy: () => false, isExpanded: () => false,
  });
  await ambient.say("必ず知らせるリマインダー");
  composer.isOpen = false;
  const flushing = ambient.flushNotice();
  composer.isOpen = true;
  resolveSpeech(null);
  await flushing;
  assert.deepEqual(displayed, []);
  composer.isOpen = false;
  await ambient.flushNotice();
  assert.deepEqual(displayed, ["必ず知らせるリマインダー"]);
});

test("キャラ切替前に生成を始めた雑談は遅れて届いても新キャラの画面へ出さない", async () => {
  let resolveChatter;
  const displayed = [];
  const env = environment("app/ambient.js", "createAmbient", {
    bridge: { idleChatter: () => new Promise(resolve => { resolveChatter = resolve; }) },
  });
  const ambient = env.factory({
    chat: { composer: { isOpen: false }, status: { clear() {} }, history: { add() {} }, showAnswer: text => displayed.push(text) },
    voice: { speaking: false, prepare: async () => null }, host: { sleeping: false }, motion: {},
    isBusy: () => false, isExpanded: () => false,
  });
  ambient.applyChatter(quantities({ chatterEnabled: true, chatterMinSecs: 30, chatterMaxSecs: 30 }));
  const old = [...env.timers.values()][0]();
  ambient.reset();
  resolveChatter({ enabled: true, reply: "旧キャラの雑談" });
  await old;
  assert.deepEqual(displayed, []);
});

function alarmEnvironment(prepare = async () => null) {
  let resolveWake;
  let rejectWake;
  let wakeCalls = 0;
  const wakeArgs = [];
  let acknowledge;
  const shown = [];
  const history = [];
  const speech = [];
  const played = [];
  const failures = [];
  const recoveries = [];
  const alarms = [{ id: "alarm", repeat: false, enabled: true }];
  let alarmReadError = null;
  let beforeWakePublish = null;
  const env = environment("app/ambient.js", "createAmbient", {
    notifyBackgroundFailure: (...args) => failures.push(args), clearBackgroundFailure: key => {
      recoveries.push(key);
      if (key === "alarm-wake" && beforeWakePublish) {
        const callback = beforeWakePublish;
        beforeWakePublish = null;
        callback();
      }
    },
    bridge: { dailyEvents: async () => ({ events: [] }), alarms: async () => { if (alarmReadError) throw alarmReadError; return { alarms }; },
      alarmWake: (_id, consumed) => { wakeCalls += 1; wakeArgs.push(consumed); return new Promise((resolve, reject) => { resolveWake = resolve; rejectWake = reject; }); } },
    window: { ...target(), PicoAlarmRuntime: {
      dueAlarms: alarms => alarms.length ? [{ alarm: alarms[0], scheduledKey: "朝", scheduledTime: "07:00", lateMinutes: 0 }] : [],
      markFired: (fired, key) => { if (fired.has(key)) return false; fired.add(key); return true; },
    } },
  });
  const ambient = env.factory({
    chat: { composer: { isOpen: false }, status: { set() {}, clear() {} },
      history: { add: (_role, text) => history.push(text) }, clearAlarm() {},
      showAlarm: (text, callback) => { shown.push(text); acknowledge = callback; } },
    voice: { prepare: text => { speech.push(text); return prepare(text); }, playPrepared: value => played.push(value), stop() {} },
    host: { sleeping: false }, motion: {}, isBusy: () => true, isExpanded: () => false,
  });
  ambient.start(quantities({ napMinutes: 0 }));
  return { ...env, ambient, shown, history, speech, played, failures, recoveries, alarms, wakeArgs, beforeWakePublish: callback => { beforeWakePublish = callback; }, failAlarmRead: error => { alarmReadError = error; }, get wakeCalls() { return wakeCalls; }, resolveWake: value => resolveWake(value), rejectWake: value => rejectWake(value), acknowledge: () => acknowledge() };
}

test("目覚ましは会話待機が20秒を超えても表示も合成もせず返答後に文字を出す", async () => {
  const env = alarmEnvironment();
  await env.settle();
  await env.advance(21000);
  assert.deepEqual(env.shown, []);
  assert.deepEqual(env.history, []);
  assert.deepEqual(env.speech, []);
  env.resolveWake({ reply: "会話が終わったので起こすよ" });
  await env.settle();
  assert.deepEqual(env.shown, ["会話が終わったので起こすよ"]);
  assert.deepEqual(env.history, env.shown);
  assert.deepEqual(env.speech, env.shown);
  assert.ok(env.recoveries.includes("alarm-wake"));
  assert.equal(env.ambient.alarmActive, true);
});

for (const reason of ["起こし文の生成に失敗", "起こし文の生成が20秒を超過", "空の起こし文"]) {
  test(`目覚ましの${reason}は理由通知だけで固定文の表示と合成へ進まない`, async () => {
    const env = alarmEnvironment();
    await env.settle();
    if (reason === "空の起こし文") env.resolveWake({ reply: "  " });
    else env.rejectWake(new Error(reason));
    await env.settle();
    assert.deepEqual(env.shown, []);
    assert.deepEqual(env.history, []);
    assert.deepEqual(env.speech, []);
    assert.equal(env.ambient.alarmActive, false);
    assert.equal(env.failures.length, 1);
    assert.equal(env.recoveries.includes("alarm-wake"), false);
    assert.equal(env.failures[0][0], "alarm-wake");
    assert.match(env.failures[0][1], reason === "空の起こし文" ? /空/ : new RegExp(reason));
    assert.match(String(env.failures[0][2]), reason === "空の起こし文" ? /空/ : new RegExp(reason));
  });
}

test("スヌーズ生成の失敗は再通知を解除し自動で繰り返さない", async () => {
  const env = alarmEnvironment();
  await env.settle();
  env.resolveWake({ reply: "初回の起こし文" });
  await env.settle();
  await env.advance(300001);
  assert.equal(env.wakeCalls, 2);
  env.rejectWake(new Error("再通知生成エラー"));
  await env.settle();
  assert.equal(env.ambient.alarmActive, false);
  assert.equal(env.failures.length, 1);
  assert.deepEqual(env.shown, ["初回の起こし文"]);
  await env.advance(300001);
  assert.equal(env.wakeCalls, 2);
});

for (const action of ["合成上限", "解除"]) {
  test(`目覚ましは文字を先に出し${action}後に届いた音声を再生しない`, async () => {
    let resolveSpeech;
    const env = alarmEnvironment(() => new Promise(resolve => { resolveSpeech = resolve; }));
    await env.settle();
    env.resolveWake({ reply: "起きる時間だよ" });
    await env.settle();
    assert.deepEqual(env.shown, ["起きる時間だよ"]);
    assert.equal(env.ambient.alarmActive, true);
    assert.deepEqual(env.played, []);
    if (action === "合成上限") await env.advance(21000);
    else env.acknowledge();
    resolveSpeech({ audio: "遅い音声" });
    await env.settle();
    assert.deepEqual(env.played, []);
  });
}

for (const stale of ["画面リセット", "バックエンド世代違い"]) {
  test(`目覚まし待機中の${stale}は旧返答を捨て同じジョブを現在キャラで完了する`, async () => {
    const env = alarmEnvironment();
    await env.settle();
    if (stale === "画面リセット") env.ambient.reset();
    env.resolveWake(stale === "画面リセット" ? { reply: "古いキャラの起こし文", consumed: true } : { stale: true, reply: "" });
    await env.settle();
    assert.deepEqual(env.shown, []);
    assert.deepEqual(env.speech, []);
    assert.equal(env.ambient.alarmActive, false);
    assert.equal(env.wakeCalls, 1, "即時再取得を繰り返さない");
    await env.advance(1000);
    assert.equal(env.wakeCalls, 2);
    assert.deepEqual(env.wakeArgs, [false, stale === "画面リセット"], "成功した消費だけを古い画面からも引き継ぐ");
    env.alarms[0].enabled = false;
    env.resolveWake({ reply: "現在キャラの起こし文", consumed: true });
    await env.settle();
    assert.deepEqual(env.shown, ["現在キャラの起こし文"]);
    assert.deepEqual(env.speech, env.shown);
    assert.equal(env.ambient.alarmActive, true);
    await env.advance(21000);
    assert.equal(env.wakeCalls, 2);
    await env.advance(300001);
    assert.equal(env.wakeArgs.at(-1), true, "スヌーズも消費済みを引き継ぐ");
    env.resolveWake({ reply: "消費後のスヌーズ", consumed: true });
    await env.settle();
    assert.equal(env.shown.at(-1), "消費後のスヌーズ");
  });
}

for (const repeat of [false, true]) {
  test(`${repeat ? "繰り返し" : "一度きり"}の初回待機中に無効化した目覚ましは未消費の応答を表示しない`, async () => {
    const env = alarmEnvironment();
    await env.settle();
    env.alarms[0].repeat = repeat;
    env.alarms[0].enabled = false;
    env.resolveWake({ reply: "無効化済みの返答" });
    await env.settle();
    assert.deepEqual(env.wakeArgs, [false]);
    assert.deepEqual(env.shown, []);
    assert.deepEqual(env.speech, []);
    assert.deepEqual(env.failures, []);
    assert.equal(env.ambient.alarmActive, false);
  });
}

test("バックエンド取消応答は消費フラグも本文も使わず無通知で解除する", async () => {
  const env = alarmEnvironment();
  await env.settle();
  env.resolveWake({ cancelled: true, reply: "取消後に表示しない", consumed: true });
  await env.settle();
  assert.deepEqual(env.shown, []);
  assert.deepEqual(env.history, []);
  assert.deepEqual(env.speech, []);
  assert.deepEqual(env.failures, []);
  assert.equal(env.ambient.alarmActive, false);
  await env.advance(21000);
  assert.equal(env.wakeCalls, 1);
});

test("遅い生成エラーは取消と登録状態を照合して有効な目覚ましの理由だけ通知する", async () => {
  const cases = [
    { name: "削除", mutate: env => { env.alarms.length = 0; }, reason: null },
    { name: "一度きりOFF", mutate: env => { env.alarms[0].enabled = false; }, reason: null },
    { name: "繰り返しOFF", mutate: env => { env.alarms[0].repeat = true; env.alarms[0].enabled = false; }, reason: null },
    { name: "登録有効", mutate() {}, reason: "遅い生成エラー" },
    { name: "一覧読込失敗", mutate: env => env.failAlarmRead(new Error("一覧を読み取れません")), reason: "一覧を読み取れません" },
  ];
  for (const entry of cases) {
    const env = alarmEnvironment();
    await env.settle();
    entry.mutate(env);
    env.rejectWake(new Error("遅い生成エラー"));
    await env.settle();
    assert.deepEqual(env.shown, [], entry.name);
    assert.deepEqual(env.history, [], entry.name);
    assert.deepEqual(env.speech, [], entry.name);
    assert.equal(env.failures.length, entry.reason ? 1 : 0, entry.name);
    if (entry.reason) assert.ok(env.failures[0][1].includes(entry.reason), entry.name);
  }
});

test("起こし文の返却と公開の間のmicrotask切替でも旧本文を出さず同じジョブで再取得する", async () => {
  const env = alarmEnvironment();
  await env.settle();
  env.beforeWakePublish(() => { Promise.resolve().then(() => env.ambient.reset()); });
  env.resolveWake({ reply: "公開直前の旧本文", consumed: true });
  await env.settle();
  assert.deepEqual(env.shown, []);
  assert.deepEqual(env.history, []);
  assert.deepEqual(env.speech, []);
  assert.deepEqual(env.wakeArgs, [false, true]);
  env.resolveWake({ reply: "再取得した新本文", consumed: true });
  await env.settle();
  assert.deepEqual(env.shown, ["再取得した新本文"]);
});

test("世代違いが続く目覚ましは再取得の待ちを増やして即時ループしない", async () => {
  const env = alarmEnvironment();
  await env.settle();
  env.resolveWake({ stale: true, reply: "" });
  await env.settle();
  await env.advance(1000);
  assert.equal(env.wakeCalls, 2);
  env.resolveWake({ stale: true, reply: "" });
  await env.settle();
  await env.advance(1000);
  assert.equal(env.wakeCalls, 2);
  await env.advance(1000);
  assert.equal(env.wakeCalls, 3);
  env.resolveWake({ reply: "新しい起こし文" });
  await env.settle();
  assert.deepEqual(env.shown, ["新しい起こし文"]);
});

for (const phase of ["生成中", "再取得待ち"]) {
  for (const action of ["利用者活動", "削除", "繰り返し無効化"]) {
    test(`目覚ましの${phase}に${action}があれば再表示せず待機を解放する`, async () => {
      const env = alarmEnvironment();
      await env.settle();
      if (phase === "再取得待ち") {
        env.resolveWake({ stale: true, reply: "" });
        await env.settle();
      }
      if (action === "利用者活動") env.ambient.onUserActivity();
      else if (action === "削除") env.alarms.length = 0;
      else { env.alarms[0].repeat = true; env.alarms[0].enabled = false; }
      if (phase === "生成中") env.resolveWake({ reply: "中止した目覚まし" });
      await env.advance(1000);
      assert.deepEqual(env.shown, []);
      assert.deepEqual(env.speech, []);
      assert.equal(env.ambient.alarmActive, false);
      assert.equal(env.wakeCalls, 1);
      // 完了したジョブに待機が残っていなければ、別の予定を処理できる。
      env.alarms.splice(0, env.alarms.length, { id: "next", repeat: false, enabled: true });
      await env.advance(21000);
      assert.equal(env.wakeCalls, 2);
      env.resolveWake({ reply: "次の予定" });
      await env.settle();
      assert.deepEqual(env.shown, ["次の予定"]);
    });
  }
}

test("おきたよで解除したスヌーズは再取得待ちのあとも復活しない", async () => {
  const env = alarmEnvironment();
  await env.settle();
  env.resolveWake({ reply: "初回" });
  await env.settle();
  await env.advance(300001);
  env.resolveWake({ stale: true, reply: "" });
  await env.settle();
  env.acknowledge();
  await env.advance(1000);
  assert.equal(env.wakeCalls, 2);
  assert.deepEqual(env.shown, ["初回"]);
  assert.equal(env.ambient.alarmActive, false);
});

for (const phase of ["生成中", "再取得待ち"]) {
  test(`画面終了は目覚ましの${phase}と全タイマーを回収して遅い返答を表示しない`, async () => {
    const env = alarmEnvironment();
    await env.settle();
    if (phase === "再取得待ち") {
      env.resolveWake({ stale: true, reply: "" });
      await env.settle();
    }
    env.context.window.handlers.get("beforeunload")();
    await env.settle();
    assert.equal(env.timers.size, 0);
    if (phase === "生成中") env.resolveWake({ reply: "終了後の返答" });
    await env.advance(21000);
    assert.equal(env.wakeCalls, 1);
    assert.deepEqual(env.shown, []);
    assert.deepEqual(env.speech, []);
  });
}

test("合成中のキャラ切替は旧音声を止めスヌーズの次回だけ新しい返答で起こす", async () => {
  let resolveSpeech;
  let first = true;
  const env = alarmEnvironment(() => {
    if (!first) return Promise.resolve(null);
    first = false;
    return new Promise(resolve => { resolveSpeech = resolve; });
  });
  await env.settle();
  env.resolveWake({ reply: "切替前の起こし文" });
  await env.settle();
  env.ambient.reset();
  resolveSpeech({ audio: "旧キャラ音声" });
  await env.settle();
  assert.deepEqual(env.played, []);
  assert.equal(env.ambient.alarmActive, true);
  await env.advance(300001);
  assert.equal(env.wakeCalls, 2);
  env.resolveWake({ reply: "新キャラの再通知" });
  await env.settle();
  assert.deepEqual(env.shown, ["切替前の起こし文", "新キャラの再通知"]);
  env.acknowledge();
  assert.equal(env.ambient.alarmActive, false);
  await env.advance(300001);
  assert.equal(env.wakeCalls, 2);
});

test("自発雑談は混雑応答を表示せず次の機会に再予約する", async () => {
  let calls = 0;
  const displayed = [];
  const speech = [];
  const env = environment("app/ambient.js", "createAmbient", {
    bridge: { idleChatter: async () => ++calls === 1 ? { busy: true, reply: "混雑中は公開しない" } : { enabled: true, reply: "次の雑談" } },
  });
  const ambient = env.factory({
    chat: { composer: { isOpen: false }, status: { clear() {} }, history: { add() {} }, showAnswer: text => displayed.push(text) },
    voice: { speaking: false, prepare: async text => { speech.push(text); return null; } },
    host: { sleeping: false }, motion: {}, isBusy: () => false, isExpanded: () => false,
  });
  ambient.applyChatter(quantities({ chatterEnabled: true, chatterMinSecs: 30, chatterMaxSecs: 30 }));
  await env.advance(30000);
  assert.equal(calls, 1);
  assert.deepEqual(displayed, []);
  assert.deepEqual(speech, []);
  assert.equal(env.timers.size, 1);
  await env.advance(30000);
  assert.equal(calls, 2);
  assert.deepEqual(displayed, ["次の雑談"]);
});

test("数量不正は自発会話と居眠りだけ理由付きで止め正常値で再開する", async () => {
  const env = alarmEnvironment();
  env.ambient.applyChatter(quantities({ chatterEnabled: true, chatterMinSecs: null, napMinutes: null, invalidNumbers: [{ key: "PICOAGENT_CHATTER_MIN_SECS", message: "間隔が不正です" }, { key: "PICOAGENT_NAP_MINUTES", message: "居眠りが不正です" }] }));
  assert.ok(env.failures.some(([key, message]) => key === "integer-chatter" && message.includes("間隔が不正")));
  assert.ok(env.failures.some(([key, message]) => key === "integer-nap" && message.includes("居眠りが不正")));
  assert.equal(env.timers.size, 3, "停止中は監視タイマーだけ残す");
  env.ambient.applyChatter(quantities({ chatterEnabled: true, napMinutes: "9223372036854775807" }));
  assert.equal(env.timers.size, 4, "正常値の更新で雑談を再予約する");
  assert.ok(env.recoveries.includes("integer-chatter"));
  assert.ok(env.recoveries.includes("integer-nap"));
  await env.advance(30000);
});
