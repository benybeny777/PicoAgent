import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const preview = readFileSync(
  new URL("../mesh-rig-preview/index.html", import.meta.url),
  "utf8",
);
const renderer = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");

test("確認画面はA・むぎ・実写を本番メッシュ描画で切り替える", () => {
  assert.match(preview, /value="female-a"/);
  assert.match(preview, /value="mugi"/);
  assert.match(preview, /value="photoreal"/);
  assert.match(preview, /characters\/female-a-lvs/);
  assert.match(preview, /characters\/photoreal-prototype/);
  assert.doesNotMatch(preview, /品質NG: 現在の生成PSD/);
  assert.match(preview, /window\.PicoMeshRig\.create/);
});

test("LocalVTuberStudio schema 3を中間形式へ書き換えず読む", () => {
  assert.match(renderer, /sourceRig\.schema_version !== 3/);
  assert.match(renderer, /sourceRig\.profile !== "lvs-anime25d-v1"/);
  assert.match(renderer, /sourceRig\.scene_graph/);
  assert.match(renderer, /sourceRig\.layers\.mouth_closed/);
  assert.match(renderer, /eyeAperture\(sourceRig\.layers\.left_eye_base/);
  assert.match(renderer, /images\.has\(`\$\{side\}_eye_closed`\)/);
  assert.match(renderer, /rootUrl\}\/\$\{lvsPartPath\(name\)\}/);
});

test("確認画面は目・視線・5母音・表情・全動作・腕の操作を保つ", () => {
  assert.match(preview, /data-wink="L"/);
  assert.match(preview, /data-wink="R"/);
  assert.match(preview, /id="gazeX"/);
  assert.match(preview, /id="gazeY"/);
  for (const label of ["あ", "い", "う", "え", "お", "笑顔", "困り顔", "考え顔", "寝顔", "聞く", "起きる", "眠る"]) {
    assert.ok(preview.includes(`>${label}<`), `${label}の確認操作が必要です`);
  }
  assert.match(preview, /id="arm"/);
  assert.match(renderer, /setEyeOpen\(left, right = left\)/);
  assert.match(renderer, /apertureWidth \* 0\.08/);
  assert.match(renderer, /setExpression\(name\)/);
  assert.match(preview, /controller\.setEyeOpen\(0, 0\); await wait\(220\)/, "完全閉眼を保持する");
});

test("キャラクター切替では破棄されたcanvasを再利用しない", () => {
  const destroy = preview.indexOf("controller?.destroy()");
  const create = preview.indexOf('document.createElement("canvas")', destroy);
  const render = preview.indexOf("window.PicoMeshRig.create", create);
  assert.ok(destroy >= 0 && create > destroy && render > create);
});

test("独自2.5Dは首境界を開く局所頭部変形をせず一体で呼吸する", () => {
  assert.doesNotMatch(renderer, /const isNeck = name === "neck"/);
  assert.doesNotMatch(renderer, /motionPose\.headX \* headWeight/);
  assert.match(renderer, /y -= breath \* 2 \* faceScale/);
});
