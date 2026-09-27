import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const voice = await readFile(new URL("../../ui/app/voice.js", import.meta.url), "utf8");
const lipsync = await readFile(new URL("../../ui/app/lipsync.js", import.meta.url), "utf8");
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");

const functionSource = voice.match(/function shouldLipSync\(audio\) \{[\s\S]*?\n {2}\}/)?.[0];
assert.ok(functionSource, "shouldLipSync の正本が見つかりません");
const mouthFormSource = lipsync.match(/function mouthFormAt\(currentTime, timeline\) \{[\s\S]*?\n\}/)?.[0];
assert.ok(mouthFormSource, "母音口形の正本が見つかりません");

function makePredicate({ currentVoice, ttsEnabledCache, voiceVolume }) {
  return Function(
    "current",
    "enabled",
    "volume",
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
  // 再生開始・音量変更のどちらでも、同じ判定を満たすときだけ口を動かす。
  assert.match(voice, /if \(!shouldLipSync\(audio\)\) \{ motion\.setSpeaking\(false\); return; \}\s*\n\s*motion\.setSpeaking\(true\);\s*\n\s*lipSync\.attach\(audio, \(\) => shouldLipSync\(audio\)\);/);
  assert.match(voice, /else if \(shouldLipSync\(current\)\) \{\s*\n\s*motion\.setSpeaking\(true\);\s*\n\s*lipSync\.attach\(current, \(\) => shouldLipSync\(current\)\);/);
  assert.doesNotMatch(voice + lipsync, /startSilentLipSync/);
  // ミュートにしたら音声と会話モーションを終える。
  assert.match(voice, /setEnabled\(value\) \{\s*\n\s*enabled = value !== false;\s*\n\s*if \(!enabled\) stop\(\);/);
});

test("読み上げ音量の未保存時初期値は35", () => {
  // 初回はAivisを自動導入して読み上げを有効にするため、既定値がそのまま最初の音量になる。
  // 100%相当は初対面の音量として大きすぎるので、画面側の初期値も含めて35へ揃える。
  assert.equal((commands.match(/PICOAGENT_TTS_VOLUME", "35"/g) ?? []).length, 2);
  assert.match(settings, /PICOAGENT_TTS_VOLUME \?\? "35"/);
  assert.match(voice, /let volume = 0\.35;/);
  assert.match(voice, /const gain = \(\) => volume \* volume;/);
  assert.match(settings, /音声・ゲーム効果音の音量（0〜100・初期値35）/);
});

test("Aivisの時間軸から母音口形を選ぶ", () => {
  const mouthFormAt = Function(`"use strict"; ${mouthFormSource}; return mouthFormAt;`)();
  const timeline = [
    { start: 0.10, end: 0.30, form: 0 },
    { start: 0.30, end: 0.50, form: 0.95 },
    { start: 0.50, end: 0.70, form: -0.95 },
  ];
  assert.equal(mouthFormAt(0.20, timeline), 0);
  assert.equal(mouthFormAt(0.35, timeline), 0.95);
  assert.equal(mouthFormAt(0.55, timeline), -0.95);
  assert.equal(mouthFormAt(0.80, timeline), 0, "母音外は中立へ戻す");
});

test("読み上げが無効なら合成経路へ入らず準備中表示も出さない", () => {
  // 音が出ない設定で「読み上げ準備中…」「読み上げを中止」が出ると、準備が続いて
  // いるように見える。判定は入口の speak 1か所に置き、呼び出し側で重ねない。
  const speak = voice.slice(voice.indexOf("  async function speak(text) {"));
  const guard = speak.indexOf("if (enabled === false || volume <= 0.001) {");
  const prepare = speak.indexOf('status.set("読み上げ準備中…", "voice")');
  assert.ok(guard >= 0 && guard < prepare, "無効時の分岐が準備中表示より前にない");
  assert.match(speak, /pager\.timed\(text\);\s*\n\s*return;/);
  // バックエンドが「無効」と答えた時点で覚える（最初の挨拶だけ準備中が出るのを防ぐ）。
  assert.match(voice, /if \(result\?\.enabled === false\) enabled = false;/);
});
