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
