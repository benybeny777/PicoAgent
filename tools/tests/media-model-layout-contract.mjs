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
  // 生成参照先は`--models`でサイドカーへ渡す（省メモリ再試行でも同じ引数列を作り直す）。
  assert.match(tool, /"--models"\.into\(\),\s*self\.models\.into\(\),/);
  assert.match(tool, /models: &models,/);
});

test('省メモリ再試行で生成条件を変えない', () => {
  const tool = source('skills/comfyui/mod.rs');
  const pipeline = source('character-maker/src/pipeline.rs');
  const media = source('character-maker/sidecar/media/generate.py');
  const completion = source('character-maker/sidecar/completion/generate.py');
  // 追加されるのは`--lowvram`だけで、seed・入力・出力先は同じ組み立てを再利用する。
  assert.match(tool, /if lowvram \{\s*arguments\.push\("--lowvram"\.into\(\)\);/);
  assert.match(pipeline, /fn with_lowvram\(arguments: &\[OsString\], lowvram: bool\)/);
  // 再試行はGPUメモリ不足の終了コードだけを見て、1回だけ行う。
  assert.match(tool, /should_retry_with_lowvram\(result\.status\.code\(\), lowvram\)/);
  assert.match(pipeline, /GPU_OOM_EXIT_CODE: i32 = 3;/);
  assert.match(media, /OOM_EXIT_CODE = 3/);
  assert.match(completion, /OOM_EXIT_CODE = 3/);
  // サイドカーは終了前にComfyUIと子孫プロセスを回収する。
  assert.match(media, /finally:\s*\n\s*#[^\n]*\n\s*stop_owned\(process\)/);
  assert.match(completion, /finally:\s*\n\s*stop_owned\(process\)/);
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
