#Requires -Version 7
# Windows各版の正規ビルド入口を直列実行し、新規Releaseだけへ保存する。
[CmdletBinding()]
param(
    [ValidateSet('All', 'Alpha', 'Partner')][string]$Kind = 'All',
    [switch]$PublicAlpha,
    [switch]$PrepareOnly,
    [ValidatePattern('^[0-9a-f]{40}$')][string]$PreviousSourceCommit,
    [string]$PartnerSourceDir = 'temp/settings-runtime'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Invoke-Checked {
    param([string]$File, [string[]]$Arguments)
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$File が失敗しました（終了コード $LASTEXITCODE）。後続の公開は停止します。" }
}
function Get-WindowsPlan {
    param([ValidateSet('All', 'Alpha', 'Partner')][string]$Selection, [string]$Version)
    if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw '数値3桁の版が必要です。' }
    $all = @(
        [pscustomobject]@{ Kind = 'Alpha'; File = "PicoAgent_${Version}_x64-setup.exe"; Source = "src-tauri/target/release/bundle/nsis/PicoAgent_${Version}_x64-setup.exe" },
        [pscustomobject]@{ Kind = 'Partner'; File = "PicoAgent_${Version}_Partner-MMD-Voice_x64-setup.exe"; Source = "temp/partner-installer/PicoAgent_${Version}_Partner-MMD-Voice_x64-setup.exe" }
    )
    @($all | Where-Object { $Selection -eq 'All' -or $_.Kind -eq $Selection })
}
function Assert-Installer {
    param([string]$Path, [datetime]$Started)
    $item = Get-Item -LiteralPath $Path
    if ($item.Length -lt 2 -or $item.LastWriteTimeUtc -lt $Started.ToUniversalTime()) { throw "今回生成した成果物ではありません: $Path" }
    $stream = [IO.File]::OpenRead($item.FullName)
    try { if ($stream.ReadByte() -ne 77 -or $stream.ReadByte() -ne 90) { throw "MZ形式ではありません: $Path" } }
    finally { $stream.Dispose() }
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}
function Get-VersionNeutralContent {
    param([string]$Path, [string]$Content)
    switch -Exact ($Path) {
        'src-tauri/tauri.conf.json' {
            $config = $Content | ConvertFrom-Json
            $config.version = '__APP_VERSION__'
            return ($config | ConvertTo-Json -Depth 100 -Compress)
        }
        { $_ -in 'src-tauri/Cargo.toml', 'xtask/Cargo.toml' } {
            # 自packageの節だけを対象にし、依存節のversionには触れない。
            return [regex]::Replace($Content, '(?ms)^\[package\]\r?\n.*?(?=^\[|\z)', {
                param($section)
                [regex]::Replace($section.Value, '(?m)^(version\s*=\s*")[^"]+("\s*)$', '${1}__APP_VERSION__${2}')
            })
        }
        { $_ -in 'src-tauri/Cargo.lock', 'xtask/Cargo.lock' } {
            $package = if ($Path -eq 'src-tauri/Cargo.lock') { 'picoagent' } else { 'xtask' }
            return [regex]::Replace($Content, "(?m)(^\[\[package\]\]\r?\nname = `"$package`"\r?\nversion = `")[^`"]+(`")", '${1}__APP_VERSION__${2}')
        }
        default { return $Content }
    }
}
function Test-EffectiveChange {
    param([string]$Before, [string]$After)
    # 日本語パスをGitのCエスケープへ変換させない。差分行全体からversionを消さない。
    $paths = @(Invoke-Checked git @('-c', 'core.quotepath=false', 'diff', '--name-only', '--no-renames', $Before, $After, '--', 'src-tauri', 'ui', 'skills', 'crates', 'characters', 'xtask', 'character-maker', 'distribution/release.json', 'scripts/build-partner-installer.ps1'))
    $selfVersionFiles = @('src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'xtask/Cargo.toml', 'xtask/Cargo.lock')
    foreach ($path in $paths) {
        if ($path -notin $selfVersionFiles) { return $true }
        # 追加・削除された設定ファイルは、版の違いではなく実効変更として扱う。
        & git cat-file -e "${Before}:$path" 2>$null
        if ($LASTEXITCODE -ne 0) { return $true }
        & git cat-file -e "${After}:$path" 2>$null
        if ($LASTEXITCODE -ne 0) { return $true }
        $old = (Invoke-Checked git @('show', "${Before}:$path")) -join "`n"
        $new = (Invoke-Checked git @('show', "${After}:$path")) -join "`n"
        if ((Get-VersionNeutralContent $path $old) -cne (Get-VersionNeutralContent $path $new)) { return $true }
    }
    return $false
}

# テストは関数だけを読み込み、外部コマンドを実行しない。
if ($MyInvocation.InvocationName -eq '.') { return }
if (-not $IsWindows) { throw 'Windows専用です。' }
if ($PublicAlpha -and $Kind -ne 'Alpha') { throw '公開先の指定はAlpha単体だけに使えます。関係者資産は公開できません。' }
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    Invoke-Checked git @('fetch', '--quiet', 'origin', 'main')
    $source = (Invoke-Checked git @('rev-parse', 'HEAD')).Trim()
    $main = (Invoke-Checked git @('rev-parse', 'origin/main')).Trim()
    if ($source -ne $main -or @(Invoke-Checked git @('status', '--porcelain', '--untracked-files=no')).Count) {
        throw '追跡変更のない最新origin/mainで実行してください。採番変更もPRでマージしてから再実行します。'
    }
    $version = (Get-Content -Raw -Encoding UTF8 src-tauri/tauri.conf.json | ConvertFrom-Json).version
    $plan = @(Get-WindowsPlan $Kind $version)
    if ($Kind -eq 'All') {
        if ((Invoke-Checked gh @('api', 'repos/benybeny777/PicoAgent-Distribution', '--jq', '.visibility')).Trim() -ne 'public') { throw '公開Alphaの保存先を確認できません。' }
        Write-Host '全種類: Alphaは公開先Draft、関係者版は非公開へ保存します。公開Alphaの最終公開は21側PR後です。'
    }
    $repo = if ($PublicAlpha) { 'benybeny777/PicoAgent-Distribution' } else { 'benybeny777/PicoAgent' }
    $visibility = (Invoke-Checked gh @('api', "repos/$repo", '--jq', '.visibility')).Trim()
    if (-not $PublicAlpha -and $visibility -ne 'private') { throw '保存先が非公開ではないため停止しました。' }
    if ($PublicAlpha -and $visibility -ne 'public') { throw '公開Alphaの保存先を確認できません。' }
    $tag = if ($PublicAlpha) { "v$version" } else { "v$version-windows-private" }
    $releases = @((Invoke-Checked gh @('api', "repos/$repo/releases", '--paginate', '--jq', '.[] | {tag_name,body}')) | ForEach-Object { $_ | ConvertFrom-Json })
    if (@($releases | Where-Object { $_.tag_name -match "^v?$([regex]::Escape($version))(?:$|-)" }).Count) {
        throw "版 $version のReleaseが存在します。追加・上書きは禁止です。patchを採番しPRをマージしてから再実行してください。"
    }
    if (-not $PreviousSourceCommit -and $releases.Count -gt 0) {
        $match = [regex]::Match([string]$releases[0].body, 'source_commit:\s*([0-9a-f]{40})')
        if ($match.Success) { $PreviousSourceCommit = $match.Groups[1].Value }
    }
    if (-not $PreviousSourceCommit) { throw '直前版のビルド元が不明です。Release記録を確認し -PreviousSourceCommit に40桁のコミットを指定してください。' }
    Invoke-Checked git @('cat-file', '-e', "$PreviousSourceCommit`^{commit}")
    if (-not (Test-EffectiveChange $PreviousSourceCommit $source)) { throw '実効内容が同じなのでビルド・アップロードしません。' }
    if ($Kind -eq 'All') {
        $publicTags = @(Invoke-Checked gh @('api', 'repos/benybeny777/PicoAgent-Distribution/releases', '--paginate', '--jq', '.[].tag_name'))
        if (@($publicTags | Where-Object { $_ -match "^v?$([regex]::Escape($version))(?:$|-)" }).Count) { throw '公開先にも同じ版が存在します。新しいpatch版で実行してください。' }
    }
    $running = @(Get-CimInstance Win32_Process | Where-Object {
        $_.Name -match '^(picoagent|llama-server|whisper-cli)(\.exe)?$' -or
        ($_.CommandLine -and $_.CommandLine -match '(?i)PicoAgent[\\/].*(aivis|browser-profile)|tools[\\/]screenshots[\\/]')
    })
    if ($running.Count) { throw 'PicoAgentが起動中です。作業を保存してアプリを終了してから実行してください。' }
    Write-Host "対象: $($plan.Kind -join ', ') / 版: $version / 保存先: $repo / 新規タグ: $tag"
    if ($PrepareOnly) { Write-Host '事前検査完了。ビルド・アップロードは実行していません。'; return }
    $runRoot = Join-Path $root "temp/windows-publish/$version-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $runRoot | Out-Null
    $records = @()
    foreach ($entry in $plan) {
        $started = [datetime]::UtcNow
        switch ($entry.Kind) {
            'Alpha' { Invoke-Checked cargo @('xtask', 'build-beta', '--bundles', 'nsis') }
            'Partner' { Invoke-Checked pwsh @('-NoProfile', '-File', 'scripts/build-partner-installer.ps1', '-SourceDir', $PartnerSourceDir) }
        }
        $hash = Assert-Installer (Join-Path $root $entry.Source) $started
        $destination = Join-Path $runRoot $entry.File
        Copy-Item -LiteralPath (Join-Path $root $entry.Source) -Destination $destination
        $records += [pscustomobject]@{ kind = $entry.Kind; file = $entry.File; sha256 = $hash; sizeMB = [math]::Round((Get-Item $destination).Length / 1e6, 2) }
    }
    if (@(Invoke-Checked git @('status', '--porcelain', '--untracked-files=no')).Count) { throw 'ビルド中に追跡ファイルが変わりました。公開せず停止します。' }
    $groups = if ($Kind -eq 'All') { @('Private', 'Public') } elseif ($PublicAlpha) { @('Public') } else { @('Private') }
    foreach ($group in $groups) {
    $isPublic = $group -eq 'Public'
    $repo = if ($isPublic) { 'benybeny777/PicoAgent-Distribution' } else { 'benybeny777/PicoAgent' }
    $tag = if ($isPublic) { "v$version" } else { "v$version-windows-private" }
    $selected = @($records | Where-Object { $Kind -ne 'All' -or ($isPublic -eq ($_.kind -eq 'Alpha')) })
    $notes = Join-Path $runRoot "release-notes-$group.md"
    $body = "Windows Alpha $version`n`nsource_commit: $source`n`n" + (($selected | ForEach-Object { "- $($_.kind): $($_.file) / $($_.sizeMB) MB / SHA-256: $($_.sha256)" }) -join "`n")
    if ($isPublic) { $body += "`n`n公開保留: 21側の目的別PR・latest.json・Pages照合が必要です。" }
    [IO.File]::WriteAllText($notes, $body, [Text.UTF8Encoding]::new($false))
    # 全生成の成功後に一度だけ作成する。既存Releaseへuploadしない。
    $assets = @($selected | ForEach-Object { Join-Path $runRoot $_.file })
    $arguments = @('release', 'create', $tag, '--repo', $repo, '--draft', '--prerelease', '--title', "PicoAgent $version Windows Alpha", '--notes-file', $notes) + $assets
    if (-not $isPublic) { $arguments += @('--target', $source) }
    Invoke-Checked gh $arguments
    $downloadRoot = Join-Path $runRoot "download-verification-$group"
    Invoke-Checked gh @('release', 'download', $tag, '--repo', $repo, '--dir', $downloadRoot)
    foreach ($record in $selected) {
        if ((Get-FileHash -LiteralPath (Join-Path $downloadRoot $record.file) -Algorithm SHA256).Hash -ne $record.sha256) { throw '再取得した成果物のSHA-256が違います。Draftのまま停止します。' }
    }
    if ($isPublic) { Write-Warning 'アップロード済み・公開未完了です。21側PR、更新フィード、Pages検証後にDraft解除してください。'; continue }
    if ((Invoke-Checked gh @('api', "repos/$repo", '--jq', '.visibility')).Trim() -ne 'private') { throw '保存先の非公開状態が変わりました。Draftのまま停止します。' }
    Write-Host "非公開Draftへの保存・再取得検証が完了しました: https://github.com/$repo/releases/tag/$tag"
    }
    if ('Private' -in $groups) { Invoke-Checked gh @('release', 'edit', "v$version-windows-private", '--repo', 'benybeny777/PicoAgent', '--draft=false') }
    if ('Public' -in $groups) { exit 2 }
}
finally { Pop-Location }
