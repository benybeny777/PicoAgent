import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import test from "node:test";

// 公開APIを増やさず、本番と同じ局所変形を隔離環境で検査する。
const source = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");
const context = vm.createContext({ window: {} });
vm.runInContext(source.replace("Object.freeze({ create })", "Object.freeze({ create, expressionDisplacement, lipMesh, MeshRigController })"), context);
const api = context.window.PicoMeshRig;
const pose = name => api.MeshRigController.prototype.expressionPose.call({ expression: name });
const rig = { drawables: { face: { textureBox: [0, 0, 300, 500] } } };

test("中立と顔領域外の頂点は表情で動かさない", () => {
  for (const name of ["neutral", "happy", "concern", "thinking", "shy"]) {
    assert.deepEqual(Array.from(api.expressionDisplacement(rig, 700, 900, pose(name), true)), [0, 0]);
  }
  for (const point of [[90, 180], [190, 200], [110, 340]]) {
    assert.deepEqual(Array.from(api.expressionDisplacement(rig, ...point, pose("neutral"), true)), [0, 0]);
  }
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
