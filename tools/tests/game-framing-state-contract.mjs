// ゲーム中のキャラ構図は一時的な上書きで、保存済みの配置・構図を変えない契約。
// 旧画面は形式ごとの倍率を書き換えて戻していたため、閉じた後に別形式の構図が残ることがあった。
// いまは stage の一時状態（overrides.game）だけで描き、保存値へは触れない。
import assert from "node:assert/strict";
import test from "node:test";
import { uiFile } from "./lib/ui-source.mjs";

const stage = uiFile("app/stage.js");
const game = uiFile("app/game.js");
const characterSource = uiFile("sources/character.js");

function body(source, signature) {
  const start = source.indexOf(signature);
  assert.ok(start >= 0, `${signature} が見つかりません`);
  return source.slice(start, source.indexOf("\n    },", start) + 1 || undefined);
}

test("ゲーム中はどの形式でも胸上で描き、閉じると保存された構図へ戻る", () => {
  assert.match(stage, /function currentFraming\(\) \{\s*\n\s*return overrides\.game \? GAME_FRAMING : savedFraming\(\);/);
  // 描画器へ渡す構図は currentFraming（形式を切り替えても毎回ここを通る）。
  assert.match(characterSource, /context\.host\.setLayout\(layoutId, context\.stage\.currentFraming\(\)\);/);
  // ゲームの開閉は一時状態を切り替えて並べ直すだけ（保存しない）。
  const setGame = body(stage, "setGame(on) {");
  assert.match(setGame, /overrides\.game = Boolean\(on\);/);
  assert.doesNotMatch(setGame, /persist\(|commit\(|writeSaved\(/);
  assert.match(game, /function open\(\) \{[\s\S]*?stage\.setGame\(true\);/);
  assert.match(game, /function close\(\) \{[\s\S]*?stage\.setGame\(false\);/);
});

test("保存する構図と保存済み矩形の縦横比は、ゲームの一時構図を使わない", () => {
  const commit = body(stage, "async commit(target, windowRect, extra = {}) {");
  assert.match(commit, /value\.framing = extra\.framing \|\| previous\?\.framing \|\| savedFraming\(\);/);
  assert.doesNotMatch(commit, /currentFraming\(\)/);
  assert.match(stage, /character\.framingAspect\(savedFraming\(\)\)/);
  // ゲーム中のキャラ矩形は保存値を使わず、ゲーム画面の右側へ置く。
  assert.match(stage, /if \(overrides\.game && target === "character"\) rect = gameCharacterRect\(geometry\);/);
  // ゲーム中はキャラ以外のソースを隠す。
  assert.match(stage, /!\(overrides\.game && name !== "character"\)/);
});
