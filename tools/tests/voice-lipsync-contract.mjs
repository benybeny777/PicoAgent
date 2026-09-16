import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const commands = await readFile(new URL("../../src-tauri/src/commands.rs", import.meta.url), "utf8");
const settings = await readFile(new URL("../../ui/settings.js", import.meta.url), "utf8");

const functionSource = renderer.match(/function shouldLipSync\(audio\) \{[\s\S]*?\n\}/)?.[0];
assert.ok(functionSource, "shouldLipSync の正本が見つかりません");
const mouthFormSource = renderer.match(/function mouthFormAt\(currentTime, timeline\) \{[\s\S]*?\n\}/)?.[0];
assert.ok(mouthFormSource, "母音口形の正本が見つかりません");

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

test("読み上げ音量の未保存時初期値は35", () => {
  // 初回はAivisを自動導入して読み上げを有効にするため、既定値がそのまま最初の音量になる。
  // 100%相当は初対面の音量として大きすぎるので、画面側の初期値も含めて35へ揃える。
  assert.equal((commands.match(/PICOAGENT_TTS_VOLUME", "35"/g) ?? []).length, 2);
  assert.match(settings, /PICOAGENT_TTS_VOLUME \?\? "35"/);
  assert.match(renderer, /^let voiceVolume = 0\.35;/m);
  assert.match(renderer, /PicoVoice = \{ speak: speakReply, stop: stopVoice, volume: \(\) => voiceVolume \}/);
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
  // いるように見える。判定は入口の speakReply 1か所に置き、呼び出し側で重ねない。
  const speakReply = renderer.match(/async function speakReply\(text\) \{[\s\S]*?\n\}/)?.[0];
  assert.ok(speakReply, "speakReply の正本が見つかりません");
  const guard = speakReply.indexOf('ttsEnabledCache === false || voiceVolume <= 0.001');
  const prepare = speakReply.indexOf('status.textContent = "読み上げ準備中…"');
  assert.ok(guard >= 0 && guard < prepare, "無効時の分岐が準備中表示より前にない");
  assert.match(speakReply, /startTimedSubtitlePages\(text\);\s*\n\s*return;/);
  // 送信経路は同じ判定を重ねない（片方だけ直す取りこぼしを防ぐ）。
  assert.doesNotMatch(renderer, /cancelStreamSpeech\(\);\s*\n\s*if \(ttsEnabledCache === false/);
  // バックエンドが「無効」と答えた時点で覚える（最初の挨拶だけ準備中が出るのを防ぐ）。
  const prepareModel = renderer.match(/async function prepareSelectedVoiceModel\(\) \{[\s\S]*?\n\}/)?.[0];
  assert.ok(prepareModel, "prepareSelectedVoiceModel の正本が見つかりません");
  assert.match(prepareModel, /if \(!statusInfo\?\.enabled\) \{[\s\S]*?ttsEnabledCache = false;[\s\S]*?return null;/);
});
