param(
    [string[]]$CharacterIds = @('mugi', 'rin', 'mai', 'rin-tha3'),
    [int]$Scale = 2,
    [string]$SourceRoot = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Drawing

$repositoryRoot = Split-Path -Parent $PSScriptRoot
if (-not $SourceRoot) {
    $SourceRoot = Join-Path $repositoryRoot 'character-sources'
}
$SourceRoot = (Resolve-Path -LiteralPath $SourceRoot).Path
$productionRoot = Join-Path $repositoryRoot 'characters'
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$workRoot = Join-Path $repositoryRoot "temp\character-asset-finish\$timestamp"
$backupRoot = Join-Path $workRoot 'backup'
$processedRoot = Join-Path $workRoot 'processed'
$stagedRoot = Join-Path $workRoot 'staged'
$qaRoot = Join-Path $workRoot 'qa'
New-Item -ItemType Directory -Force -Path $backupRoot, $processedRoot, $stagedRoot, $qaRoot | Out-Null

$sourceIndex = Join-Path $SourceRoot 'index.json'
if (Test-Path -LiteralPath $sourceIndex) {
    $expandedRoot = Join-Path $workRoot 'source-expanded'
    $index = Get-Content -LiteralPath $sourceIndex -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($index.version -ne 1 -or $null -eq $index.characters) {
        throw "Character source index is invalid: $sourceIndex"
    }
    foreach ($characterId in $CharacterIds) {
        $mappings = $index.characters.PSObject.Properties[$characterId].Value
        if ($null -eq $mappings) {
            throw "Character source index entry not found: $characterId"
        }
        foreach ($mapping in $mappings.PSObject.Properties) {
            $relative = [string]$mapping.Name
            $blob = [string]$mapping.Value
            if ([IO.Path]::IsPathRooted($relative) -or $relative.Split('/\') -contains '..' -or
                [IO.Path]::IsPathRooted($blob) -or $blob.Split('/\') -contains '..') {
                throw "Unsafe character source path: $relative -> $blob"
            }
            $source = Join-Path $SourceRoot $blob
            if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
                throw "Character source blob not found: $blob"
            }
            $destination = Join-Path (Join-Path $expandedRoot $characterId) $relative
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
            Copy-Item -LiteralPath $source -Destination $destination
        }
    }
    $SourceRoot = $expandedRoot
}

Add-Type -Path (Join-Path $PSScriptRoot 'CharacterAssetImageProcessor.cs') -ReferencedAssemblies 'System.Drawing'

function Convert-ToJsPath([string]$Path) {
    return $Path.Replace('\', '/').Replace('"', '\"')
}

function Get-SourceFiles([string]$CharacterId) {
    $characterRoot = Join-Path $SourceRoot $CharacterId
    if (-not (Test-Path -LiteralPath $characterRoot)) {
        throw "Character source not found: $CharacterId"
    }
    $files = @()
    $preview = Join-Path $characterRoot 'preview.png'
    if (Test-Path -LiteralPath $preview) {
        $files += Get-Item -LiteralPath $preview
    }
    foreach ($directoryName in @('animations', 'layers', 'boot')) {
        $directory = Join-Path $characterRoot $directoryName
        if (Test-Path -LiteralPath $directory) {
            $files += Get-ChildItem -LiteralPath $directory -Recurse -File -Filter '*.png'
        }
    }
    return @($files | Sort-Object FullName -Unique)
}

function Invoke-IsolatedPhotoshop([string]$ScriptPath) {
    if (Get-Process -Name 'Photoshop' -ErrorAction SilentlyContinue) {
        throw 'Photoshop is already running. Close it before starting this isolated batch.'
    }
    $photoshop = $null
    try {
        $photoshop = New-Object -ComObject Photoshop.Application
        $photoshop.DisplayDialogs = 3
        $photoshop.DoJavaScriptFile($ScriptPath) | Out-Null
    }
    finally {
        if ($null -ne $photoshop) {
            try {
                $photoshop.Quit()
            }
            finally {
                [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($photoshop)
            }
        }
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while ((Get-Process -Name 'Photoshop' -ErrorAction SilentlyContinue) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 250
    }
    if (Get-Process -Name 'Photoshop' -ErrorAction SilentlyContinue) {
        Get-Process -Name 'Photoshop' -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Seconds 1
    }
    if (Get-Process -Name 'Photoshop' -ErrorAction SilentlyContinue) {
        throw 'Photoshop could not be reclaimed after the isolated batch.'
    }
}

function Write-UpscaleScript([object[]]$Jobs, [string]$Path) {
    $lines = @('#target photoshop', 'app.displayDialogs = DialogModes.NO;', 'var jobs = [')
    foreach ($job in $Jobs) {
        if ($job.ResizeScale -le 1) { continue }
        $inputPath = Convert-ToJsPath $job.NormalizedOutput
        $outputPath = Convert-ToJsPath $job.UpscaledOutput
        $lines += ('{{ input: "{0}", output: "{1}", scale: {2} }},' -f $inputPath, $outputPath, $job.ResizeScale)
    }
    $lines += @(
        '];',
        'for (var index = 0; index < jobs.length; index++) {',
        '    var documentRef = app.open(new File(jobs[index].input));',
        '    try {',
        '        var width = UnitValue(documentRef.width.as("px") * jobs[index].scale, "px");',
        '        var height = UnitValue(documentRef.height.as("px") * jobs[index].scale, "px");',
        '        documentRef.resizeImage(width, height, null, ResampleMethod.PRESERVEDETAILS);',
        '        var options = new PNGSaveOptions();',
        '        options.compression = 6;',
        '        options.interlaced = false;',
        '        documentRef.saveAs(new File(jobs[index].output), options, true, Extension.LOWERCASE);',
        '    } finally {',
        '        documentRef.close(SaveOptions.DONOTSAVECHANGES);',
        '    }',
        '}'
    )
    Set-Content -LiteralPath $Path -Value ($lines -join "`r`n") -Encoding UTF8
}

function Get-QaBackground([string]$Mode) {
    switch ($Mode) {
        'white' { return [System.Drawing.Color]::White }
        'navy' { return [System.Drawing.Color]::FromArgb(255, 18, 31, 58) }
        'magenta' { return [System.Drawing.Color]::FromArgb(255, 255, 62, 174) }
        default { return [System.Drawing.Color]::FromArgb(255, 220, 230, 242) }
    }
}

function Fill-QaBackground(
    [System.Drawing.Graphics]$Graphics,
    [string]$Mode,
    [int]$Left,
    [int]$Top,
    [int]$Width,
    [int]$Height
) {
    if ($Mode -ne 'checker') {
        $brush = [System.Drawing.SolidBrush]::new((Get-QaBackground $Mode))
        try { $Graphics.FillRectangle($brush, $Left, $Top, $Width, $Height) }
        finally { $brush.Dispose() }
        return
    }
    $size = 20
    $colors = @(
        [System.Drawing.Color]::FromArgb(255, 238, 238, 238),
        [System.Drawing.Color]::FromArgb(255, 170, 170, 170)
    )
    for ($y = 0; $y -lt $Height; $y += $size) {
        for ($x = 0; $x -lt $Width; $x += $size) {
            $color = $colors[(($x / $size) + ($y / $size)) % 2]
            $brush = [System.Drawing.SolidBrush]::new($color)
            try { $Graphics.FillRectangle($brush, $Left + $x, $Top + $y, $size, $size) }
            finally { $brush.Dispose() }
        }
    }
}

function New-QaSheet(
    [string]$CharacterId,
    [object[]]$Files,
    [string]$Mode,
    [string]$OutputPath
) {
    $uniqueFiles = @($Files | Sort-Object Hash -Unique)
    $columns = 5
    $cellWidth = 240
    $cellHeight = 300
    $rows = [Math]::Ceiling($uniqueFiles.Count / $columns)
    $sheet = [System.Drawing.Bitmap]::new(
        $columns * $cellWidth,
        $rows * $cellHeight,
        [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
    )
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($sheet)
        try {
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $font = [System.Drawing.Font]::new('Yu Gothic UI', 9)
            $textColor = if ($Mode -eq 'navy') { [System.Drawing.Color]::White } else { [System.Drawing.Color]::FromArgb(255, 25, 35, 55) }
            $textBrush = [System.Drawing.SolidBrush]::new($textColor)
            try {
                for ($index = 0; $index -lt $uniqueFiles.Count; $index++) {
                    $column = $index % $columns
                    $row = [Math]::Floor($index / $columns)
                    $left = $column * $cellWidth
                    $top = $row * $cellHeight
                    Fill-QaBackground $graphics $Mode $left $top $cellWidth $cellHeight
                    $image = [System.Drawing.Image]::FromFile($uniqueFiles[$index].Path)
                    try {
                        $ratio = [Math]::Min(($cellWidth - 20) / $image.Width, ($cellHeight - 38) / $image.Height)
                        $drawWidth = [int]($image.Width * $ratio)
                        $drawHeight = [int]($image.Height * $ratio)
                        $drawLeft = $left + [int](($cellWidth - $drawWidth) / 2)
                        $graphics.DrawImage($image, $drawLeft, $top + 4, $drawWidth, $drawHeight)
                    }
                    finally { $image.Dispose() }
                    $graphics.DrawString($uniqueFiles[$index].Label, $font, $textBrush, $left + 6, $top + $cellHeight - 28)
                }
            }
            finally {
                $font.Dispose()
                $textBrush.Dispose()
            }
        }
        finally { $graphics.Dispose() }
        $sheet.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $sheet.Dispose() }
}

if ($Scale -lt 1 -or $Scale -gt 4) {
    throw 'Scale must be between 1 and 4.'
}

$allJobs = @()
$destinationRecords = @()
foreach ($characterId in $CharacterIds) {
    $sourceCharacterRoot = Join-Path $SourceRoot $characterId
    $files = @(Get-SourceFiles $characterId)
    if ($files.Count -eq 0) { throw "No source PNG files: $characterId" }
    $groups = $files | ForEach-Object {
        [PSCustomObject]@{
            File = $_
            Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
            Relative = $_.FullName.Substring($sourceCharacterRoot.Length + 1)
        }
    } | Group-Object Hash

    $jobIndex = 0
    foreach ($group in $groups) {
        $representative = $group.Group[0]
        $prefix = "$characterId-$jobIndex"
        $job = [PSCustomObject]@{
            CharacterId = $characterId
            Source = $representative.File.FullName
            SourceAnalysis = [PicoAgent.Tools.CharacterAssetImageProcessor]::Analyze($representative.File.FullName)
            RawMaskOutput = Join-Path $processedRoot "$prefix-raw-mask.png"
            NormalizedOutput = Join-Path $processedRoot "$prefix-normalized.png"
            UpscaledOutput = Join-Path $processedRoot "$prefix-upscaled.png"
            FinalOutput = Join-Path $processedRoot "$prefix-final.png"
            ResizeScale = if ($representative.Relative.StartsWith('boot\', [System.StringComparison]::OrdinalIgnoreCase)) { 1 } else { $Scale }
            AlphaThreshold = if ($characterId -eq 'rin-tha3') { 32 } else { 8 }
            Destinations = @($group.Group)
        }
        $allJobs += $job
        foreach ($item in $group.Group) {
            $destination = Join-Path (Join-Path $productionRoot $characterId) $item.Relative
            $staged = Join-Path (Join-Path $stagedRoot $characterId) $item.Relative
            $backup = Join-Path (Join-Path $backupRoot $characterId) $item.Relative
            $destinationRecords += [PSCustomObject]@{
                CharacterId = $characterId
                Relative = $item.Relative
                Destination = $destination
                Staged = $staged
                Backup = $backup
                Existed = Test-Path -LiteralPath $destination
                Job = $job
            }
        }
        $jobIndex++
    }
}

foreach ($record in $destinationRecords) {
    if (-not $record.Existed) { continue }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $record.Backup) | Out-Null
    Copy-Item -LiteralPath $record.Destination -Destination $record.Backup -Force
}

foreach ($job in $allJobs) {
    [PicoAgent.Tools.CharacterAssetImageProcessor]::NormalizeAlpha(
        $job.Source,
        $job.RawMaskOutput,
        $job.AlphaThreshold
    )
    [PicoAgent.Tools.CharacterAssetImageProcessor]::RemoveWhiteMatte(
        $job.RawMaskOutput,
        $job.NormalizedOutput
    )
}

$upscaleJobs = @($allJobs | Where-Object ResizeScale -gt 1)
if ($upscaleJobs.Count) {
    $upscaleScript = Join-Path $workRoot 'photoshop-upscale.jsx'
    Write-UpscaleScript $upscaleJobs $upscaleScript
    Write-Host "Upscaling $($upscaleJobs.Count) images with Photoshop Preserve Details 2.0."
    Invoke-IsolatedPhotoshop $upscaleScript
}

foreach ($job in $allJobs) {
    $upscaleSource = if ($job.ResizeScale -gt 1) { $job.UpscaledOutput } else { $job.NormalizedOutput }
    [PicoAgent.Tools.CharacterAssetImageProcessor]::ApplyScaledAlphaMask(
        $upscaleSource,
        $job.NormalizedOutput,
        $job.FinalOutput,
        $job.AlphaThreshold
    )
    foreach ($record in @($destinationRecords | Where-Object Job -eq $job)) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $record.Staged) | Out-Null
        Copy-Item -LiteralPath $job.FinalOutput -Destination $record.Staged -Force
    }
}

$reportItems = @()
foreach ($record in $destinationRecords) {
    $analysis = [PicoAgent.Tools.CharacterAssetImageProcessor]::Analyze($record.Staged)
    $expectedWidth = $record.Job.SourceAnalysis.Width * $record.Job.ResizeScale
    $expectedHeight = $record.Job.SourceAnalysis.Height * $record.Job.ResizeScale
    if ($analysis.Width -ne $expectedWidth -or $analysis.Height -ne $expectedHeight) {
        throw "Unexpected dimensions: $($record.CharacterId)/$($record.Relative) $($analysis.Width)x$($analysis.Height)"
    }
    if ($analysis.VisiblePixels -eq 0) {
        throw "Empty image: $($record.CharacterId)/$($record.Relative)"
    }
    $source = $record.Job.SourceAnalysis
    $scale = $record.Job.ResizeScale
    $bboxDelta = [Math]::Max(
        [Math]::Abs($analysis.MinX - $source.MinX * $scale),
        [Math]::Max(
            [Math]::Abs($analysis.MinY - $source.MinY * $scale),
            [Math]::Max(
                [Math]::Abs($analysis.MaxX - (($source.MaxX + 1) * $scale - 1)),
                [Math]::Abs($analysis.MaxY - (($source.MaxY + 1) * $scale - 1))
            )
        )
    )
    $warnings = @()
    if ($bboxDelta -gt (4 * $scale)) { $warnings += "bbox delta $bboxDelta" }
    if (@($analysis.CornerAlpha | Where-Object { $_ -gt 8 }).Count -eq 4) { $warnings += 'all corners opaque' }
    $reportItems += [PSCustomObject]@{
        character = $record.CharacterId
        path = $record.Relative.Replace('\', '/')
        resamplingMethod = if ($scale -gt 1) { 'Photoshop Preserve Details 2.0' } else { 'none' }
        sourceWidth = $source.Width
        sourceHeight = $source.Height
        width = $analysis.Width
        height = $analysis.Height
        bbox = @($analysis.MinX, $analysis.MinY, $analysis.MaxX, $analysis.MaxY)
        bboxDelta = $bboxDelta
        cornerAlpha = $analysis.CornerAlpha
        semiTransparentPixels = $analysis.SemiTransparentPixels
        lightNeutralEdgePixels = $analysis.LightNeutralEdgePixels
        darkNeutralEdgePixels = $analysis.DarkNeutralEdgePixels
        warnings = $warnings
    }
}

$report = [PSCustomObject]@{
    generatedAt = (Get-Date).ToString('o')
    sourceRoot = $SourceRoot
    scale = $Scale
    resamplingMethod = if ($Scale -gt 1) { 'Photoshop Preserve Details 2.0' } else { 'none' }
    files = $destinationRecords.Count
    uniqueImages = $allJobs.Count
    warnings = @($reportItems | Where-Object { $_.warnings.Count -gt 0 }).Count
    items = $reportItems
}
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $qaRoot 'qa-report.json') -Encoding UTF8

foreach ($characterId in $CharacterIds) {
    $qaFiles = @($destinationRecords | Where-Object CharacterId -eq $characterId | ForEach-Object {
        [PSCustomObject]@{
            Path = $_.Staged
            Label = $_.Relative.Replace('\', '/')
            Hash = (Get-FileHash -LiteralPath $_.Staged -Algorithm SHA256).Hash
        }
    })
    foreach ($mode in @('white', 'navy', 'magenta', 'checker')) {
        $qaPath = Join-Path $qaRoot "$characterId-$mode.png"
        New-QaSheet $characterId $qaFiles $mode $qaPath
        Write-Host "QA: $qaPath"
    }
}

$applied = @()
try {
    foreach ($record in $destinationRecords) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $record.Destination) | Out-Null
        Copy-Item -LiteralPath $record.Staged -Destination $record.Destination -Force
        $applied += $record
    }
}
catch {
    foreach ($record in $destinationRecords) {
        if ($record.Existed) {
            Copy-Item -LiteralPath $record.Backup -Destination $record.Destination -Force
        }
        elseif (Test-Path -LiteralPath $record.Destination) {
            Remove-Item -LiteralPath $record.Destination -Force
        }
    }
    throw
}

Write-Host "Original image backup: $backupRoot"
Write-Host "QA report: $(Join-Path $qaRoot 'qa-report.json')"
Write-Host 'Finished the 2D character images transactionally.'
