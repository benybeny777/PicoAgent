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
  const [components, settings, style] = await Promise.all([
    read("src-tauri/src/components.rs"),
    read("ui/settings.js"),
    read("ui/settings.css"),
  ]);
  assert.match(components, /join\("python\/Scripts\/python\.exe"\)/);
  assert.match(components, /85_000_000_000/);
  assert.match(components, /nvidia-smi\.exe/);
  assert.match(components, /約77GB/);
  assert.doesNotMatch(components.toLowerCase(), /tha3/);
  assert.match(settings, /function createOperationProgress\(/);
  assert.match(settings, /operationProgress\.update\(progress\)/);
  assert.match(settings, /経過 \$\{minutes\}分/);
  assert.match(settings, /operation-progress-detail/);
  assert.match(settings, /\$\{percent\}% ・ \$\{formatSize\(downloaded\)\} \/ \$\{formatSize\(total\)\}/);
  assert.match(style, /operation-progress-track\.is-indeterminate/);
  assert.match(style, /operation-progress-detail/);
  assert.match(style, /@keyframes operation-progress-slide/);
});

test("むぎは現行2.5Dリグだけを配信し旧素材へフォールバックしない", async () => {
  const manifest = JSON.parse(await read("characters/mugi/character.json"));
  assert.equal(manifest.meshRig?.rig, "rig2d/rig.json");
  for (const key of ["boot", "animations", "frameGeneration", "runtimeLayers", "runtimeLayerEncoding", "frameEncoding"]) {
    assert.equal(manifest[key], undefined, key);
  }
});

test("採用モデルだけを固定SHAで管理領域へ取得する", async () => {
  const setup = await read("character-maker/installer/setup-runtime-models.py");
  const hashes = [...setup.matchAll(/"([0-9a-f]{64})"/g)].map(match => match[1]);
  assert.equal(hashes.length, 16);
  assert.equal(new Set(hashes).size, 16);
  assert.doesNotMatch(setup, /Layered_ComfyUI|TripoSR|Animagine/);
  assert.match(setup, /pending\.replace\(destination\)/);
});

test("完成キャラをアプリ内の本番描画経路で確認できる", async () => {
  const [backend, settings, bridge, preview, config, capability] = await Promise.all([
    read("src-tauri/src/character_maker.rs"),
    read("ui/settings.js"),
    read("ui/picoagent.js"),
    read("ui/character-preview.js"),
    read("src-tauri/tauri.conf.json"),
    read("src-tauri/capabilities/default.json"),
  ]);
  assert.match(settings, /textContent: "動作を確認"/);
  assert.match(bridge, /characterMakerPreviewData/);
  assert.match(backend, /Reader::acquire/);
  assert.match(backend, /WindowEvent::Destroyed/);
  assert.match(preview, /window\.PicoMeshRig\.create/);
  for (const control of ["blink", "gazeX", "gazeY", "faceDemo", "motionDemo", "arm"]) {
    assert.match(preview, new RegExp(`getElementById\\(\"${control}\"\\)`));
  }
  assert.match(config, /\$APPDATA\/character-maker\/characters\/\*\*/);
  assert.match(capability, /character-preview-\*/);
});

test("起動時は原画と現行公開世代だけを残して生成キャッシュを回収する", async () => {
  const [backend, references, startup] = await Promise.all([
    read("src-tauri/src/character_maker.rs"),
    read("character-maker/src/generation_reference.rs"),
    read("src-tauri/src/lib.rs"),
  ]);
  assert.match(startup, /character_maker::cleanup_startup\(app\.handle\(\)\)/);
  for (const cache of ["source/isolated.png", "analysis", "layers", "rig2d-base", "completion-source", "temp"]) {
    assert.match(backend, new RegExp(`\"${cache.replace("/", "\\/")}\"`));
  }
  assert.doesNotMatch(backend, /remove_regenerable_path\(&directory\.join\("source\/input\.png"\)\)/);
  assert.match(references, /pub fn collect_startup/);
  assert.match(references, /selected\.previous\.take\(\)/);
  assert.match(backend, /status: "pending"/);
});
