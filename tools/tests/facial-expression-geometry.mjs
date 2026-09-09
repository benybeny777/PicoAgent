import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";
import path from "node:path";
import { fileURLToPath } from "node:url";

// 公開APIを増やさず、本番と同じ局所変形を隔離環境で検査する。
const source = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");
const context = vm.createContext({ window: {} });
vm.runInContext(source.replace("Object.freeze({ create })", "Object.freeze({ create, expressionDisplacement, lipMesh, cheekLayout, faceSupportWeight, MeshRigController })"), context);
const api = context.window.PicoMeshRig;
const pose = name => api.MeshRigController.prototype.expressionPose.call({ expression: name });
const rig = { drawables: { face: { textureBox: [0, 0, 300, 500] } } };

test("全7パックで口の素材領域が目に重ならない", () => {
  const overlaps = (a,b) => a[0]<b[2] && a[2]>b[0] && a[1]<b[3] && a[3]>b[1];
  const root = fileURLToPath(new URL("../../characters/", import.meta.url));
  for (const name of ["photoreal-prototype", "aoi", "mugi", "mugi-normal-lvs", "rin", "mai", "sena"]) {
    const folder = path.join(root, name);
    const manifest = JSON.parse(readFileSync(path.join(folder, "character.json"), "utf8"));
    const value = JSON.parse(readFileSync(path.join(folder, manifest.meshRig.rig), "utf8"));
    if (value.schema_version === 3) {
      const left = value.layers.left_eye_base.feature_box;
      const right = value.layers.right_eye_base.feature_box;
      assert.ok(!overlaps(value.layers.mouth_open.texture_box,left) && !overlaps(value.layers.mouth_open.texture_box,right), name);
    } else {
      assert.ok(value.anchors?.eyeL && value.anchors?.eyeR, `${name}: 実測座標が失われています`);
      for(const eye of [value.anchors.eyeL,value.anchors.eyeR]) {
        assert.ok(!overlaps(value.drawables.mouth_open.textureBox,[eye.x0,eye.y0,eye.x1,eye.y1]), name);
      }
    }
  }
});

test("顔の透明境界は固定し内部だけに表情変形を許す", () => {
  const face = {sourceAlpha:new Uint8Array(100*100).fill(255)};
  assert.equal(api.faceSupportWeight(face,[10,20,110,120],11,50),0);
  assert.equal(api.faceSupportWeight(face,[10,20,110,120],60,70),1);
  face.sourceAlpha.fill(0,45*100,46*100);
  assert.equal(api.faceSupportWeight({sourceAlpha:face.sourceAlpha},[10,20,110,120],60,65),0);
});

test("頬は傾いた顔でも同じ大きさで左右の目から同じ距離に置く", () => {
  const anchors = { eyeL: {x0:20,y0:50,x1:40,y1:60}, eyeR: {x0:70,y0:30,x1:90,y1:40}, mouth: {cx:65,cy:95} };
  const layout = api.cheekLayout(anchors);
  const a = layout.centers[0], b = layout.centers[1];
  assert.ok(Math.abs((b[0] - a[0]) - 50) < 0.00001);
  assert.ok(Math.abs((b[1] - a[1]) + 20) < 0.00001);
  const shifted = Object.fromEntries(Object.entries(anchors).map(([name, box]) => [name, Object.fromEntries(Object.entries(box).map(([key,value]) => [key,value + (key.includes("x") ? 137 : 81)]))]));
  const translated = api.cheekLayout(shifted);
  assert.ok(Math.abs(translated.centers[0][0] - a[0] - 137) < 0.00001);
  assert.ok(Math.abs(translated.centers[0][1] - a[1] - 81) < 0.00001);
  assert.equal(translated.radiusY, layout.radiusY);
});

test("中立と顔領域外の頂点は表情で動かさない", () => {
  for (const name of ["neutral", "happy", "concern", "thinking", "shy", "angry"]) {
    assert.deepEqual(Array.from(api.expressionDisplacement(rig, 700, 900, pose(name), true)), [0, 0]);
  }
  for (const point of [[90, 180], [190, 200], [110, 340]]) {
    assert.deepEqual(Array.from(api.expressionDisplacement(rig, ...point, pose("neutral"), true)), [0, 0]);
  }
});

test("怒りと困りは眉頭を逆方向へ動かし赤塗りにしない", () => {
  assert.ok(pose("angry").browInnerY > 0);
  assert.ok(pose("concern").browInnerY < 0);
  assert.equal(pose("angry").blush, 0);
});

test("発話で上体を駆動し胸は遅れて追従して無音後に収束する", () => {
  const controller = {
    rig: { mode: "lvs-anime25d-v1" }, mouth: 0, speechEnvelope: 0,
    speechBody: { value: 0, velocity: 0 }, chestFollow: { value: 0, velocity: 0 },
    angleX: 0, updateAnime25dMesh() {},
  };
  const tick = () => api.MeshRigController.prototype.updateMesh.call(controller, 0, 1 / 60);
  for (let i = 0; i < 120; i++) tick();
  assert.equal(controller.speechBody.value, 0);
  assert.equal(controller.chestFollow.value, 0);
  controller.mouth = 0.8;
  for (let i = 0; i < 20; i++) tick();
  assert.ok(controller.speechBody.value > 0.1);
  assert.ok(Math.abs(controller.chestFollow.value - controller.speechBody.value) > 0.01);
  controller.mouth = 0;
  for (let i = 0; i < 600; i++) tick();
  assert.ok(Math.abs(controller.chestFollow.value) < 0.0001);
  assert.ok(Math.abs(controller.speechBody.value) < 0.0001);
});

test("実写の笑顔と困り顔は口角を逆方向へ動かす", () => {
  const happy = api.expressionDisplacement(rig, 192, 342.5, pose("happy"), true);
  const concern = api.expressionDisplacement(rig, 192, 342.5, pose("concern"), true);
  assert.ok(happy[1] < -1);
  assert.ok(concern[1] > 1);
});

test("感情の曲率は母音幅を変えず唇へ反映する", () => {
  const layer = { texture_box: [0, 0, 100, 80], feature_box: [20, 30, 80, 45], lip_seam: [[20, 36], [50, 40], [80, 36]] };
  const happy = api.lipMesh(layer, 0, 0, 1);
  const concern = api.lipMesh(layer, 0, 0, -1);
  assert.equal(happy.upper[2][1][0], concern.upper[2][1][0]);
  assert.ok(happy.upper[2][1][1] > concern.upper[2][1][1]);
});

test("縦長のすぼめ口は開口中だけ横幅を解放し閉口形状を保持する", () => {
  const layer = {texture_box:[0,0,100,100], feature_box:[30,30,70,65], lip_seam:[[30,42],[50,46],[70,50]]};
  const closed = api.lipMesh(layer,0,0), opened = api.lipMesh(layer,1,0);
  assert.equal(closed.upper[1][1][0],30);
  assert.equal(closed.upper[1][1][1],42);
  assert.ok(opened.upper[1][1][0] < 30);
  const gap = opened.lower[2][0][1] - opened.upper[2][1][1];
  assert.ok(gap > 16);
});
