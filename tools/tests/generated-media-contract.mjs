import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = path => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

test("生成画像はgenerated-media内だけを表示し、既定アプリで開く", () => {
  const renderer = read("ui/renderer.js");
  const bridge = read("ui/picoagent.js");
  const commands = read("src-tauri/src/commands.rs");
  const lib = read("src-tauri/src/lib.rs");
  const conf = read("src-tauri/tauri.conf.json");
  assert.match(renderer, /function isGeneratedMediaPath\(path\)[\s\S]*?generated-media/);
  assert.match(renderer, /img\.src = window\.picoagent\.assetUrl\(path\)/);
  assert.match(bridge, /openGeneratedMedia:\s*\(path\)\s*=>\s*call\("pet_open_generated_media"/);
  assert.match(commands, /pub fn pet_open_generated_media\(/);
  assert.match(commands, /target\.parent\(\) != Some\(root\.as_path\(\)\)/);
  assert.match(lib, /commands::pet_open_generated_media,/);
  assert.match(conf, /"\$DATA\/PicoAgent\/generated-media\/\*\*"/);
});
