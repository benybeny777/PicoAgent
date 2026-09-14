<#
.SYNOPSIS
  選別済みデータセット1件をStyle-Bert-VITS2 JP-Extraで前処理・学習し、ONNXとAIVMXへ書き出す。

.DESCRIPTION
  入力は scripts/select-voice-training-clips.py が作った Data/<Dataset>（raw/ と esd.list）。
  SBV2の共有設定 config.yml の model_name / dataset_path を毎回このデータセットへ合わせてから実行する。
  合わせないまま別キャラを続けて扱うと、train_ms_jp_extra.py が別キャラの Data/<ID>/config.json と
  model_assets/<ID>/style_vectors.npy を上書きする（2026-09-12の旧みこ・ころねモデルで発生）。
  既存の model_assets や導入済みAIVMXは上書きしない。長時間の学習は呼び出し側で終了コードとログを追跡する。

.EXAMPLE
  pwsh -File scripts/train-voice-model.ps1 -SbvRoot temp/voice-models/Style-Bert-VITS2 -Dataset sakura-miko-clean-v2 -Stage preprocess -BatchSize 4 -Epochs 200 -SaveEverySteps 1000
  pwsh -File scripts/train-voice-model.ps1 -SbvRoot temp/voice-models/Style-Bert-VITS2 -Dataset sakura-miko-clean-v2 -Stage train
  pwsh -File scripts/train-voice-model.ps1 -SbvRoot temp/voice-models/Style-Bert-VITS2 -Dataset sakura-miko-clean-v2 -Stage export -Checkpoint model_assets/sakura-miko-clean-v2/sakura-miko-clean-v2_e100_s6000.safetensors -OutputDir temp/voice-models/release/sakura-miko-clean-v2
