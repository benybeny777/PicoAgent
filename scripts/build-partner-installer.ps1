#Requires -Version 7
# 許諾済み関係者向けのMMD・音声同梱版インストーラを生成する（Windows専用）。
# 正規入口はリポジトリ直下の「関係者版インストーラをビルド.bat」。公開アルファ版は cargo xtask build-beta を使う。
[CmdletBinding()]
param(
    # 同梱する初期設定スクリプト（seed-voices.ps1）を置いたフォルダー。Git管理外。
    [string]$SourceDir = 'temp\settings-runtime',
    # 専用設定と生成物の保存先。Git管理外。
    [string]$OutputDir = 'temp\partner-installer',
    # origin/main より古い、または未コミット変更のあるチェックアウトでもビルドする。
    [switch]$AllowStale
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw '関係者版インストーラはWindowsでだけ生成できます。'
}

$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

# 同梱する許諾済み素材。キャラはPicoAgentの登録データ、音声はAivisのモデルフォルダーから取り込む。
$characters = @('miko-mmd', 'korone-mmd')
$voiceModels = @('8e120ad8-9ea4-451d-ab07-4a8e8dbb9956', '360c3bb8-1400-4a3a-ac00-4ad5b5444994')

function Invoke-Step {
    param([string]$Label, [string]$File, [string[]]$Arguments)
    Write-Host "==> $Label"
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Label に失敗しました（終了コード $LASTEXITCODE）。"
    }
}

# 1. 同梱素材がそろっているかをビルド前に確かめる。
$sourceRoot = if ([System.IO.Path]::IsPathRooted($SourceDir)) { $SourceDir } else { Join-Path $root $SourceDir }
$seedScript = Join-Path $sourceRoot 'seed-voices.ps1'
$characterRoot = Join-Path $env:APPDATA 'PicoAgent\characters'
$voiceRoot = Join-Path $env:APPDATA 'PicoAgent\aivis-userdata\profile\AppData\Roaming\AivisSpeech-Engine\Models'
$missing = [System.Collections.Generic.List[string]]::new()
if (-not (Test-Path -LiteralPath $seedScript -PathType Leaf)) {
    $missing.Add($seedScript)
}
foreach ($id in $characters) {
    $characterJson = Join-Path (Join-Path $characterRoot $id) 'character.json'
    if (-not (Test-Path -LiteralPath $characterJson -PathType Leaf)) {
        $missing.Add($characterJson)
    }
}
foreach ($uuid in $voiceModels) {
    $model = Join-Path $voiceRoot "$uuid.aivmx"
    if (-not (Test-Path -LiteralPath $model -PathType Leaf)) {
        $missing.Add($model)
    }
}
if ($missing.Count -gt 0) {
    throw "同梱する素材が見つかりません。用意してから再実行してください:`n  " + ($missing -join "`n  ")
}

# 2. 修正を取り込んだ最新の main から作る。
Invoke-Step 'origin/main を取得' 'git' @('fetch', '--quiet', 'origin', 'main')
$behind = [int](git rev-list --count 'HEAD..origin/main')
$dirty = @(git status --porcelain --untracked-files=no)
Write-Host "ビルド対象: $(git log --oneline -1)"
if (($behind -gt 0 -or $dirty.Count -gt 0) -and -not $AllowStale) {
    throw "チェックアウトが origin/main より $behind コミット古いか、未コミットの変更があります。git pull --ff-only で最新にしてから再実行してください（このまま作る場合は -AllowStale）。"
}
$version = (Get-Content -Raw -Encoding UTF8 -LiteralPath 'src-tauri\tauri.conf.json' | ConvertFrom-Json).version

