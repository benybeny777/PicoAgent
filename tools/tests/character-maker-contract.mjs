import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const read = path => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("キャラ選択の決定ボタンはピンクで選択中と区別し、文字を読める濃さにする", async () => {
  const style = await read("ui/settings.css");
  // 淡い --pink に白文字（2.4:1）、--pink-soft に --pink の文字（2.0:1）はどちらも
  // WCAG AAの4.5:1に届かない。ピンクのまま読める濃さの専用トークンを使う。
  assert.match(style, /\.character-card \.character-select-button \{[\s\S]*?background: var\(--pink-strong\);[\s\S]*?color: #fff;/);
  assert.match(style, /\.character-card \.character-select-button:disabled \{[\s\S]*?background: var\(--pink-soft\);[\s\S]*?color: var\(--pink-on-soft\);/);
  assert.match(style, /--pink-strong: #b4477f;/);
  assert.match(style, /--pink-on-soft: #e98ab6;/);
});

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
  // 共通Python環境は1つのまま、実行ファイル名だけをOS別に解決する。
  assert.match(components, /python_executable\(&shared_python_dir\(app\)\)/);
  assert.match(components, /python_root\.join\("Scripts\/python\.exe"\)/);
  assert.match(components, /python_root\.join\("bin\/python3"\)/);
  const production = components.split("#[cfg(test)]")[0];
  const prepare = production.split("async fn prepare_shared_python(")[1].split("async fn verify_shared_python(")[0];
  const maker = production.split("async fn install_character_maker(")[1].split("async fn install_modern_media_runtime(")[0];
  assert.match(prepare, /install_base_python\(uv, &stage\.join\("runtime"\), label\)\.await\?/);
  assert.match(prepare, /create_venv\(uv, &base, stage, label\)\.await\?/);
  assert.match(prepare, /normalize_python_links\(stage\)\?/);
  assert.match(prepare, /verify_shared_python\(&python, label\)\.await\?/);
  assert.doesNotMatch(prepare, /copy_dir_all|python-venv/);
  assert.match(maker, /prepare_shared_python\(&uv, &python, "キャラ作成の共通Python導入"\)\.await\?/);
  const ordered = ["generation_selection().await", "free_disk_bytes(&bundled)? >= 85_000_000_000", "acquire_uv(app", "prepare_shared_python(", "verify_generation_python(&staged_python", "let comfy_zip", "run_component_command_with_progress(", "replace_installed_pair("];
  const positions = ordered.map(token => maker.indexOf(token));
  assert.ok(positions.every((position, index) => position >= 0 && (index === 0 || position > positions[index - 1])), "GPU・容量・Python検証を取得と配置に先行させる");
  assert.match(production, /swap::replace_with_validation\([\s\S]*?finalize_shared_python\(uv, &root/);
  assert.match(maker, /finalize_shared_python\(\s*&uv,\s*&shared_python_dir\(app\)/);
  const finalizer = production.split("async fn finalize_python_using(")[1].split("async fn finalize_shared_python(")[0];
  assert.ok(finalizer.indexOf("configure.await?") < finalizer.indexOf("normalize_python_links(root)?"));
  assert.ok(finalizer.indexOf("normalize_python_links(root)?") < finalizer.indexOf("verify.await"));
  assert.match(components, /"--relocatable"/);
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
  // 寝姿は居眠り表示専用。起動準備中は窓ごと隠すので起動用の印は持たない。
  assert.equal(manifest.boot?.sleeping, "boot/sleeping.png");
  assert.equal(manifest.boot?.useSleepingOnStartup, undefined);
  for (const key of ["animations", "frameGeneration", "runtimeLayers", "runtimeLayerEncoding", "frameEncoding"]) {
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
  assert.match(config, /\$DATA\/PicoAgent\/character-maker\/characters\/\*\*/);
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
