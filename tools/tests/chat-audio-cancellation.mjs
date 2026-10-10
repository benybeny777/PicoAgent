import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { uiFile } from "./lib/ui-source.mjs";

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

function target() {
  const handlers = new Map();
  return { handlers, addEventListener(name, callback) { handlers.set(name, callback); },
    setAttribute() {}, classList: { toggle() {} } };
}

function environment(file, factory, extra) {
  const timers = new Map();
  let next = 0;
  const schedule = (callback, delay) => { timers.set(++next, { callback, delay }); return next; };
  const context = vm.createContext({
    document: { ...target(), hidden: false }, window: target(), Blob,
    setTimeout: schedule, setInterval: schedule,
    clearTimeout: id => timers.delete(id), clearInterval: id => timers.delete(id),
    performance: { now: () => 1000 }, toast() {}, logApp() {}, errorDetail: String,
    errorMessage: String, preview: String, ...extra,
  });
  vm.runInContext(uiFile(file).replace(/^import .*;\r?$/gm, "").replace(/export function/g, "function")
    + `\nthis.factory = ${factory};`, context);
  async function tick(delay) {
    const entry = [...timers].find(([, timer]) => timer.delay === delay);
    assert.ok(entry, `${delay}ms の待機が存在する`);
    timers.delete(entry[0]);
    await entry[1].callback();
  }
  return { context, timers, factory: context.factory, tick };
}

function micHarness(media = async () => ({ getTracks: () => [{ stop() {} }] })) {
  const transcript = deferred();
  let acquired = 0;
  let submitted = 0;
  let processor;
  const busy = [];
  const composer = { value: "", focus() {}, open() {}, resetNavigation() {},
    setValue(text) { this.value = text; }, form: { requestSubmit() { submitted += 1; } } };
  const env = environment("app/mic.js", "createMic", {
    navigator: { mediaDevices: { getUserMedia: () => { acquired += 1; return media(); } } },
    bridge: { transcribeAudio: () => transcript.promise }, blobToDataUrl: async () => "録音",
  });
  env.context.window.AudioContext = class {
    sampleRate = 16000;
    createMediaStreamSource() { return { connect() {} }; }
    createScriptProcessor() { processor = { connect() {}, disconnect() {} }; return processor; }
    close() { return Promise.resolve(); }
  };
  const micButton = target(), loopButton = target();
  const mic = env.factory({
    chat: { composer, buttons: { micButton, voiceLoopButton: loopButton },
      status: { clear() {}, set() {}, setBusy(value) { busy.push(value); } } },
    voice: { stop() {}, speaking: false }, motion: { rest() {}, react() {} },
    isBusy: () => false, canListen: () => true,
  });
  const clickLoop = () => loopButton.handlers.get("click")({ stopPropagation() {} });
  return { ...env, mic, composer, transcript, busy, clickLoop,
    get acquired() { return acquired; },
    get submitted() { return submitted; },
    record() { processor.onaudioprocess({ inputBuffer: { getChannelData: () => new Float32Array(16000) } }); },
    clickMic() { return micButton.handlers.get("click")({ stopPropagation() {} }); } };
}

test("連続会話は確定文字起こしと自動送信の待機中に次の録音を開始しない", async () => {
  const env = micHarness();
  env.clickLoop();
  await env.tick(450);
  env.record();
  const finish = env.clickMic();
  await Promise.resolve();
  env.mic.scheduleContinuous();
  await env.tick(450);
  assert.equal(env.acquired, 1);
  env.transcript.resolve({ text: "認識した本文" });
  await finish;
  assert.equal(env.composer.value, "認識した本文");
  env.mic.scheduleContinuous();
  await env.tick(450);
  assert.equal(env.acquired, 1);
  env.clickLoop();
  assert.equal(env.timers.size, 0);
});

test("連続会話の停止は許可待ちのマイクを取り消し遅れて取得したトラックを回収する", async () => {
  const media = deferred();
  let stopped = 0;
  const env = micHarness(() => media.promise);
  env.clickLoop();
  const start = env.tick(450);
  env.clickLoop();
  media.resolve({ getTracks: () => [{ stop() { stopped += 1; } }] });
  await start;
  assert.equal(env.mic.active, false);
  assert.equal(stopped, 1);
});

test("確定文字起こし中の連続会話停止は待機表示を解除し遅延結果で入力を変更しない", async () => {
  const env = micHarness();
  env.clickLoop();
  await env.tick(450);
  env.record();
  const finish = env.clickMic();
  await Promise.resolve();
  assert.equal(env.busy.at(-1), true);
  env.clickLoop();
  assert.equal(env.busy.at(-1), false);
  env.transcript.resolve({ text: "取り消した本文" });
  await finish;
  assert.equal(env.composer.value, "");
});

