import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import { fitTransform } from "../../ui/character/framing.js";

const manifest = JSON.parse(readFileSync(new URL("../../characters/mao/character.json", import.meta.url), "utf8"));
const source = readFileSync(new URL("../../ui/character/live2d.js", import.meta.url), "utf8")
  .replace(/^import .*;$/gm, "").replace("export class Live2dRenderer", "class Live2dRenderer");
const context = vm.createContext({ el: () => ({}) });
vm.runInContext(source, context);
const Live2dRenderer = vm.runInContext("Live2dRenderer", context);

function renderer(config) {
  const result = new Live2dRenderer();
  result.config = config;
  // 切り出し位置の差と倍率は元モデル寸法から独立する。代表寸法で実描画器の計算を検査する。
  result.originalWidth = 2000;
  result.originalHeight = 3000;
  result.buildFramings();
  return result;
}

function previousRenderer() {
  const config = structuredClone(manifest.live2d);
  // 実画素検査で帽子の先が胸上枠の上端へ接していた旧調整。
  config.layoutFraming.stream.offsetY = 30;
  return renderer(config);
}

test("まお配信の胸上は旧上端の帽子先に余白を作り、描画倍率を維持する", () => {
  const before = previousRenderer();
  const after = renderer(manifest.live2d);
  before.setLayoutId("stream");
  after.setLayoutId("stream");
  const oldFrame = before.framingRect("bust");
  const newFrame = after.framingRect("bust");
  assert.equal(newFrame.w, oldFrame.w);
  assert.equal(newFrame.h, oldFrame.h);
  for (const [width, height] of [[603, 533], [400, 500], [900, 800]]) {
    const oldFit = fitTransform(oldFrame, width, height);
    const newFit = fitTransform(newFrame, width, height);
    assert.equal(newFit.scale, oldFit.scale, "帽子の欠けを全体縮小で隠さない");
    const oldHatTop = oldFrame.y * oldFit.scale + oldFit.offsetY;
    const newHatTop = oldFrame.y * newFit.scale + newFit.offsetY;
    assert.ok(newHatTop - oldHatTop > height * 0.03, "旧上端から十分な余白を確保する");
  }
});

test("まおのペット・ミニと全身構図は配信補正で変えない", () => {
  const before = previousRenderer();
  const after = renderer(manifest.live2d);
  for (const layoutId of ["pet", "mascot", "stream"]) {
    before.setLayoutId(layoutId);
    after.setLayoutId(layoutId);
    const names = layoutId === "stream" ? ["full"] : ["full", "upper", "bust"];
    for (const name of names) {
      assert.deepEqual(after.framingRect(name), before.framingRect(name), `${layoutId}/${name}`);
    }
  }
});
