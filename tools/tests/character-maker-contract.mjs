import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("キャラ作成は現行4工程だけをPicoAgent設定へ公開する", async () => {
  const [pipeline, settings] = await Promise.all([
    read("character-maker/src/pipeline.rs"),
    read("ui/settings.js"),
  ]);
  assert.match(pipeline, /\["isolate", "decompose", "rig2d", "complete"\]/);
  assert.match(settings, /id: "character-maker", label: "キャラ作成（テスト）"/);
  for (const retired of ["mesh/generate.py", "rigging/generate.py", "facepatch", "obs"]) {
    assert.doesNotMatch(pipeline, new RegExp(retired.replace("/", "\\/")));
  }
});

test("管理導入は共通Python一環境と容量・GPU判定を先に行う", async () => {
  const components = await read("src-tauri/src/components.rs");
  assert.match(components, /join\("python\/Scripts\/python\.exe"\)/);
  assert.match(components, /85_000_000_000/);
  assert.match(components, /nvidia-smi\.exe/);
  assert.match(components, /約77GB/);
  assert.doesNotMatch(components.toLowerCase(), /tha3/);
});

test("採用モデルだけを固定SHAで管理領域へ取得する", async () => {
  const setup = await read("character-maker/installer/setup-runtime-models.py");
  const hashes = [...setup.matchAll(/"([0-9a-f]{64})"/g)].map(match => match[1]);
  assert.equal(hashes.length, 16);
  assert.equal(new Set(hashes).size, 16);
  assert.doesNotMatch(setup, /Layered_ComfyUI|TripoSR|Animagine/);
  assert.match(setup, /pending\.replace\(destination\)/);
});
