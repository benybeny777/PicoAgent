import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

await import("../../ui/alarm-runtime.js");

const index = await readFile(new URL("../../ui/index.html", import.meta.url), "utf8");
const renderer = await readFile(new URL("../../ui/renderer.js", import.meta.url), "utf8");

test("上限到達時も現在の発火記録を消さない", () => {
  const fired = new Set(Array.from({ length: 50 }, (_, index) => `old-${index}`));
  const key = "alarm-1@2026-08-18 07:00";

  assert.equal(globalThis.PicoAlarmRuntime.markFired(fired, key), true);
  assert.equal(fired.size, 1);
  assert.equal(fired.has(key), true);
  assert.equal(globalThis.PicoAlarmRuntime.markFired(fired, key), false);
});

test("実画面の目覚まし判定が共通の発火記録を使う", () => {
  assert.ok(index.indexOf('<script src="alarm-runtime.js') < index.indexOf('<script src="renderer.js'));
  assert.match(renderer, /PicoAlarmRuntime\.markFired\(firedAlarms, key\)/);
});
