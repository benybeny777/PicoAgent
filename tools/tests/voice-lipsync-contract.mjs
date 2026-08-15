import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");

const functionSource = renderer.match(/function shouldLipSync\(audio\) \{[\s\S]*?\n\}/)?.[0];
assert.ok(functionSource, "shouldLipSync の正本が見つかりません");

function makePredicate({ currentVoice, ttsEnabledCache, voiceVolume }) {
  return Function(
    "currentVoice",
    "ttsEnabledCache",
    "voiceVolume",
    `"use strict"; ${functionSource}; return shouldLipSync;`,
  )(currentVoice, ttsEnabledCache, voiceVolume);
}

test("口パクは有効な音声が実際に再生中のときだけ許可する", () => {
  const audio = { paused: false, ended: false };
  assert.equal(makePredicate({ currentVoice: audio, ttsEnabledCache: true, voiceVolume: 0.5 })(audio), true);
  assert.equal(makePredicate({ currentVoice: audio, ttsEnabledCache: false, voiceVolume: 0.5 })(audio), false, "ミュート中");
  assert.equal(makePredicate({ currentVoice: audio, ttsEnabledCache: true, voiceVolume: 0 })(audio), false, "音量0");
  assert.equal(makePredicate({ currentVoice: audio, ttsEnabledCache: true, voiceVolume: 0.5 })({ paused: false, ended: false }), false, "別の音声");
  // 再生前／停止中は「別インスタンスだから false」では検証にならないので、
  // currentVoice と同一の要素を paused にして判定させる。
  const paused = { paused: true, ended: false };
  assert.equal(makePredicate({ currentVoice: paused, ttsEnabledCache: true, voiceVolume: 0.5 })(paused), false, "再生前または停止中");
  audio.ended = true;
  assert.equal(makePredicate({ currentVoice: audio, ttsEnabledCache: true, voiceVolume: 0.5 })(audio), false, "再生終了後");
});

test("すべてのリップシンク入口が共通判定を使う", () => {
  const uses = renderer.match(/shouldLipSync\(/g)?.length ?? 0;
  assert.ok(uses >= 6, `共通判定の利用箇所が不足しています: ${uses}`);
  assert.doesNotMatch(renderer, /startSilentLipSync/);
  assert.match(renderer, /if \(!ttsEnabledCache\) stopVoice\(true\)/, "ミュート時に音声と会話モーションを終了する");
});

test("読み上げ音量の未保存時初期値は50", () => {
  assert.equal((commands.match(/PICOAGENT_TTS_VOLUME", "50"/g) ?? []).length, 2);
  assert.match(settings, /PICOAGENT_TTS_VOLUME \?\? "50"/);
});
