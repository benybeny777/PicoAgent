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

test("目覚ましは生成と合成の遅さで表示を止めない", () => {
  const fire = renderer.slice(renderer.indexOf("async function fireAlarm"), renderer.indexOf("function startAlarmPolling"));
  // 文字が先。声はあとから。
  assert.ok(fire.indexOf("renderAlarmReply(line)") < fire.indexOf("prepareSpeech(line)"),
    "音声合成を待ってから表示しています");
  // 再通知の起点も合成待ちで遅らせない。
  assert.ok(fire.indexOf("armAlarmSnooze(a)") < fire.indexOf("prepareSpeech(line)"),
    "スヌーズの起点が音声合成待ちの後になっています");
  // 生成・合成のどちらにも上限がある（どちらも数十秒〜数分かかり得る）。
  assert.match(fire, /waitWithLimit\(wakeRequest, ALARM_WAKE_TIMEOUT_MS\)/);
  assert.match(fire, /waitWithLimit\(speechRequest, ALARM_SPEECH_TIMEOUT_MS\)/);
  // 遅れて届いた音声は鳴らさない。
  assert.match(fire, /if \(!speech\.timedOut && speech\.value\) playPreparedSpeech\(speech\.value\)/);
  assert.match(renderer, /const ALARM_WAKE_TIMEOUT_MS = 20000/);
  assert.match(renderer, /const ALARM_SPEECH_TIMEOUT_MS = 20000/);
});
