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
assert.equal(format.layers.length, 22);
assert.deepEqual(
  format.layers.filter(name => name === "base" || name.startsWith("blink_")),
  ["base", "blink_17", "blink_33", "blink_half", "blink_67", "blink_83", "blink_closed"],
  "まばたきは開眼を含む7段階",
);
assert.deepEqual(
  format.layers.filter(name => name.startsWith("mouth_")),
  ["mouth_closed", "mouth_17", "mouth_33", "mouth_half", "mouth_67", "mouth_83", "mouth_open"],
  "口パクは閉口から最大開口まで7段階",
);
assert.equal(template.characterFormat, format.format);
assert.equal(typeof template.frameGeneration, "string");
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
  if (manifest.live2d) continue;
  const isPortrait = manifest.characterFormat === format.format;
  if (isPortrait) {
    assert.match(manifest.frameGeneration || "", /^[0-9a-f]{16}$/, `${entry.name}: frameGeneration`);
    for (const layer of format.layers) {
      await readFile(new URL(`characters/${entry.name}/layers/${layer}.png`, root));
    }
  }
  if (manifest.motionProfile !== "full" && !isPortrait) continue;
  for (const [state, rule] of Object.entries(format.animations)) {
    const paths = manifest.animations?.[state]?.framePaths || [];
    const hashes = new Set();
    for (const relative of paths) {
      const bytes = await readFile(join(fileURLToPath(new URL(`characters/${entry.name}/`, root)), relative));
      if (isPortrait) {
        assert.ok(
          relative.startsWith(`frames/${manifest.frameGeneration}/`),
          `${entry.name}/${state}: 現世代以外を参照`,
        );
      }
      hashes.add(createHash("sha256").update(bytes).digest("hex"));
    }
    assert.ok(paths.length >= rule.frames, `${entry.name}/${state}: 総コマ数が不足`);
    assert.ok(hashes.size >= rule.minUniqueFrames, `${entry.name}/${state}: 固有絵が不足`);
  }
}

const builder = await readFile(new URL("scripts/build-portrait-2d-character.py", root), "utf8");
assert.match(builder, /"frameGeneration": generation/);
assert.match(builder, /f"frames\/\{generation\}\//);
assert.doesNotMatch(builder, /action_paths\.get\(state, \["preview\.png"\]\)/);
assert.equal((builder.match(/required=True/g) || []).length >= 3, true, "連番・22レイヤー・顔ROIを必須にする");
assert.match(builder, /ensure_changes_inside_roi/, "顔ROI外のRGBA一致を検査する");

const renderer = await readFile(new URL("ui/renderer.js", root), "utf8");
assert.match(renderer, /model\.internalModel\.physics = null;/, "Live2D物理を無効化する");
assert.doesNotMatch(renderer, /configureSpringMotion|animateSpringMotion|characterSpringMotion/);

console.log("portrait-2d contract: OK");
