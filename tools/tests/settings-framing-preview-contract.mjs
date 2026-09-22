import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const settings = readFileSync(new URL("../../ui/settings.js", import.meta.url), "utf8");
const editor = settings.slice(settings.indexOf("function buildCharacterFramingEditor()"));
const updateSource = editor.slice(editor.indexOf("  const previewPositionScale ="), editor.indexOf("  const load = async"));
const moveSource = editor.slice(editor.indexOf('  preview.addEventListener("pointermove"'), editor.indexOf("  const endDrag ="));

function previewHarness(height, zoomValue, xValue, yValue) {
  const image = { clientHeight: height, style: {} };
  const handlers = new Map();
  const context = {
    preview: {
      querySelector: () => image,
      addEventListener: (name, callback) => handlers.set(name, callback),
    },
    x: { value: String(xValue) }, y: { value: String(yValue) }, zoom: { value: String(zoomValue) },
    zoomValue: {}, positionValue: {}, presetSelect: {},
    drag: { pointerId: 7, startX: 0, startY: 0, x: xValue, y: yValue },
  };
  runInNewContext(`${updateSource}\nfunction save() { updatePreview(); }\n${moveSource}\nupdatePreview();`, context);
  const geometry = () => {
    const match = image.style.transform.match(/translate\(calc\(-50% \+ ([\d.e+-]+)px\),([\d.e+-]+)px\) scale\(([\d.e+-]+)\)/);
    assert.ok(match, "構図プレビューの位置と倍率を読み取れません");
    return match.slice(1).map(Number);
  };
  return { context, geometry, move: event => handlers.get("pointermove")(event) };
}

test("構図プレビューは本体の形式補正を位置へも掛け、保存座標を変えない", () => {
  // 本体は scale:1.32 の内側で translate(100px,-40px) するため、最終移動は132px,-52.8px。
  // プレビュー寸法へ縮めた位置を比較し、拡大率が変わっても移動量が変わらないことも確かめる。
  for (const height of [182, 315]) {
    for (const zoom of [1, 2.05, 4]) {
      const { context, geometry } = previewHarness(height, zoom, 100, -40);
      const [x, y, scale] = geometry();
      assert.ok(Math.abs(x - 132 * height / 315) < 1e-9);
      assert.ok(Math.abs(y - -52.8 * height / 315) < 1e-9);
      assert.equal(scale, zoom * 1.32);
      assert.equal(context.x.value, "100");
      assert.equal(context.y.value, "-40");
    }
  }
});

test("構図ドラッグは補正後の位置を逆変換し、画像をポインターへ追従させる", () => {
  for (const height of [182, 315]) {
    const harness = previewHarness(height, 2.05, 100, -40);
    const before = harness.geometry();
    harness.move({ pointerId: 7, clientX: 12, clientY: -8 });
    const after = harness.geometry();
    // 保存座標を整数へ丸める分だけ誤差を認める。
    const roundingError = height / 315 * 1.32 / 2 + 1e-9;
    assert.ok(Math.abs(after[0] - before[0] - 12) <= roundingError);
    assert.ok(Math.abs(after[1] - before[1] + 8) <= roundingError);
    assert.equal(harness.context.presetSelect.value, "custom");
  }
});
