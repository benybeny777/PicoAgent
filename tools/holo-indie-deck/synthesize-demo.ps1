# 審査用動画の固定応答を、起動済みAivisの公開音声モデルで合成する。
[CmdletBinding()]
param(
    [string]$Endpoint = 'http://127.0.0.1:11436',
    [int]$Speaker = 888753760
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$output = Join-Path $root 'temp/holo-indie-review-installer/aivis-holo-demo.wav'
$speech = 'にゃっはろ～！予定の整理も調べものも、いっしょに進めようにぇ。'
$encoded = [uri]::EscapeDataString($speech)
$query = Invoke-RestMethod -Method Post -Uri "$Endpoint/audio_query?text=$encoded&speaker=$Speaker"
$query.volumeScale = 0.72
$json = $query | ConvertTo-Json -Depth 30 -Compress
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $output) | Out-Null
Invoke-WebRequest -Method Post -Uri "$Endpoint/synthesis?speaker=$Speaker" -ContentType 'application/json' -Body $json -OutFile $output
Write-Host "撮影用Aivis音声を生成しました: $output"
