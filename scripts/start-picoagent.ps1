#Requires -Version 7
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repositoryRoot
$host.UI.RawUI.WindowTitle = 'PicoAgentを起動'

if ($null -eq (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host 'Rust（cargo）が見つかりません。' -ForegroundColor Red
    Write-Host 'https://rustup.rs/ から導入して、もう一度実行してください。'
    [void](Read-Host 'Enterを押して閉じる')
    exit 1
}

Write-Host 'PicoAgentを起動します。'
Write-Host '初回は必要なファイルの準備とビルドに時間がかかる場合があります。'
Write-Host 'PicoAgentを終了するまで、この画面を閉じないでください。'
Write-Host ''

& cargo xtask dev
$exitCode = $LASTEXITCODE
if ($exitCode -ne 0) {
    Write-Host ''
    Write-Host "PicoAgentを起動できませんでした（終了コード: $exitCode）。" -ForegroundColor Red
    Write-Host '詳細は上のメッセージを確認してください。'
    [void](Read-Host 'Enterを押して閉じる')
}
exit $exitCode
