import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const styles = readFileSync(new URL("../../ui/styles.css", import.meta.url), "utf8");
const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");
const meshRig = readFileSync(new URL("../../ui/mesh-rig.js", import.meta.url), "utf8");

test("配信メッシュは縦基準で縦横比を維持する", () => {
  const selector =
    '.pet-app[data-layout="stream"]:not(.is-booting) .character[data-render-kind="mesh-rig"] .character-view > canvas';
  const start = styles.indexOf(selector);
  assert.ok(start >= 0, "メッシュ描画の一般指定が必要です");
  const rule = styles.slice(start, styles.indexOf("}", start) + 1);
  assert.match(rule, /width:auto!important/);
  assert.match(rule, /height:min\(calc\(76vh \* var\(--stream-mesh-scale,2\.6\) \/ 2\.6\),var\(--mesh-source-css-height/);
  assert.doesNotMatch(rule, /width:var\(--stream-render-size/);
  assert.match(renderer, /--stream-mesh-scale/);
  assert.match(renderer, /applyMeshRigFraming\(\)/);
  assert.match(renderer, /setFraming\(scale\)/);
  assert.match(meshRig, /visibleSourceHeight = this\.app\.screen\.height \/ this\.textureFit \/ this\.framingScale/);
  assert.match(meshRig, /visibleSourceHeight \/ dpr/);
  assert.match(meshRig, /addEventListener\("resize", this\.refreshSourceLimit\)/);
  assert.match(meshRig, /removeEventListener\("resize", this\.refreshSourceLimit\)/);
});

test("native-bustのメッシュ描画へ全身絵用クロップを適用しない", () => {
  const general = styles.indexOf(
    '.character[data-render-kind="mesh-rig"] .character-view > canvas',
  );
  const selector =
    '.character[data-render-kind="mesh-rig"][data-source-framing="native-bust"] .character-view > canvas';
  const override = styles.indexOf(selector);
  assert.ok(general >= 0, "メッシュ描画の一般指定が必要です");
  assert.ok(override > general, "native-bustの上書きは一般指定より後に置いてください");

  const rule = styles.slice(override, styles.indexOf("}", override) + 1);
  assert.match(rule, /width:auto!important/);
  assert.match(rule, /height:min\(calc\(76vh \* var\(--stream-mesh-scale,2\.6\) \/ 2\.6\),var\(--mesh-source-css-height/);
  assert.match(rule, /left:50%/);
  assert.match(rule, /translateX\(-50%\)/);
  assert.match(renderer, /isNativeBustImage && usesStreamImageCrop[\s\S]*FRAMING_PRESETS\.full/);
});

test("低頭身メッシュは配信でも低頭身用の胸上構図を保つ", () => {
  const general = styles.indexOf(
    '.character[data-render-kind="mesh-rig"] .character-view > canvas',
  );
  const selector =
    '.character[data-render-kind="mesh-rig"][data-body-proportion="compact"] .character-view > canvas';
  const override = styles.indexOf(selector);
  assert.ok(override > general, "低頭身メッシュの上書きは一般指定より後に置いてください");
  const rule = styles.slice(override, styles.indexOf("}", override) + 1);
  assert.match(rule, /height:min\(calc\(76vh \* var\(--stream-mesh-scale,2\.6\) \/ 2\.6\),var\(--mesh-source-css-height/);
  assert.match(rule, /left:50%/);
  assert.match(renderer, /character\.dataset\.renderKind === "mesh-rig"/);
  assert.match(renderer, /isCompactImage/);
});

test("Live2Dの小型アイコンはモデル固有の顔位置を適用できる", () => {
  assert.match(renderer, /function applyLive2DFraming\(\)/);
  assert.match(renderer, /activeLive2dConfig\.iconFraming/);
  assert.match(renderer, /activeLive2dConfig\.layoutFraming\?\.\[currentLayout\]/);
  assert.match(renderer, /applyLive2DFraming\(\);/);
});
