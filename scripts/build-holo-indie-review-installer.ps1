#Requires -Version 7
# holo Indie審査へ限定共有する、公式MMD 1体・Aivisの仮音声によるWindows版を生成する。
# 正規入口はリポジトリ直下の「holo Indie審査版インストーラをビルド.bat」。
[CmdletBinding()]
param(
    # 専用設定と生成物の保存先。Git管理外。
    [string]$OutputDir = 'temp\holo-indie-review-installer',
    # origin/mainより古い、または未コミット変更のあるチェックアウトでもビルドする。
    [switch]$AllowStale,
    # 同梱範囲と生成設定だけを検査し、重いビルドは行わない。
    [switch]$PrepareOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw 'holo Indie審査版インストーラはWindowsでだけ生成できます。'
}

$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

# 初回審査は出所を確認できたさくらみこ公式MMDだけを使う。
# 本人音声モデルは同梱せず、Aivis公式の既定モデルを初回起動時に取得する。
$characterId = 'miko-mmd'
$reviewIdentifier = 'com.picoagent.holo-indie-review'
$reviewDataRoot = "PicoAgent\instances\$reviewIdentifier"
$reviewDataScope = $reviewDataRoot -replace '\\', '/'
$characterRoot = Join-Path $env:APPDATA 'PicoAgent\characters'
$characterSource = Join-Path $characterRoot $characterId
$characterJson = Join-Path $characterSource 'character.json'
$licenseReadme = Join-Path $characterSource 'mmd\readme.txt'
$missing = @(
    @($characterJson, $licenseReadme) | Where-Object {
        -not (Test-Path -LiteralPath $_ -PathType Leaf)
    }
)
if ($missing.Count -gt 0) {
    throw "審査版に必要な公式MMDまたは利用条件が見つかりません:`n  " + ($missing -join "`n  ")
}

function Invoke-Step {
    param([string]$Label, [string]$File, [string[]]$Arguments)
    Write-Host "==> $Label"
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Label に失敗しました（終了コード $LASTEXITCODE）。"
    }
}

Invoke-Step 'origin/mainを取得' 'git' @('fetch', '--quiet', 'origin', 'main')
$behind = [int](git rev-list --count 'HEAD..origin/main')
$dirty = @(git status --porcelain --untracked-files=no)
Write-Host "ビルド対象: $(git log --oneline -1)"
if (($behind -gt 0 -or $dirty.Count -gt 0) -and -not $AllowStale) {
    throw "チェックアウトがorigin/mainより $behind コミット古いか、未コミットの変更があります。最新化してから再実行してください（このまま作る場合は -AllowStale）。"
}
$version = (Get-Content -Raw -Encoding UTF8 -LiteralPath 'src-tauri\tauri.conf.json' | ConvertFrom-Json).version

$outputRoot = Join-Path $root $OutputDir
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$hookPath = Join-Path $outputRoot 'installer-holo-indie-review.nsh'
$configPath = Join-Path $outputRoot 'tauri.holo-indie-review.conf.json'
$initialConfigPath = Join-Path $outputRoot 'config.holo-indie-review.json'

$initialConfig = [ordered]@{
    PICOAGENT_CHARACTER       = $characterId
    PICOAGENT_LAYOUT          = 'stream'
    PICOAGENT_PROVIDER        = 'local'
    PICOAGENT_TTS_ENABLED     = $true
    PICOAGENT_ONBOARDING_DONE = $true
    PICOAGENT_TTS_CHARACTER_OVERRIDES = @{
        'miko-mmd' = @{
            speaker = '888753760'
            modelUuid = 'a59cb814-0083-4369-8542-f51a29e72af7'
            speakerUuid = 'e756b8e4-b606-4e15-99b1-3f9c6a1b2317'
            styleName = 'ノーマル'
        }
    }
    PICOAGENT_CHATTER_ENABLED = $false
}
[System.IO.File]::WriteAllText(
    $initialConfigPath,
    ($initialConfig | ConvertTo-Json -Depth 5),
    [System.Text.UTF8Encoding]::new($false)
)

