// 2D描画器の破棄時に、画像取得と背景プリロードが復活しないことを確認する。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { createFrameGate } from "../../ui/character/frame-budget.js";

function fixture(immediate = false) {
  const images = [];
  const timers = new Map();
  let sequence = 0;
  class Image {
    constructor() { this.complete = immediate; this.naturalWidth = 2; this.naturalHeight = 2; images.push(this); }
    set src(value) { this.url = value; }
    removeAttribute() { this.removed = true; }
  }
  const context = { drawImage() {}, getImageData: () => ({ data: new Uint8ClampedArray(16).fill(255) }) };
  const source = readFileSync(new URL("../../ui/character/image.js", import.meta.url), "utf8")
    .replace(/^import[\s\S]*?;\r?\n/gm, "").replace("export class ImageRenderer", "class ImageRenderer");
  const Renderer = runInNewContext(`${source}\nImageRenderer`, {
    createFrameGate,
    Image, HTMLImageElement: Image, AbortController, queueMicrotask, performance,
    setTimeout: callback => { const id = ++sequence; timers.set(id, callback); return id; },
    clearTimeout: id => timers.delete(id),
    el: () => ({ dataset: {}, getContext: () => context, remove() {} }),
    packUrl: (pack, path) => `${pack.root}/${path}`,
    framingRectsFor: () => ({}), clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    window: { PicoFrameScheduler: { createLoop: () => ({ start() {}, stop() {} }) } },
  });
  const renderer = new Renderer();
  renderer.pack = { root: "character" };
  return { renderer, images, timers };
}

test("取得中に破棄すると画像取得を中断し、Mapへ再登録しない", async () => {
  const { renderer, images, timers } = fixture();
  const pending = renderer.preload("idle.png");
  assert.equal(images.length, 1);
  renderer.dispose();
  assert.equal(await pending, null);
  assert.equal(images[0].removed, true);
  assert.equal(renderer.images.size, 0);
  assert.equal(timers.size, 0);
  assert.equal(await renderer.preload("talking.png"), null);
  assert.equal(images.length, 1);
});

test("背景プリロードの予定を破棄で回収し、遅れたタイマーも取得を始めない", async () => {
  const { renderer, images, timers } = fixture(true);
  await renderer.load({ root: "character", animations: {
    idle: { framePaths: ["idle.png"] }, talking: { framePaths: ["talking.png"] },
  } });
  const scheduled = [...timers.values()];
  renderer.dispose();
  assert.equal(timers.size, 0);
  for (const callback of scheduled) callback();
  assert.equal(images.some(image => image.url.endsWith("talking.png")), false);
  assert.equal(renderer.images.size, 0);
});
