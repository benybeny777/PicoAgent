import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const source = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');

test('選択動画モデルの導入先と生成参照先が同じコンポーネント直下になる', () => {
  const installer = source('src-tauri/src/components.rs');
  const setup = source('character-maker/sidecar/media/setup_models.py');
  const tool = source('skills/comfyui/mod.rs');
  assert.match(installer, /"--runtime",\s*&stage\.to_string_lossy\(\)/);
  assert.match(installer, /replace_installed\(&stage, &installed_dir\(app, id\), id\)/);
  assert.match(setup, /root = args\.runtime\.resolve\(\)/);
  assert.match(setup, /destination = safe_path\(root, relative\)/);
  assert.match(tool, /_ => bundled\.join\(engine\.component\(\)\),/);
  assert.match(tool, /\.arg\("--models"\)\s*\.arg\(&models\)/);
});

test('動画とキャラGPU工程がcharacters直下の共通ロックを使う', () => {
  const tool = source('skills/comfyui/mod.rs');
  const app = source('src-tauri/src/character_maker.rs');
  const completion = source('character-maker/sidecar/completion/generate.py');
  const pipeline = source('character-maker/src/pipeline.rs');
  assert.match(app, /characters_root: data\.join\("character-maker\/characters"\)/);
  assert.match(tool, /character-maker\/characters\/temp\/completion-gpu\.lock/);
  assert.match(completion, /workspace = character\.parent\/'temp'/);
  assert.match(completion, /workspace\/'completion-gpu\.lock'/);
  assert.match(pipeline, /matches!\(stage, "isolate" \| "decompose"\)[\s\S]*?startup_cleanup_guard\(\s*&self\.characters_root,?\s*\)/);
});