$hook = [System.Collections.Generic.List[string]]::new()
$hook.Add("!include `"$(Join-Path $root 'src-tauri\installer-hooks-lite.nsh')`"")
$hook.Add('')
$hook.Add('; holo Indie審査への限定共有用。通常版とは別の利用者領域へ配置する。')
$hook.Add('!macro NSIS_HOOK_POSTINSTALL')
$hook.Add('  SetOverwrite off')
$hook.Add("  SetOutPath `"`$APPDATA\$reviewDataRoot\characters\$characterId`"")
$hook.Add("  File /r `"$characterSource\*`"")
$hook.Add("  SetOutPath `"`$APPDATA\$reviewDataRoot`"")
$hook.Add("  File /oname=config.json `"$initialConfigPath`"")
$hook.Add('  SetOverwrite on')
$hook.Add('!macroend')
[System.IO.File]::WriteAllText($hookPath, ($hook -join "`r`n") + "`r`n", [System.Text.UTF8Encoding]::new($true))

$config = [ordered]@{
    productName = 'PicoAgent holo Indie 審査版'
    identifier  = $reviewIdentifier
    app         = [ordered]@{
        security = [ordered]@{
            assetProtocol = [ordered]@{
                enable = $true
                scope = @(
                    "`$DATA/$reviewDataScope/characters/**"
                    "`$DATA/$reviewDataScope/display/**"
                    "`$DATA/$reviewDataScope/character-maker/characters/**"
                    "`$DATA/$reviewDataScope/generated-media/**"
                )
            }
        }
    }
    bundle      = [ordered]@{
        resources = @(
            'resources/bundled/engine/**/*',
            'resources/bundled/aivis/**/*',
            'resources/component-manifest.json'
        )
        windows   = [ordered]@{
            nsis = [ordered]@{
                languages = @('Japanese')
                installerHooks = ($hookPath -replace '\\', '/')
            }
        }
    }
}
[System.IO.File]::WriteAllText($configPath, ($config | ConvertTo-Json -Depth 10), [System.Text.UTF8Encoding]::new($false))

$forbidden = @('.aivmx', 'seed-voices')
$generated = (Get-Content -Raw -Encoding UTF8 -LiteralPath $hookPath) + (Get-Content -Raw -Encoding UTF8 -LiteralPath $configPath)
foreach ($word in $forbidden) {
    if ($generated -match [regex]::Escape($word)) {
        throw "審査版の生成設定へ音声資産を示す文字列が混入しています: $word"
    }
}
if ($generated -match 'SetOutPath\s+"\$APPDATA\\com\.picoagent\.desktop(?:\\|\")') {
    throw '審査版が通常版のキャラクター領域を上書きする設定になっています。'
}
if ($initialConfig.PICOAGENT_TTS_ENABLED -ne $true -or $generated -notmatch 'resources/bundled/aivis') {
    throw '審査版のAivis読み上げ設定が揃っていません。'
}
if ($generated -notmatch [regex]::Escape("`$DATA/$reviewDataScope/characters/**")) {
    throw '審査版のキャラクター読み取り範囲が専用領域へ設定されていません。'
}
if (-not (Test-Path -LiteralPath 'src-tauri/resources/bundled/aivis/run.exe' -PathType Leaf)) {
    throw 'Aivis本体が見つかりません。cargo xtask setup aivis を実行してください。'
}
$bundledVoiceModels = @(Get-ChildItem -LiteralPath 'src-tauri/resources/bundled/aivis' -Recurse -File -Filter '*.aivmx')
if ($bundledVoiceModels.Count -gt 0) {
    throw '審査版のAivis本体に本人音声モデルが混入しています。'
}

if ($PrepareOnly) {
    Write-Host ''
    Write-Host "準備検査完了: $outputRoot"
    Write-Host '公式MMD: さくらみこ1体 / Aivis: 同梱 / 本人音声モデル: なし / 読み上げ: 既定ON / 通常版と利用者領域を分離'
    exit 0
}

$started = Get-Date
Invoke-Step '起動中のPicoAgentと同梱エンジンを停止' 'cargo' @('xtask', 'stop')
Invoke-Step 'Live2DとローカルAI・Aivisエンジンを準備' 'cargo' @('xtask', 'setup', 'live2d', 'engine', 'aivis')
Invoke-Step '配布対象キャラクターを同期' 'cargo' @('xtask', 'preview-distribution')
Invoke-Step 'アルファ版の画面を生成' 'cargo' @('xtask', 'build-ui', '--release', '--beta')
Invoke-Step 'NSISインストーラを生成' 'cargo' @(
    'tauri', 'build', '--features', 'beta-full', '--bundles', 'nsis',
    '--config', 'src-tauri/tauri.distribution.conf.json',
    '--config', ($configPath -replace '\\', '/')
)

$bundleRoot = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$items = @(
    Get-ChildItem -LiteralPath $bundleRoot -Filter '*_x64-setup.exe' -File -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -ge $started }
)
if ($items.Count -ne 1) {
    throw "今回のビルドで生成されたインストーラを1件に特定できません: $bundleRoot"
}
$bundle = $items[0].FullName
$item = $items[0]
$magic = [byte[]]::new(2)
$stream = [System.IO.File]::OpenRead($bundle)
try {
    [void]$stream.Read($magic, 0, 2)
}
finally {
    $stream.Dispose()
}
if ($magic[0] -ne 0x4D -or $magic[1] -ne 0x5A) {
    throw "生成物がWindows実行ファイルではありません: $bundle"
}
$destination = Join-Path $outputRoot "PicoAgent_${version}_holo-Indie-Review_x64-setup.exe"
Move-Item -LiteralPath $bundle -Destination $destination -Force
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash
$sizeMb = [math]::Round((Get-Item -LiteralPath $destination).Length / 1e6)
Write-Host ''
Write-Host "完了: $destination"
Write-Host "版: $version / 容量: 約 $sizeMb MB / SHA-256: $hash"
Write-Host 'この成果物はholo Indie審査への限定共有専用です。公開配布しないでください。'
