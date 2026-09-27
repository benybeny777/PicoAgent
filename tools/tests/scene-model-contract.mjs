// シーン（形式ごとの画面構成）の計算の契約。ui/app/scene-model.js を直接動かす。
// ソースの矩形はアンカー（9点）からのずれと大きさだけで表し、倍率を持たない。
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import {
  ANCHORS, MIN_SIZE, cloneScenes, containRect, fitScene, fromRect, keepReachable, nearestAnchor, resolvePlacement, toRect, writeSaved,
} from "../../ui/app/scene-model.js";

const scenesDir = new URL("../../ui/scenes/", import.meta.url);
const scenes = Object.fromEntries(readdirSync(scenesDir).filter(name => name.endsWith(".json"))
  .map(name => [name.replace(/\.json$/, ""), JSON.parse(readFileSync(new URL(name, scenesDir), "utf8"))]));

test("形式はペット・ミニ・配信の3つで、既定のシーン定義がそろっている", () => {
  assert.deepEqual(Object.keys(scenes).sort(), ["mascot", "pet", "stream"]);
  const layoutRs = readFileSync(new URL("../../src-tauri/src/layout.rs", import.meta.url), "utf8");
  for (const id of Object.keys(scenes)) {
    assert.equal(scenes[id].id, id);
    assert.ok(layoutRs.includes(`"${id}"`), `${id} がバックエンドの形式一覧にありません`);
    assert.ok(scenes[id].sources.character, `${id} にキャラがありません`);
    for (const name of scenes[id].order) assert.ok(scenes[id].sources[name], `${id} の重ね順 ${name} に定義がありません`);
  }
  // 配信は 1920×1080 固定で、窓へは1回だけ等倍率で収める。ペット・ミニは窓の大きさそのもの。
  assert.deepEqual(scenes.stream.size, { w: 1920, h: 1080 });
  assert.equal(scenes.pet.size, "window");
  assert.equal(scenes.mascot.size, "window");
});

test("シーン定義にもソースにも倍率（scale / zoom）を持たせない", () => {
  const text = JSON.stringify(scenes);
  assert.doesNotMatch(text, /"(scale|zoom)"/);
});

test("アンカー基準の値と矩形は往復で一致する（どのアンカーでも）", () => {
  const rect = { left: 120, top: 80, width: 300, height: 200 };
  for (const anchor of ANCHORS) {
    const placement = fromRect(rect, anchor, 1000, 700);
    assert.deepEqual(toRect(placement, 1000, 700), rect, anchor);
  }
});

test("窓の伸縮ではアンカー側の辺が付いていく", () => {
  const placement = fromRect({ left: 700, top: 400, width: 200, height: 300 }, "bottom-right", 1000, 700);
  const wider = toRect(placement, 1200, 900);
  assert.equal(1200 - (wider.left + wider.width), 100, "右端からの距離が保たれていません");
  assert.equal(900 - (wider.top + wider.height), 0, "下端からの距離が保たれていません");
  assert.equal(nearestAnchor({ left: 700, top: 400, width: 200, height: 300 }, 1000, 700), "bottom-right");
  assert.equal(nearestAnchor({ left: 400, top: 300, width: 200, height: 100 }, 1000, 700), "center");
});

test("保存値が無ければ既定定義から、キャラは構図の縦横比で幅を決める", () => {
  const placement = resolvePlacement(scenes.pet.sources.character, null, 900, 600, 0.5);
  assert.equal(placement.saved, false);
  assert.ok(Math.abs(placement.w / placement.h - 0.5) < 1e-9);
  // 幅の上限を越えたら高さごと縮めて縦横比を保つ。
  const wide = resolvePlacement(scenes.pet.sources.character, null, 900, 600, 2);
  assert.ok(wide.w <= 900 * scenes.pet.sources.character.maxWidthFrac + 1e-9);
  assert.ok(Math.abs(wide.w / wide.h - 2) < 1e-9);
  const saved = resolvePlacement(scenes.pet.sources.character, { anchor: "top-left", x: 1, y: 2, w: 300, h: 600 }, 900, 600, 0.5);
  assert.deepEqual([saved.x, saved.y, saved.w, saved.h, saved.saved], [1, 2, 300, 600, true]);
});

test("配信MMDだけ胸元を字幕上へ置き、保存済み配置を優先する", () => {
  const definition = scenes.stream.sources.character;
  const image = resolvePlacement(definition, null, 1920, 1080, 0.9, "mesh-rig");
  const mmd = resolvePlacement(definition, null, 1920, 1080, 0.9, "mmd");
  assert.equal(image.h, 480);
  assert.equal(mmd.h, 711);
  assert.equal(mmd.w, 639.9);
  assert.equal(mmd.y, -196);
  const saved = resolvePlacement(definition, { anchor: "center", x: 0, y: 0, w: 600, h: 600 }, 1920, 1080, 0.9, "mmd");
  assert.deepEqual([saved.y, saved.w, saved.h, saved.saved], [0, 600, 600, true]);
});

test("最小の大きさを下回らず、窓外へ消えない", () => {
  assert.equal(fromRect({ left: 0, top: 0, width: 5, height: 5 }, "top-left", 100, 100).w, MIN_SIZE);
  const lost = keepReachable({ left: 5000, top: -5000, width: 200, height: 200 }, 800, 600);
  assert.ok(lost.left <= 800 - 48 && lost.top + lost.height >= 48, "掴み直せる幅が残っていません");
  // 会話パネルは窓の内側へ完全に収める（入力欄・ボタンを窓外へ出さない）。
  const inside = containRect({ left: -40, top: 500, width: 900, height: 300 }, 400, 600, 6);
  assert.ok(inside.left >= 6 && inside.top >= 6 && inside.left + inside.width <= 394 && inside.top + inside.height <= 594);
});

test("配信シーンは窓へ等倍率で収め、余白を左右か上下に均等に取る", () => {
  const fit = fitScene(1920, 1080, 1280, 800);
  assert.equal(fit.scale, 1280 / 1920);
  assert.equal(fit.offsetX, 0);
  assert.equal(fit.offsetY, (800 - 1080 * fit.scale) / 2);
});

test("保存は形式ごと・キャラごとの差分だけで、空になった入れ物は残さない", () => {
  const saved = cloneScenes({ version: 1 });
  writeSaved(saved, "pet", { characterId: "mugi" }, { anchor: "top-left", x: 0, y: 0, w: 100, h: 200, framing: "bust" });
  writeSaved(saved, "stream", { source: "chat" }, { anchor: "right", x: 0, y: 0, w: 300, h: 600 });
  assert.equal(saved.pet.characters.mugi.framing, "bust");
  assert.equal(saved.mascot, undefined, "別の形式へ漏れています");
  writeSaved(saved, "pet", { characterId: "mugi" }, null);
  assert.equal(saved.pet, undefined);
  assert.deepEqual(Object.keys(saved).sort(), ["stream", "version"]);
});