#>
param(
    [Parameter(Mandatory = $true)][string]$SbvRoot,
    [Parameter(Mandatory = $true)][string]$Dataset,
    [Parameter(Mandatory = $true)][ValidateSet('preprocess', 'train', 'export')][string]$Stage,
    [int]$BatchSize = 4,
    [int]$Epochs = 200,
    [int]$SaveEverySteps = 1000,
    [string]$Checkpoint,
    [string]$OutputDir
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$root = (Resolve-Path -LiteralPath $SbvRoot).Path
$python = Join-Path $root '.venv\Scripts\python.exe'
$datasetDir = Join-Path $root "Data\$Dataset"
$configPath = Join-Path $datasetDir 'config.json'
if (-not (Test-Path -LiteralPath $python)) { throw "SBV2のPythonがありません: $python" }
if (-not (Test-Path -LiteralPath (Join-Path $datasetDir 'esd.list'))) { throw "選別済みデータセットがありません: $datasetDir" }

function Invoke-Sbv2 {
    param([string]$Label, [string[]]$Arguments)
    Write-Output "== $Label"
    Push-Location $root
    try {
        & $python @Arguments
        if ($LASTEXITCODE -ne 0) { throw "$Label が失敗しました（終了コード $LASTEXITCODE）" }
    } finally {
        Pop-Location
    }
}

# 共有設定 config.yml をこのデータセットへ合わせる（学習・スタイル生成の保存先がここで決まる）。
$selectConfig = @'
import sys, yaml
from pathlib import Path
root, dataset = Path(sys.argv[1]), sys.argv[2]
path = root / "config.yml"
data = yaml.safe_load(path.read_text(encoding="utf-8"))
data["model_name"] = dataset
data["dataset_path"] = str(Path("Data") / dataset)
data["train_ms"]["keep_ckpts"] = 1
path.write_text(yaml.dump(data, allow_unicode=True), encoding="utf-8")
'@
Invoke-Sbv2 'config.ymlの対象切替' @('-c', $selectConfig, $root, $Dataset)

switch ($Stage) {
    'preprocess' {
        $modelsDir = Join-Path $datasetDir 'models'
        if (Get-ChildItem -LiteralPath $modelsDir -Filter 'G_*.pth' -ErrorAction SilentlyContinue) {
            throw "学習が始まっているため前処理を停止します（チェックポイントを上書きしない）: $modelsDir"
        }
        $makeConfig = @'
import json, shutil, sys
from pathlib import Path
root, dataset, batch, epochs, save = Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
config = json.loads((root / "configs/config_jp_extra.json").read_text(encoding="utf-8"))
config["model_name"] = dataset
config["data"]["training_files"] = str(Path("Data") / dataset / "train.list")
config["data"]["validation_files"] = str(Path("Data") / dataset / "val.list")
config["data"]["use_jp_extra"] = True
config["train"].update({
    "batch_size": batch, "epochs": epochs, "eval_interval": save, "log_interval": 200, "bf16_run": False,
    "freeze_ZH_bert": True, "freeze_JP_bert": False, "freeze_EN_bert": True, "freeze_style": False, "freeze_decoder": False,
})
dataset_dir = root / "Data" / dataset
(dataset_dir / "config.json").write_text(json.dumps(config, indent=2, ensure_ascii=False), encoding="utf-8")
# 途中で失敗した前処理の再実行では、配置済みの事前学習モデルをそのまま使う。
if not (dataset_dir / "models" / "G_0.safetensors").exists():
    shutil.copytree(root / "pretrained_jp_extra", dataset_dir / "models", ignore=shutil.ignore_patterns(".cache"), dirs_exist_ok=True)
'@
        Invoke-Sbv2 '学習設定と事前学習モデルの配置' @('-c', $makeConfig, $root, $Dataset, $BatchSize, $Epochs, $SaveEverySteps)
        # 選別スクリプトで正規化済みのため、--normalize と --trim は付けない（pyloudnorm正規化でクリップするため）。
        Invoke-Sbv2 'リサンプル' @('resample.py', '-i', "Data\$Dataset\raw", '-o', "Data\$Dataset\wavs", '--sr', '44100', '--num_processes', '4')
        Invoke-Sbv2 '文章前処理' @('preprocess_text.py', '--config-path', "Data\$Dataset\config.json", '--transcription-path', "Data\$Dataset\esd.list", '--train-path', "Data\$Dataset\train.list", '--val-path', "Data\$Dataset\val.list", '--val-per-lang', '5', '--yomi_error', 'raise', '--correct_path', '--use_jp_extra')
        Invoke-Sbv2 'BERT特徴量' @('bert_gen.py', '-c', "Data\$Dataset\config.json")
        Invoke-Sbv2 'スタイル特徴量' @('style_gen.py', '-c', "Data\$Dataset\config.json", '--num_processes', '4')
    }
    'train' {
        if (-not (Test-Path -LiteralPath $configPath)) { throw "先に -Stage preprocess を実行してください: $configPath" }
        # 明示した学習量だけを反映する。VRAMが専用領域からあふれると共有メモリ経由で極端に遅くなるため、
        # その場合は品質設定を変えず -BatchSize だけを下げて再開する。
        $overrides = @{}
        foreach ($name in @('BatchSize', 'Epochs', 'SaveEverySteps')) {
            if ($PSBoundParameters.ContainsKey($name)) { $overrides[$name] = $PSBoundParameters[$name] }
        }
        if ($overrides.Count -gt 0) {
            $updateConfig = @'
import json, sys
path, values = sys.argv[1], json.loads(sys.argv[2])
config = json.loads(open(path, encoding="utf-8").read())
keys = {"BatchSize": "batch_size", "Epochs": "epochs", "SaveEverySteps": "eval_interval"}
for name, value in values.items():
    config["train"][keys[name]] = int(value)
open(path, "w", encoding="utf-8").write(json.dumps(config, indent=2, ensure_ascii=False))
print("学習設定を更新しました:", {keys[k]: v for k, v in values.items()})
'@
            Invoke-Sbv2 '学習設定の更新' @('-c', $updateConfig, $configPath, ($overrides | ConvertTo-Json -Compress))
        }
        Invoke-Sbv2 '学習' @('train_ms_jp_extra.py', '-c', "Data\$Dataset\config.json", '-m', "Data\$Dataset", '--no_progress_bar')
    }
    'export' {
        if (-not $Checkpoint -or -not $OutputDir) { throw '-Stage export には -Checkpoint と -OutputDir が必要です' }
        $checkpointPath = (Resolve-Path -LiteralPath (Join-Path $root $Checkpoint) -ErrorAction SilentlyContinue).Path
        if (-not $checkpointPath) { $checkpointPath = (Resolve-Path -LiteralPath $Checkpoint).Path }
        $assetDir = Split-Path -Parent $checkpointPath
        $output = New-Item -ItemType Directory -Force -Path $OutputDir
        $aivmx = Join-Path $output.FullName "$Dataset.aivmx"
        if (Test-Path -LiteralPath $aivmx) { throw "出力先に既にAIVMXがあるため停止します（上書きしない）: $aivmx" }
        $onnx = [IO.Path]::ChangeExtension($checkpointPath, '.onnx')
        if (-not (Test-Path -LiteralPath $onnx)) {
            Invoke-Sbv2 'ONNX変換' @('convert_onnx.py', '--model', $checkpointPath)
        }
        foreach ($file in @($checkpointPath, $onnx, (Join-Path $assetDir 'config.json'), (Join-Path $assetDir 'style_vectors.npy'))) {
            Copy-Item -LiteralPath $file -Destination $output.FullName
        }
        $aivmlib = Join-Path $root '.venv\Scripts\aivmlib.exe'
        & $aivmlib create-aivmx -o $aivmx -m $onnx -h (Join-Path $assetDir 'config.json') -s (Join-Path $assetDir 'style_vectors.npy') -a 'Style-Bert-VITS2 (JP-Extra)'
        if ($LASTEXITCODE -ne 0) { throw "AIVMX作成が失敗しました（終了コード $LASTEXITCODE）" }
        # AIVMX内のハイパーパラメータとスタイルベクトルが書き出し元と一致することを確認する。
        $verify = @'
import json, sys, aivmlib, numpy as np
from pathlib import Path
aivmx, asset_dir = Path(sys.argv[1]), Path(sys.argv[2])
with aivmx.open("rb") as f:
    meta = aivmlib.read_aivmx_metadata(f)
config = json.loads((asset_dir / "config.json").read_text(encoding="utf-8"))
if meta.hyper_parameters.model_name != config["model_name"]:
    raise SystemExit(f"model_nameが一致しません: {meta.hyper_parameters.model_name} != {config['model_name']}")
style = np.load(asset_dir / "style_vectors.npy")
embedded = np.frombuffer(meta.style_vectors, dtype=np.float32) if isinstance(meta.style_vectors, (bytes, bytearray)) else None
if embedded is not None and embedded.size != style.size:
    raise SystemExit("スタイルベクトルの要素数が一致しません")
print(json.dumps({"model_uuid": str(meta.manifest.uuid), "name": meta.manifest.name,
                  "speakers": [{"uuid": str(s.uuid), "name": s.name, "styles": [st.name for st in s.styles]} for s in meta.manifest.speakers]},
                 ensure_ascii=False))
'@
        Invoke-Sbv2 'AIVMX整合確認' @('-c', $verify, $aivmx, $assetDir)
    }
}
