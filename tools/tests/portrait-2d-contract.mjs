import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = new URL("../../", import.meta.url);
const readJson = async relative => JSON.parse(await readFile(new URL(relative, root), "utf8"));
const format = await readJson("characters/portrait-2d-format.json");
const template = await readJson(
  ".agents/skills/rebuild-picoagent-character/assets/portrait-2d-template/character.template.json",
);

assert.equal(format.format, "picoagent-portrait-2d-v1");
assert.equal(format.physics, false);
assert.equal(format.layers.length, 13);
assert.equal(template.characterFormat, format.format);
assert.equal("secondaryMotion" in template, false);
assert.equal("springPhysics" in template, false);
for (const [state, rule] of Object.entries(format.animations)) {
  assert.equal(template.animations[state].frames, rule.frames, `テンプレートの${state}総コマ数`);
  assert.equal(template.animations[state].fps, rule.fps, `テンプレートの${state}fps`);
}

const charactersRoot = new URL("characters/", root);
for (const entry of await readdir(charactersRoot, { withFileTypes: true })) {
  if (!entry.isDirectory()) continue;
  let manifest;
  try {
    manifest = await readJson(`characters/${entry.name}/character.json`);
  } catch (error) {
    if (error.code === "ENOENT") continue;
    throw error;
  }
  assert.equal("secondaryMotion" in manifest, false, `${entry.name}: secondaryMotionは廃止済み`);
  assert.equal("springPhysics" in manifest, false, `${entry.name}: springPhysicsは廃止済み`);
  for (const variant of manifest.variants || []) {
    assert.equal("secondaryMotion" in variant, false, `${entry.name}: variant secondaryMotionは廃止済み`);
    assert.equal("springPhysics" in variant, false, `${entry.name}: variant springPhysicsは廃止済み`);
  }
  if (manifest.live2d || manifest.motionProfile !== "full") continue;
  for (const [state, rule] of Object.entries(format.animations)) {
    const paths = manifest.animations?.[state]?.framePaths || [];
    const hashes = new Set();
    for (const relative of paths) {
      const bytes = await readFile(join(fileURLToPath(new URL(`characters/${entry.name}/`, root)), relative));
      hashes.add(createHash("sha256").update(bytes).digest("hex"));
    }
    assert.ok(paths.length >= rule.frames, `${entry.name}/${state}: 総コマ数が不足`);
    assert.ok(hashes.size >= rule.minUniqueFrames, `${entry.name}/${state}: 固有絵が不足`);
  }
}

const renderer = await readFile(new URL("ui/renderer.js", root), "utf8");
assert.match(renderer, /model\.internalModel\.physics = null;/, "Live2D物理を無効化する");
assert.doesNotMatch(renderer, /configureSpringMotion|animateSpringMotion|characterSpringMotion/);

console.log("portrait-2d contract: OK");
