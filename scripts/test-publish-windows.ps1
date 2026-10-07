#Requires -Version 7
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/publish-windows.ps1"
function Assert-True([bool]$Value, [string]$Label) { if (-not $Value) { throw $Label }; Write-Host "合格: $Label" }
function Assert-Rejected([scriptblock]$Action, [string]$Label) {
    $rejected = $false
    try { & $Action | Out-Null } catch { $rejected = $true }
    Assert-True $rejected $Label
}
$plan = @(Get-WindowsPlan All '1.2.3')
Assert-True ($plan.Count -eq 2 -and @($plan.File | Select-Object -Unique).Count -eq 2) '全種類のファイル名は重複せず2件'
foreach ($kind in 'Alpha', 'Partner') { Assert-True (@(Get-WindowsPlan $kind '1.2.3').Count -eq 1) "$kind 単体は1件だけ" }
Assert-Rejected { Get-WindowsPlan Review '1.2.3' } '移行済みの審査版指定は空計画へせず拒否'
Assert-Rejected { Get-WindowsPlan Unknown '1.2.3' } '不明な種類を拒否'
Assert-Rejected { Get-WindowsPlan All '../1' } '不正な版を拒否'
$testRoot = Join-Path (Split-Path $PSScriptRoot) "temp/publish-test-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory $testRoot | Out-Null
try {
    $file = Join-Path $testRoot 'installer.exe'
    [IO.File]::WriteAllBytes($file, [byte[]](77,90,0,0))
    Assert-True ((Assert-Installer $file ([datetime]::UtcNow.AddSeconds(-5))) -eq (Get-FileHash $file).Hash) 'MZ・生成時刻・SHA256を検証'
    Assert-Rejected { Assert-Installer $file ([datetime]::UtcNow.AddSeconds(5)) } '旧成果物を拒否'
    [IO.File]::WriteAllBytes($file, [byte[]](0,0,0,0))
    Assert-Rejected { Assert-Installer $file ([datetime]::UtcNow.AddSeconds(-5)) } '非PE形式を拒否'
    $source = Get-Content -Raw -Encoding UTF8 "$PSScriptRoot/publish-windows.ps1"
    Assert-True ($source -notmatch "'release', 'upload'|--clobber|-AllowStale") '既存への追加・上書き・古い版の強制実行なし'
    Assert-True ($source.Contains("if (`$PrepareOnly)") -and $source.IndexOf('if ($PrepareOnly)') -lt $source.IndexOf("'xtask', 'build-beta'")) '準備専用はビルドより前に終了'
    Assert-True ($source.IndexOf('foreach ($entry in $plan)') -lt $source.IndexOf("'release', 'create'")) '全ビルド後だけReleaseを作成'
    Push-Location $testRoot
    try {
        Invoke-Checked git @('init', '--quiet')
        Invoke-Checked git @('config', 'user.name', '検査')
        Invoke-Checked git @('config', 'user.email', 'test@example.invalid')
        Invoke-Checked git @('config', 'core.autocrlf', 'false')
        New-Item -ItemType Directory src-tauri, xtask, characters | Out-Null
        $utf8 = [Text.UTF8Encoding]::new($false)
        foreach ($entry in @(@('src-tauri', 'picoagent'), @('xtask', 'xtask'))) {
            [IO.File]::WriteAllText((Join-Path $testRoot "$($entry[0])/Cargo.toml"), "[package]`nname = `"$($entry[1])`"`nversion = `"1.0.0`"`n[dependencies.demo]`nversion = `"2.0.0`"`n", $utf8)
            [IO.File]::WriteAllText((Join-Path $testRoot "$($entry[0])/Cargo.lock"), "[[package]]`nname = `"$($entry[1])`"`nversion = `"1.0.0`"`n[[package]]`nname = `"demo`"`nversion = `"2.0.0`"`n", $utf8)
        }
        [IO.File]::WriteAllText((Join-Path $testRoot 'src-tauri/tauri.conf.json'), '{"version":"1.0.0","identifier":"example"}', $utf8)
        [IO.File]::WriteAllText((Join-Path $testRoot 'characters/日本語素材.txt'), '旧素材', $utf8)
        [IO.File]::WriteAllText((Join-Path $testRoot 'src-tauri/model.json'), '{"version":"1.0.0"}', $utf8)
        Invoke-Checked git @('add', '.')
        Invoke-Checked git @('commit', '--quiet', '-m', '初期状態')
        $before = (Invoke-Checked git @('rev-parse', 'HEAD')).Trim()
        foreach ($path in 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'xtask/Cargo.toml', 'xtask/Cargo.lock', 'src-tauri/tauri.conf.json') {
            $full = Join-Path $testRoot $path
            [IO.File]::WriteAllText($full, ([IO.File]::ReadAllText($full).Replace('1.0.0', '1.0.1')), $utf8)
        }
        Invoke-Checked git @('add', '.')
        Invoke-Checked git @('commit', '--quiet', '-m', '自版だけ変更')
        Assert-True (-not (Test-EffectiveChange $before HEAD)) '本体・xtask・設定・lockの自版だけなら再ビルドしない'
        foreach ($case in @(
            @('characters/日本語素材.txt', '旧素材', '新素材', '日本語パスだけの変更を検出'),
            @('src-tauri/Cargo.toml', '2.0.0', '2.0.1', '依存節のversionだけの変更を検出'),
            @('xtask/Cargo.lock', '2.0.0', '2.0.1', 'lockの依存versionだけの変更を検出'),
            @('src-tauri/model.json', '1.0.0', '1.0.1', 'モデルのversionだけの変更を検出')
        )) {
            $before = (Invoke-Checked git @('rev-parse', 'HEAD')).Trim()
            $full = Join-Path $testRoot $case[0]
            [IO.File]::WriteAllText($full, ([IO.File]::ReadAllText($full).Replace($case[1], $case[2])), $utf8)
            Invoke-Checked git @('add', '.')
            Invoke-Checked git @('commit', '--quiet', '-m', $case[3])
            Assert-True (Test-EffectiveChange $before HEAD) $case[3]
        }
    }
    finally { Pop-Location }
}
finally {
    $expected = [IO.Path]::GetFullPath((Join-Path (Split-Path $PSScriptRoot) 'temp')) + [IO.Path]::DirectorySeparatorChar
    if (-not [IO.Path]::GetFullPath($testRoot).StartsWith($expected, [StringComparison]::OrdinalIgnoreCase)) { throw 'テスト削除先が範囲外です。' }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