function voiceHarness() {
  const synth = deferred();
  const revoked = [];
  const audio = [];
  const messages = [];
  const speaking = [];
  const env = environment("app/voice.js", "createVoice", {
    bridge: { speak: () => synth.promise },
    URL: { createObjectURL: () => "音声URL", revokeObjectURL: url => revoked.push(url) },
    Audio: class {
      constructor() { this.paused = true; audio.push(this); }
      handlers = new Map();
      addEventListener(name, callback) { this.handlers.set(name, callback); }
      play() { return new Promise((_, reject) => { this.rejectPlay = reject; }); }
      pause() { this.paused = true; }
      removeAttribute() {}
      load() {}
    },
  });
  const voice = env.factory({
    motion: { setSpeaking: value => speaking.push(value) }, lipSync: { attach() {}, detach() {} },
    pager: { cancel() {}, timed() {} }, status: { set: text => messages.push(text), flash: text => messages.push(text), clear() {} },
    onCancelVisible() {},
  });
  voice.setVolumeError(null);
  voice.setVolume(35);
  return { ...env, voice, synth, revoked, audio, messages, speaking };
}

test("音声再生の開始待ちで停止しても待機PromiseとObject URLを直ちに解放する", async () => {
  const env = voiceHarness();
  let completed = false;
  const playback = env.voice.playPrepared({ audio: [1] }).then(() => { completed = true; });
  env.voice.stop();
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(completed, true);
  assert.deepEqual(env.revoked, ["音声URL"]);
  await playback;
});

test("音声設定を無効へ変更した後の古い合成結果は有効状態と診断を戻さない", async () => {
  const env = voiceHarness();
  const preparation = env.voice.prepare("合成中の本文");
  env.voice.setEnabled(false);
  env.synth.resolve({ enabled: true, audio: [1] });
  assert.equal(await preparation, null);
  assert.equal(env.voice.enabled, false);
  assert.deepEqual(env.messages, []);
});

test("文字起こしの確定待ちで編集した下書きは遅延結果で上書きも自動送信もしない", async () => {
  const env = micHarness();
  env.clickLoop();
  await env.tick(450);
  env.record();
  const finish = env.clickMic();
  await Promise.resolve();
  env.composer.setValue("手で編集した大事な下書き");
  env.transcript.resolve({ text: "音声認識の結果" });
  await finish;
  assert.equal(env.composer.value, "手で編集した大事な下書き");
  assert.equal([...env.timers.values()].some(timer => timer.delay === 1500), false);
  env.clickLoop();
});

test("音声の自動送信待ちで下書きを変更したら編集内容を勝手に送らない", async () => {
  const env = micHarness();
  env.clickLoop();
  await env.tick(450);
  env.record();
  const finish = env.clickMic();
  env.transcript.resolve({ text: "認識した本文" });
  await finish;
  env.composer.setValue("まだ送らない編集途中の本文");
  await env.tick(1500);
  assert.equal(env.submitted, 0);
  assert.equal(env.composer.value, "まだ送らない編集途中の本文");
  env.clickLoop();
});

test("音声の再生拒否は画面へ理由を出し停止後の古い拒否は表示しない", async () => {
  for (const cancelled of [false, true]) {
    const env = voiceHarness();
    const playing = env.voice.playPrepared({ audio: [1] });
    if (cancelled) env.voice.stop();
    env.audio[0].rejectPlay(new Error("音声デバイスを開けません"));
    await playing;
    assert.equal(env.messages.some(text => /音声を再生できません.*音声デバイス/.test(text)), !cancelled);
    assert.equal(env.revoked.length, 1);
  }
});

test("音声が終わった瞬間に口パクを止め次の合成待ちへ持ち越さない", async () => {
  const env = voiceHarness();
  const playing = env.voice.playPrepared({ audio: [1] });
  env.audio[0].paused = false;
  env.audio[0].handlers.get("playing")();
  assert.equal(env.speaking.at(-1), true);
  env.audio[0].handlers.get("ended")();
  assert.equal(env.speaking.at(-1), false);
  await playing;
});

test("消音で次の字幕を出すときは以前の再生待ちと音声URLを解放する", async () => {
  const env = voiceHarness();
  const playing = env.voice.playPrepared({ audio: [1] });
  env.voice.setVolume(0);
  await env.voice.speak("次の本文");
  assert.equal(env.revoked.length, 1);
  await playing;
});