# 3. 専用のNSIS hookとTauri設定を、このチェックアウトの絶対パスで生成する。
#    公開用の設定・許可キャラ一覧は変更しない。
$outputRoot = Join-Path $root $OutputDir
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$hookPath = Join-Path $outputRoot 'installer-partner.nsh'
$configPath = Join-Path $outputRoot 'tauri.partner.conf.json'
$hook = [System.Collections.Generic.List[string]]::new()
$hook.Add("!include `"$(Join-Path $root 'src-tauri\installer-hooks-lite.nsh')`"")
$hook.Add('')
$hook.Add('; 許諾済み関係者の試用向け。既存の登録データとモデルは上書きしない。')
$hook.Add('!macro NSIS_HOOK_POSTINSTALL')
$hook.Add('  SetOverwrite off')
foreach ($id in $characters) {
    $hook.Add("  SetOutPath `"`$APPDATA\PicoAgent\characters\$id`"")
    $hook.Add("  File /r `"$(Join-Path $characterRoot $id)\*`"")
}
$hook.Add('  SetOutPath "$APPDATA\PicoAgent\aivis-userdata\profile\AppData\Roaming\AivisSpeech-Engine\Models"')
foreach ($uuid in $voiceModels) {
    $hook.Add("  File `"$(Join-Path $voiceRoot "$uuid.aivmx")`"")
}
$hook.Add('  SetOverwrite on')
$hook.Add('  InitPluginsDir')
$hook.Add('  SetOutPath "$PLUGINSDIR"')
$hook.Add("  File `"$seedScript`"")
$hook.Add('  nsExec::ExecToLog `powershell -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\seed-voices.ps1" -ConfigPath "$APPDATA\PicoAgent\config.json"`')
$hook.Add('  Pop $0')
$hook.Add('  StrCmp $0 "0" +3')
$hook.Add('  DetailPrint "関係者デモの初期設定に失敗しました。インストールを再実行してください。"')
$hook.Add('  Abort')
$hook.Add('!macroend')
# NSISが日本語をUTF-8として読むよう、BOM付きで書く。
[System.IO.File]::WriteAllText($hookPath, ($hook -join "`r`n") + "`r`n", [System.Text.UTF8Encoding]::new($true))
$config = [ordered]@{
    bundle = [ordered]@{
        resources = @(
            'resources/bundled/engine/**/*',
            'resources/bundled/aivis/**/*',
            'resources/component-manifest.json'
        )
        windows   = [ordered]@{
            nsis = [ordered]@{ installerHooks = ($hookPath -replace '\\', '/') }
        }
    }
}
[System.IO.File]::WriteAllText($configPath, ($config | ConvertTo-Json -Depth 10), [System.Text.UTF8Encoding]::new($false))

# 4. 公開アルファ版と同じ前処理（配布対象キャラ・ALPHA表示・全機能試用feature）でビルドする。
$started = Get-Date
Invoke-Step '起動中のPicoAgentと同梱エンジンを停止' 'cargo' @('xtask', 'stop')
Invoke-Step 'Live2D・ローカルAIエンジン・音声エンジンを準備' 'cargo' @('xtask', 'setup', 'live2d', 'engine', 'aivis')
Invoke-Step '配布対象キャラクターを同期' 'cargo' @('xtask', 'preview-distribution')
Invoke-Step 'アルファ版の画面を生成' 'cargo' @('xtask', 'build-ui', '--release', '--beta')
Invoke-Step 'NSISインストーラを生成' 'cargo' @(
    'tauri', 'build', '--features', 'beta-full', '--bundles', 'nsis',
    '--config', 'src-tauri/tauri.distribution.conf.json',
    '--config', ($configPath -replace '\\', '/')
)

# 5. 今回のビルドで生成された実行ファイルだけを専用名で保存し、容量とSHA-256を示す。
$bundle = Join-Path $root "src-tauri\target\release\bundle\nsis\PicoAgent_${version}_x64-setup.exe"
$item = Get-Item -LiteralPath $bundle -ErrorAction SilentlyContinue
if ($null -eq $item -or $item.LastWriteTime -lt $started) {
    throw "今回のビルドで生成されたインストーラが見つかりません: $bundle"
}
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
$destination = Join-Path $outputRoot "PicoAgent_${version}_Partner-MMD-Voice_x64-setup.exe"
Move-Item -LiteralPath $bundle -Destination $destination -Force
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash
$sizeMb = [math]::Round((Get-Item -LiteralPath $destination).Length / 1e6)
Write-Host ''
Write-Host "完了: $destination"
Write-Host "版: $version / 容量: 約 $sizeMb MB / SHA-256: $hash"
