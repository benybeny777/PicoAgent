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
Assert-True ($plan.Count -eq 3 -and @($plan.File | Select-Object -Unique).Count -eq 3) '全種類のファイル名は重複せず3件'
foreach ($kind in 'Alpha', 'Partner', 'Review') { Assert-True (@(Get-WindowsPlan $kind '1.2.3').Count -eq 1) "$kind 単体は1件だけ" }
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
}
finally {
    $expected = [IO.Path]::GetFullPath((Join-Path (Split-Path $PSScriptRoot) 'temp')) + [IO.Path]::DirectorySeparatorChar
    if (-not [IO.Path]::GetFullPath($testRoot).StartsWith($expected, [StringComparison]::OrdinalIgnoreCase)) { throw 'テスト削除先が範囲外です。' }
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
