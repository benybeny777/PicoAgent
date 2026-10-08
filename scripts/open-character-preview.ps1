#Requires -Version 7
[CmdletBinding()]
param(
    [ValidateSet('menu', '2d', 'live2d', 'miko-mmd', 'korone-mmd')]
    [string]$Mode = 'menu'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repositoryRoot

if ($Mode -eq 'menu') {
    Write-Host '確認するキャラクターを選んでください。'
    Write-Host '  1: 独自2.5D（全キャラ切替）'
    Write-Host '  2: MMD さくらみこ'
    Write-Host '  3: MMD 戌神ころね'
    Write-Host '  4: Live2D（登録済みキャラから選択）'
    $selection = Read-Host '番号'
    $Mode = switch ($selection) {
        '1' { '2d' }
        '2' { 'miko-mmd' }
        '3' { 'korone-mmd' }
        '4' { 'live2d' }
        default { throw '1、2、3、4のいずれかを入力してください。' }
    }
}

if ($Mode -eq '2d') {
    if (-not (Test-Path -LiteralPath 'ui\vendor\pixi\pixi.min.js' -PathType Leaf)) {
        throw 'PIXIが未準備です。先に cargo xtask setup pixi を実行してください。'
    }
    $uv = Get-Command uv -ErrorAction SilentlyContinue
    if ($null -eq $uv) {
        throw 'uvが見つかりません。'
    }

    $server = Start-Process -FilePath $uv.Source `
        -ArgumentList @('run', '--python', '3.12', '--offline', 'python', '-m', 'http.server', '8776', '--bind', '127.0.0.1', '--directory', $repositoryRoot) `
        -WorkingDirectory $repositoryRoot -WindowStyle Hidden -PassThru
    try {
        $ready = $false
        for ($attempt = 0; $attempt -lt 50; $attempt++) {
            if ($server.HasExited) {
                throw "確認用サーバーが起動直後に終了しました（終了コード $($server.ExitCode)）。"
            }
            try {
                $client = [Net.Sockets.TcpClient]::new()
                $client.Connect('127.0.0.1', 8776)
                $client.Dispose()
                $ready = $true
                break
            }
            catch {
                Start-Sleep -Milliseconds 100
            }
        }
        if (-not $ready) {
            throw '確認用サーバーが5秒以内に応答しませんでした。'
        }
        Start-Process 'http://127.0.0.1:8776/tools/mesh-rig-preview/'
        Write-Host '独自2.5Dの確認画面を開きました。'
        Write-Host '確認が終わったら、この画面でEnterを押してサーバーを終了してください。'
        [void](Read-Host)
    }
    finally {
        if (-not $server.HasExited) {
            & taskkill.exe /PID $server.Id /T /F *> $null
        }
        $server.Dispose()
    }
    exit 0
}

# 開発起動は通常版（%APPDATA%\PicoAgent）ではなく、開発用の保存先を使う。
$devCharacterRoot = Join-Path $env:APPDATA 'PicoAgent-Dev\characters'

if ($Mode -eq 'live2d') {
    $live2dCharacters = [System.Collections.Generic.List[object]]::new()
    $seen = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($root in @(
        (Join-Path $repositoryRoot 'characters'),
        $devCharacterRoot
    )) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) { continue }
        foreach ($directory in Get-ChildItem -LiteralPath $root -Directory) {
            $manifestPath = Join-Path $directory.FullName 'character.json'
            if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { continue }
            try {
                $manifest = Get-Content -Raw -Encoding UTF8 -LiteralPath $manifestPath | ConvertFrom-Json
                $live2dProperty = $manifest.PSObject.Properties['live2d']
                if ($null -ne $live2dProperty -and $null -ne $live2dProperty.Value -and
                    $seen.Add([string]$manifest.id)) {
                    $live2dCharacters.Add([pscustomobject]@{
                        Id = [string]$manifest.id
                        Name = [string]$manifest.name
                    })
                }
            }
            catch {
                Write-Warning "読めないキャラクター設定を除外しました: $manifestPath"
            }
        }
    }
    if ($live2dCharacters.Count -eq 0) {
        throw '確認できるLive2Dキャラクターが見つかりません。'
    }
    Write-Host '確認するLive2Dキャラクターを選んでください。'
    for ($index = 0; $index -lt $live2dCharacters.Count; $index++) {
        Write-Host "  $($index + 1): $($live2dCharacters[$index].Name)（$($live2dCharacters[$index].Id)）"
    }
    $selected = 0
    if (-not [int]::TryParse((Read-Host '番号'), [ref]$selected) -or
        $selected -lt 1 -or $selected -gt $live2dCharacters.Count) {
        throw "1から$($live2dCharacters.Count)の番号を入力してください。"
    }
    $Mode = $live2dCharacters[$selected - 1].Id
}

if ($Mode -in @('miko-mmd', 'korone-mmd')) {
    $characterPath = Join-Path $devCharacterRoot "$Mode\character.json"
    if (-not (Test-Path -LiteralPath $characterPath -PathType Leaf)) {
        throw "開発版に登録済みのMMDキャラクターが見つかりません: $characterPath（開発版は通常版と保存先が別のため、開発版の設定画面から追加してください）"
    }
}
if ($null -eq (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Rust（cargo）が見つかりません。'
}

$env:PICO_PREVIEW_CHARACTER = $Mode
Write-Host "保存中のキャラ選択を変えず、$Mode を実アプリで確認します。"
Write-Host 'PicoAgentを終了するまで、この画面を閉じないでください。'
& cargo xtask dev
exit $LASTEXITCODE
