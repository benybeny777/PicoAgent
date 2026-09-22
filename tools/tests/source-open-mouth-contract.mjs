import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");
const context = { window: {} };
runInNewContext(source.replace("Object.freeze({ create })", "Object.freeze({ create, sourceOpenMouthGeometry, drawSourceOpenMouth, MeshRigController })"), context);
const api = context.window.PicoMeshRig;
const rig = JSON.parse(readFileSync(new URL("../../characters/mugi/rig2d/rig.json", import.meta.url), "utf8"));
const layer = rig.layers.mouth_closed;

test("原画開口は細い閉口から単調に開き、最大開口で原画へ一致する", () => {
  let previousHeight = 0;
  for (const open of [0, 0.01, 0.2, 0.5, 0.78, 1]) {
    const mesh = api.sourceOpenMouthGeometry(layer, open, 0);
    const column = mesh.target[8];
    const height = column[1][1] - column[0][1];
    assert.ok(height > previousHeight);
    if (open === 0) assert.ok(height < 2, "閉口時に原画の開いた口を残しません");
    if (open === 1) assert.deepEqual(mesh.target, mesh.source);
    previousHeight = height;
  }
  const closed = api.sourceOpenMouthGeometry(layer, 0, 0);
  api.sourceOpenMouthGeometry(layer, 0.78, 1, 0.6);
  assert.deepEqual(api.sourceOpenMouthGeometry(layer, 0, 0), closed, "発話後に閉口へ戻ります");
});

test("口形の横幅は原画の前後8%に抑え、曲率は口角へ局所的に反映する", () => {
  const width = layer.feature_box[2] - layer.feature_box[0];
  for (const form of [-1, 0, 1]) {
    const mesh = api.sourceOpenMouthGeometry(layer, 0.78, form);
    const actual = mesh.target.at(-1)[0][0] - mesh.target[0][0][0];
    assert.ok(actual >= width * 0.92 - 1e-9 && actual <= width * 1.08 + 1e-9);
    for (const column of mesh.target) assert.ok(column.flat().every(Number.isFinite));
  }
  const neutral = api.sourceOpenMouthGeometry(layer, 0, 0);
  const happy = api.sourceOpenMouthGeometry(layer, 0, 0, 0.6);
  const concern = api.sourceOpenMouthGeometry(layer, 0, 0, -1.6);
  assert.ok(happy.target[0][0][1] < neutral.target[0][0][1]);
  assert.ok(concern.target[0][0][1] > neutral.target[0][0][1]);
  assert.deepEqual(happy.target[8], neutral.target[8]);
  assert.deepEqual(concern.target[8], neutral.target[8]);
});

test("最大開口は原画の口領域だけを直接描き、歯や口内を生成しない", () => {
  const calls = [];
  const image = {};
  api.drawSourceOpenMouth({ drawImage: (...args) => calls.push(args) }, image, layer, 1, 0);
  const [left, top, right, bottom] = layer.feature_box;
  assert.deepEqual(calls, [[image, left - layer.texture_box[0], top - layer.texture_box[1], right - left, bottom - top,
    left, top, right - left, bottom - top]]);
});

test("通常の母音は口全体を一度で描き、分割の継ぎ目を作らない", () => {
  for (const opening of [0, 0.3, 0.78, 1]) {
    for (const form of [-1, 0, 1]) {
      const calls = [];
      api.drawSourceOpenMouth({ drawImage: (...args) => calls.push(args) }, {}, layer, opening, form);
      assert.equal(calls.length, 1);
      assert.equal(calls[0].length, 9, "素材全体ではなく口領域だけを切り出します");
    }
  }
});

test("閉口でも肌パッチを先に描き、人物IDに依存せず原画口を局所合成する", () => {
  const draws = [];
  const images = new Map(["scene_face", "mouth_open", "mouth_closed"].map(name => [name, { name }]));
  const ctx = {
    save() {}, restore() {}, clearRect() {}, translate() {}, beginPath() {}, rect() {}, clip() {},
    moveTo() {}, lineTo() {}, closePath() {}, transform() {}, putImageData() {},
    drawImage: (...args) => draws.push(args),
    getImageData: () => ({ data: new Uint8ClampedArray(4) }),
    createLinearGradient() { throw new Error("原画開口では人工の口内・歯を描きません"); },
  };
  const state = {
    rig: { sourceRig: rig, mouthMode: "source-open" },
    lvsFace: { ctx, images, canvas: { width: 1, height: 1 }, sourceAlpha: new Uint8Array([255]), texture: { baseTexture: { update() {} } } },
    mouth: 0, mouthForm: 0, eyeOpenL: 1, eyeOpenR: 1, gazeX: 0, gazeY: 0,
    expressionPose: () => ({ mouthOpen: 0, mouthForm: 0, mouthCurve: 0, eye: 1 }),
  };
  api.MeshRigController.prototype.updateLvsAppearance.call(state);
  assert.deepEqual(draws.slice(0, 2).map(args => args[0].name), ["scene_face", "mouth_open"]);
  assert.ok(draws.slice(2).every(args => args[0].name === "mouth_closed"));
  assert.doesNotMatch(api.sourceOpenMouthGeometry.toString(), /mugi|characterId/);
  const scaledLayer = { texture_box: layer.texture_box.map(value => value * 2), feature_box: layer.feature_box.map(value => value * 2) };
  const a = api.sourceOpenMouthGeometry(layer, 0.4, -0.5, 0.6);
  const b = api.sourceOpenMouthGeometry(scaledLayer, 0.4, -0.5, 0.6);
  assert.deepEqual(b.target, a.target.map(column => column.map(point => point.map(value => value * 2))));
});

test("未知の口モードを明示エラーにし、本体とプレビューの指定を一致させる", async () => {
  await assert.rejects(api.create({ mouthMode: "unknown" }), /未対応のメッシュ口描画モード/);
  await assert.rejects(api.create({}), /PIXI 6/, "省略は従来のsplitとして受け付けます");
  const manifest = JSON.parse(readFileSync(new URL("../../characters/mugi/character.json", import.meta.url), "utf8"));
  assert.equal(manifest.meshRig.mouthMode, "source-open");
  const main = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
  const preview = readFileSync(new URL("../mesh-rig-preview/index.html", import.meta.url), "utf8");
  assert.match(main, /mouthMode: cfg\.mouthMode/);
  assert.match(preview, /mouthMode: manifest\.meshRig\.mouthMode/);
});
