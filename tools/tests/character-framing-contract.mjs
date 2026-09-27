// キャラの構図（どこを切り出して枠へ収めるか）の契約。
//
// 旧画面は「キャンバス（枠）を拡大して中身を切る」5段の掛け算で構図を作っていたため、
// 拡大するとキャラが切れた。いまの構図はモデル座標上の矩形（full / upper / bust）で、
// 描画器はこの矩形を枠へ縦横比を保って下端中央に収めるだけ。枠の大きさを変えても切れない。
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import {
  FRAMING_NAMES, MAX_BACKING_EDGE, backingResolution, defaultFramingFor, fitTransform, framingRectsFor, resolveFraming,
} from "../../ui/character/framing.js";

const content = { x: 100, y: 50, w: 400, h: 1000 };
const within = (inner, outer) => inner.x >= outer.x - 1e-9 && inner.y >= outer.y - 1e-9
  && inner.x + inner.w <= outer.x + outer.w + 1e-9 && inner.y + inner.h <= outer.y + outer.h + 1e-9;

test("構図は3つの名前だけで、どれも全身の矩形の内側に収まる", () => {
  assert.deepEqual([...FRAMING_NAMES], ["full", "upper", "bust"]);
  for (const options of [{}, { proportion: "compact" }, { sourceFraming: "native-bust" }, { sourceFraming: "native-full" }]) {
    const rects = framingRectsFor(content, options);
    for (const name of FRAMING_NAMES) {
      assert.ok(within(rects[name], rects.full), `${JSON.stringify(options)} の ${name} が全身の外へ出ています`);
      // 頭の上は切らない（どの構図も全身と同じ上端から切り出す）。
      assert.equal(rects[name].y, rects.full.y);
    }
    assert.ok(rects.bust.h < rects.upper.h || options.sourceFraming === "native-bust", "胸上は上半身より狭い");
  }
});

test("全身の矩形は可視範囲の上と左右に余白を足し、下端は足さない", () => {
  const full = framingRectsFor(content).full;
  assert.ok(full.y < content.y, "髪の先が枠に接して切れて見えます");
  assert.ok(full.x < content.x && full.x + full.w > content.x + content.w);
  assert.equal(full.y + full.h, content.y + content.h, "足元の下へ余白を足すと浮いて見えます");
});

test("低頭身は同じ名前でも深めに切り出し、原画が胸上なら全身＝上半身", () => {
  const standard = framingRectsFor(content);
  const compact = framingRectsFor(content, { proportion: "compact" });
  assert.ok(compact.upper.h > standard.upper.h);
  assert.ok(compact.bust.h > standard.bust.h);
  const nativeBust = framingRectsFor(content, { sourceFraming: "native-bust" });
  assert.deepEqual(nativeBust.upper, nativeBust.full);
});

test("枠へ写すと切り出し矩形は枠の内側に下端中央で収まる（枠の大きさに依らない）", () => {
  const frame = framingRectsFor(content).bust;
  for (const [width, height] of [[100, 100], [400, 200], [120, 900], [3000, 1600]]) {
    const { scale, offsetX, offsetY } = fitTransform(frame, width, height);
    const left = frame.x * scale + offsetX;
    const top = frame.y * scale + offsetY;
    const right = left + frame.w * scale;
    const bottom = top + frame.h * scale;
    assert.ok(left >= -1e-9 && top >= -1e-9 && right <= width + 1e-9 && bottom <= height + 1e-9, `${width}×${height} で枠の外へ出ます`);
    assert.ok(Math.abs(bottom - height) < 1e-9, "下端に揃っていません");
    assert.ok(Math.abs((left + right) / 2 - width / 2) < 1e-9, "左右中央に揃っていません");
  }
});

test("保存した構図が最優先。無ければシーンの既定（配信は胸上）、それ以外は描画器の既定", () => {
  assert.equal(resolveFraming("full", "bust", {}, "upper"), "full");
  assert.equal(resolveFraming("", "bust", {}, "upper"), "bust");
  assert.equal(resolveFraming("", "bust", { sourceFraming: "native-bust" }, "upper"), "full");
  assert.equal(resolveFraming("", undefined, {}, "upper"), "upper");
  assert.equal(resolveFraming("zoom", undefined, {}, "bust"), "bust", "知らない名前は保存値として使わない");
  // 規則: 低頭身2Dは上半身、長身・実写・Live2Dは上半身〜胸上、原画が胸上なら全体。
  assert.equal(defaultFramingFor({ bodyProportion: "compact" }, "2d"), "upper");
  assert.equal(defaultFramingFor({}, "2d"), "bust");
  assert.equal(defaultFramingFor({}, "live2d"), "upper");
  assert.equal(defaultFramingFor({ sourceFraming: "native-bust" }, "mesh-rig"), "full");
});

test("描画バッファの1辺は上限を超えない（密度の側を下げる）", () => {
  assert.equal(backingResolution(400, 300, 2), 2);
  const ratio = backingResolution(3000, 1000, 2);
  assert.ok(Math.abs(3000 * ratio - MAX_BACKING_EDGE) < 1e-9);
});

test("メッシュは枠の大きさでキャンバスを作り直し、切り出し矩形を下端中央へ収める", () => {
  const meshRig = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");
  const root = { devicePixelRatio: 1, addEventListener() {}, removeEventListener() {} };
  runInNewContext(meshRig.replace("Object.freeze({ create })", "Object.freeze({ create, MeshRigController })"),
    { window: root, performance: { now: () => 0 } });
  const transforms = {};
  const resized = [];
  let renders = 0;
  const app = {
    screen: { width: 400, height: 1000 },
    renderer: { resolution: 1, resize: (w, h) => resized.push([w, h]) },
    stage: Object.fromEntries(["pivot", "position", "scale"].map(key => [key, { set: (...v) => { transforms[key] = v; } }])),
    ticker: { add() {}, stop() {}, remove() {} }, destroy() {}, render: () => { renders += 1; },
  };
  const controller = new root.PicoMeshRig.MeshRigController(app, { style: {}, remove() {} }, {}, [], null, null, 1, { x: 0, y: 0, w: 400, h: 1000 });
  controller.layout(200, 300, 2, { x: 100, y: 0, w: 200, h: 300 });
  assert.deepEqual(resized.at(-1), [200, 300]);
  assert.equal(app.renderer.resolution, 2);
  assert.equal(transforms.scale[0], 1);
  assert.deepEqual(transforms.position, [-100, 0]);
  assert.equal(renders, 1, "待機中も新しい大きさで描き直す");
});

test("描画器は構図をCSSの拡大で作らない", () => {
  // 旧画面の切れる原因（枠を transform: scale で拡大）を戻さない。
  const css = readdirSync(new URL("../../ui/components/", import.meta.url)).map(name => readFileSync(new URL(`../../ui/components/${name}`, import.meta.url), "utf8")).join("\n");
  assert.doesNotMatch(css, /--char-zoom|--char-render-size|--stream-mesh-scale|--mesh-source-css-height/);
  const character = readFileSync(new URL("../../ui/components/character.css", import.meta.url), "utf8");
  assert.doesNotMatch(character, /scale\(/);
});
