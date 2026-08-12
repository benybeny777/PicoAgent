param(
    [string[]]$CharacterIds = @('koharu', 'sena', 'noa'),
    [string[]]$LayerNames = @('base', 'blink_half', 'blink_closed', 'mouth_closed', 'mouth_open', 'gaze_left', 'gaze_right', 'expression_happy', 'expression_sad'),
    [int]$Scale = 2
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($Scale -lt 2 -or $Scale -gt 4) { throw 'Scale must be between 2 and 4.' }
if (Get-Process -Name 'Photoshop' -ErrorAction SilentlyContinue) {
    throw 'Photoshop must be closed before the isolated batch.'
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$workRoot = Join-Path $repositoryRoot "temp\sharp-expression-finish\$timestamp"
$backupRoot = Join-Path $workRoot 'backup'
$outputRoot = Join-Path $workRoot 'output'
$qaRoot = Join-Path $workRoot 'qa'
New-Item -ItemType Directory -Force -Path $backupRoot, $outputRoot, $qaRoot | Out-Null

Add-Type -Path (Join-Path $PSScriptRoot 'CharacterAssetImageProcessor.cs') -ReferencedAssemblies 'System.Drawing'

function Convert-ToJsPath([string]$Path) {
    return $Path.Replace('\', '/').Replace('"', '\"')
}

$jobs = @()
foreach ($characterId in $CharacterIds) {
    foreach ($name in $LayerNames) {
        $source = Join-Path $repositoryRoot "characters\$characterId\layers\$name.png"
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "素材がありません: $source" }
        $backup = Join-Path $backupRoot "$characterId\$name.png"
        $photoshopOutput = Join-Path $outputRoot "$characterId-$name-photoshop.png"
        $finalOutput = Join-Path $outputRoot "$characterId-$name-final.png"
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $backup) | Out-Null
        Copy-Item -LiteralPath $source -Destination $backup
        $jobs += [PSCustomObject]@{
            CharacterId = $characterId
            Name = $name
            Source = $source
            Backup = $backup
            PhotoshopOutput = $photoshopOutput
            FinalOutput = $finalOutput
            Analysis = [PicoAgent.Tools.CharacterAssetImageProcessor]::Analyze($source)
        }
    }
}

$jsxPath = Join-Path $workRoot 'photoshop-preserve-details.jsx'
$lines = @('#target photoshop', 'app.displayDialogs = DialogModes.NO;', 'var jobs = [')
foreach ($job in $jobs) {
    $lines += ('{{ input: "{0}", output: "{1}", scale: {2} }},' -f (Convert-ToJsPath $job.Source), (Convert-ToJsPath $job.PhotoshopOutput), $Scale)
}
$lines += @(
    '];',
    'for (var index = 0; index < jobs.length; index++) {',
    '  var documentRef = app.open(new File(jobs[index].input));',
    '  try {',
    '    documentRef.resizeImage(UnitValue(documentRef.width.as("px") * jobs[index].scale, "px"), UnitValue(documentRef.height.as("px") * jobs[index].scale, "px"), null, ResampleMethod.PRESERVEDETAILS);',
    '    var options = new PNGSaveOptions(); options.compression = 6; options.interlaced = false;',
    '    documentRef.saveAs(new File(jobs[index].output), options, true, Extension.LOWERCASE);',
    '  } finally { documentRef.close(SaveOptions.DONOTSAVECHANGES); }',
    '}'
)
Set-Content -LiteralPath $jsxPath -Value ($lines -join "`r`n") -Encoding UTF8

$photoshop = $null
try {
    $photoshop = New-Object -ComObject Photoshop.Application
    $photoshop.Visible = $false
    $photoshop.DisplayDialogs = 3
    $photoshop.DoJavaScriptFile($jsxPath) | Out-Null
}
finally {
    if ($null -ne $photoshop) {
        try { $photoshop.Quit() }
        finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($photoshop) }
    }
}

$report = @()
foreach ($job in $jobs) {
    [PicoAgent.Tools.CharacterAssetImageProcessor]::ApplyScaledAlphaMask(
        $job.PhotoshopOutput,
        $job.Source,
        $job.FinalOutput,
        8
    )
    $analysis = [PicoAgent.Tools.CharacterAssetImageProcessor]::Analyze($job.FinalOutput)
    if ($analysis.Width -ne $job.Analysis.Width * $Scale -or $analysis.Height -ne $job.Analysis.Height * $Scale) {
        throw "Photoshop出力寸法が不正です: $($job.CharacterId)/$($job.Name)"
    }
    if ($analysis.VisiblePixels -eq 0 -or @($analysis.CornerAlpha | Where-Object { $_ -gt 8 }).Count -gt 0) {
        throw "Photoshop出力の透過検査に失敗しました: $($job.CharacterId)/$($job.Name)"
    }
    $report += [PSCustomObject]@{
        character = $job.CharacterId
        layer = $job.Name
        width = $analysis.Width
        height = $analysis.Height
        cornerAlpha = $analysis.CornerAlpha
        visiblePixels = $analysis.VisiblePixels
    }
}

$applied = @()
try {
    foreach ($job in $jobs) {
        Copy-Item -LiteralPath $job.FinalOutput -Destination $job.Source -Force
        $applied += $job
    }
}
catch {
    foreach ($job in $applied) { Copy-Item -LiteralPath $job.Backup -Destination $job.Source -Force }
    throw
}

$report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $qaRoot 'qa-report.json') -Encoding UTF8
Write-Host "Photoshop Preserve Details 2.0: $($jobs.Count) images"
Write-Host "Backup: $backupRoot"
Write-Host "QA: $(Join-Path $qaRoot 'qa-report.json')"
