import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

const preview = read("scripts/open-character-preview.ps1");
const checker = read("scripts/check-synthesized-voice.py");
const selector = read("scripts/select-voice-training-clips.py");
const cleaner = read("scripts/clean-voice-training-pauses.py");
const enhancer = read("scripts/enhance-voice-training-chunks.py");
const trainer = read("scripts/train-voice-model.ps1");
const common = read("scripts/voice_training_common.py");

test("Live2D列挙は任意プロパティをStrictModeでも安全に読む", () => {
  assert.match(preview, /PSObject\.Properties\['live2d'\]/);
  assert.doesNotMatch(preview, /\$manifest\.live2d/);
});

test("合成QAは空の話者基準と既存結果の上書きを拒否する", () => {
  assert.match(checker, /話者基準に使える採用断片がありません/);
  assert.match(checker, /話者基準の埋め込みがありません/);
  assert.match(checker, /検査結果を上書きしない/);
  assert.match(checker, /result\["wav"\] = str\(out_dir/);
  assert.match(checker, /incoming\.replace\(out_dir\)/);
});

test("学習断片の測定と選別はWAVの同一性と一意名を検証する", () => {
  assert.match(selector, /source_sha256/);
  assert.match(selector, /file_sha256\(source_wav\)/);
  assert.match(selector, /複数データセットの採用断片に同名WAVがあります/);
  assert.match(selector, /output_root\.with_name\(f"\{output_root\.name\}\.incoming"\)/);
});

test("学習WAVは管理下の相対パスだけを許可する", () => {
  assert.match(common, /path\.is_absolute\(\) or path\.drive or path\.root/);
  assert.match(common, /part in \("", "\.", "\.\."\)/);
  assert.match(common, /path\.suffix\.casefold\(\) != "\.wav"/);
  assert.match(common, /candidate\.relative_to\(resolved_root\)/);
  assert.match(common, /part\.casefold\(\)/);
});

test("データセット名はData直下の単一フォルダーだけを許可する", () => {
  assert.match(common, /len\(path\.parts\) != 1/);
  assert.match(common, /WINDOWS_RESERVED_NAMES/);
  assert.match(selector, /datasets = \[dataset_name\(dataset\) for dataset in args\.dataset\]/);
  assert.match(trainer, /\[IO\.Path\]::IsPathRooted\(\$Dataset\)/);
  assert.match(trainer, /データセット名はData直下の安全なフォルダー名/);
});

test("ノイズ補正版は元データとサンプルレートを照合してから確定する", () => {
  assert.match(enhancer, /manifest\.get\("source"\) != source\.name/);
  assert.match(enhancer, /"source_sha256": file_sha256\(source_wav\)/);
  assert.match(enhancer, /file_sha256\(source_wav\) != expected_hash/);
  assert.match(enhancer, /sr != expected_sample_rate/);
  assert.match(enhancer, /incoming\.replace\(output\)/);
});

test("合間が0件の統計を閾値超過として記録しない", () => {
  assert.match(cleaner, /"count": 0, "median_db": None, "louder_than_-48db": None/);
  assert.match(enhancer, /"pause_median_db": .* if levels else None/);
  assert.match(enhancer, /"pauses_louder_than_-48db": .* if levels else None/);
});

test("AIVMX書き出しは採用チェックポイントからONNXを作り直す", () => {
  assert.match(trainer, /保存済みチェックポイントが停止目標/);
  assert.match(trainer, /\$checkpointCopy = Join-Path/);
  assert.match(trainer, /'convert_onnx\.py', '--model', \$checkpointCopy/);
  assert.doesNotMatch(trainer, /if \(-not \(Test-Path -LiteralPath \$onnx\)\)/);
  assert.match(trainer, /Move-Item -LiteralPath \$output\.FullName -Destination \$outputPath/);
});
