import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const styles = readFileSync(new URL("../../ui/styles.css", import.meta.url), "utf8");
const renderer = readFileSync(new URL("../../ui/renderer.js", import.meta.url), "utf8");

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
  assert.match(rule, /height:135%!important/);
  assert.match(rule, /left:50%/);
  assert.match(rule, /translateX\(-50%\)/);
});

test("native-fullの独自2.5Dを配信画面で過大クロップしない", () => {
  assert.match(renderer, /\["native-bust", "native-full"\]\.includes\(manifest\.sourceFraming\)/);
  const selector =
    '.character[data-render-kind="mesh-rig"][data-source-framing="native-full"] .character-view > canvas';
  const override = styles.indexOf(selector);
  assert.ok(override >= 0, "native-fullの上書きが必要です");
  const rule = styles.slice(override, styles.indexOf("}", override) + 1);
  assert.match(rule, /width:auto!important/);
  assert.match(rule, /height:100%!important/);
  assert.match(rule, /left:50%/);
  assert.match(rule, /translateX\(-50%\)/);
  assert.doesNotMatch(rule, /260%/);
});
