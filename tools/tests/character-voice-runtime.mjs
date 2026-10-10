// 同じ原因の声の失敗を重ねて通知せず、キャラ別の復旧だけを解除する。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";

const source = readFileSync(new URL("../../ui/app/voice.js", import.meta.url), "utf8")
  .replace(/^import .*;\r?$/gm, "").replace(/export function/g, "function");

function setup() {
  const notified = [], active = new Set(), statuses = [];
  let reply = () => Promise.reject(new Error("UUIDがありません"));
  const voice = runInNewContext(`${source}\ncreateVoice({ motion: { setSpeaking() {} }, lipSync: { detach() {} }, pager: { cancel() {} }, status, onCancelVisible() {} })`, {
    bridge: { speak: () => reply(), prepareVoiceModel: () => reply() },
    status: { set: text => statuses.push(text), clear() {} },
    logApp() {}, errorDetail: String, errorMessage: error => error.message || String(error),
    notifyBackgroundFailure(key, message) { if (!active.has(key)) notified.push({ key, message }); active.add(key); },
    clearBackgroundFailure(key) { active.delete(key); },
  });
  voice.setVolumeError(null);
  voice.setVolume(35);
  return { voice, notified, active, statuses, reply: fn => { reply = fn; } };
}

test("合成と起動準備の同じ失敗は一度だけ通知しキャラ別復旧で再通知する", async () => {
  const state = setup();
  const a = { id: "a", name: "キャラA", manifest: {} }, b = { id: "b", name: "キャラB", manifest: {} };
  state.voice.setCharacter(a);
  await state.voice.prepare("本文");
  await assert.rejects(state.voice.prepareModel());
  assert.equal(state.notified.length, 1);
  assert.match(state.notified[0].message, /キャラA/);
  state.voice.setCharacter(b);
  await state.voice.prepare("本文");
  state.reply(async () => ({ enabled: true, ready: true, audio: [1] }));
  await state.voice.prepare("本文");
  state.voice.setCharacter(a);
  state.reply(async () => { throw new Error("UUIDがありません"); });
  await state.voice.prepare("本文");
  assert.equal(state.notified.length, 2);
  state.reply(async () => ({ enabled: true, ready: true, audio: [1] }));
  await state.voice.prepare("本文");
  state.reply(async () => { throw new Error("UUIDがありません"); });
  await state.voice.prepare("本文");
  assert.equal(state.notified.length, 3);
});

test("モデル準備成功は未復旧の合成失敗を解除せず音声成功だけが解除する", async () => {
  const state = setup();
  state.voice.setCharacter({ id: "a", name: "キャラA", manifest: {} });
  await state.voice.prepare("本文");
  state.reply(async () => ({ enabled: true, ready: true }));
  await state.voice.prepareModel();
  state.reply(async () => { throw new Error("UUIDがありません"); });
  await state.voice.prepare("本文");
  assert.equal(state.notified.length, 1);
  state.reply(async () => ({ enabled: true, audio: [1] }));
  await state.voice.prepare("本文");
  state.reply(async () => { throw new Error("UUIDがありません"); });
  await state.voice.prepare("本文");
  assert.equal(state.notified.length, 2);
});

test("実際の上書きモデルの診断は表示キャラ名だけを置換して保持する", async () => {
  const state = setup();
  state.voice.setCharacter({ id: "mugi", name: "むぎ", manifest: { voice: { modelName: "既定モデル" } } });
  state.reply(async () => { throw new Error("mugiの声（上書きモデル/指定スタイル）を準備できません: 話者なし"); });
  await state.voice.prepare("本文");
  assert.match(state.notified[0].message, /むぎの声（上書きモデル\/指定スタイル）/);
  assert.doesNotMatch(state.notified[0].message, /既定モデル/);
});

test("音声APIの橋渡しはmanifestの第4引数を落とさない", () => {
  const bridge = readFileSync(new URL("../../ui/app/bridge.js", import.meta.url), "utf8");
  const api = readFileSync(new URL("../../ui/picoagent.js", import.meta.url), "utf8");
  assert.match(bridge, /api\.setCharacterProfile\(characterId, profile, greeting, voice\)/);
  assert.match(api, /pet_set_character_profile", \{ characterId, profile, greeting, voice \}/);
});

test("不正音量は既存値で合成せず無音を維持し正常保存後に再開する", async () => {
  const state = setup();
  let calls = 0;
  state.reply(async () => { calls += 1; return { enabled: true, audio: [1] }; });
  state.voice.setVolumeError("音量が整数ではありません");
  assert.equal(state.voice.gain(), 0);
  await state.voice.prepare("本文");
  assert.equal(calls, 0);
  state.voice.setVolumeError(null);
  state.voice.setVolume(35);
  await state.voice.prepare("本文");
  assert.equal(calls, 1);
  assert.ok(state.voice.gain() > 0);
});
