import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const shoot = await readFile(new URL("../screenshots/shoot.mjs", import.meta.url), "utf8");

test("Live2D撮影は共用PIXIの現行配置を参照する", () => {
  assert.match(shoot, /ui\/vendor\/pixi\/pixi\.min\.js/);
  assert.doesNotMatch(shoot, /ui\/vendor\/live2d\/pixi\.min\.js/);
});
