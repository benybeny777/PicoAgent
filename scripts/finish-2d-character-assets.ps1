param(
    [string[]]$CharacterIds = @('mugi', 'rin', 'mai', 'rin-tha3'),
    [int]$Scale = 2
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Drawing

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$workRoot = Join-Path $repositoryRoot "temp\character-asset-finish\$timestamp"
$backupRoot = Join-Path $workRoot 'backup'
$processedRoot = Join-Path $workRoot 'processed'
$qaRoot = Join-Path $workRoot 'qa'
New-Item -ItemType Directory -Force -Path $backupRoot, $processedRoot, $qaRoot | Out-Null

function Convert-ToJsPath([string]$Path) {
    return $Path.Replace('\', '/').Replace('"', '\"')
}

function Get-TargetFiles([string]$CharacterId) {
    $characterRoot = Join-Path $repositoryRoot "characters\$CharacterId"
    if (-not (Test-Path -LiteralPath $characterRoot)) {
        throw "Character not found: $CharacterId"
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

function Convert-AlphaAwareImage(
    [string]$InputPath,
    [string]$OutputPath,
    [int]$AlphaThreshold,
    [int]$ResizeScale
) {
    $loaded = [System.Drawing.Bitmap]::new($InputPath)
    try {
        $source = [System.Drawing.Bitmap]::new(
            $loaded.Width,
            $loaded.Height,
            [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
        )
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($source)
            try {
                $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
                $graphics.DrawImageUnscaled($loaded, 0, 0)
            }
            finally {
                $graphics.Dispose()
            }

            for ($y = 0; $y -lt $source.Height; $y++) {
                for ($x = 0; $x -lt $source.Width; $x++) {
                    $color = $source.GetPixel($x, $y)
                    if ($color.A -le $AlphaThreshold) {
                        $source.SetPixel($x, $y, [System.Drawing.Color]::Transparent)
                    }
                }
            }

            $width = $source.Width * $ResizeScale
            $height = $source.Height * $ResizeScale
            $output = [System.Drawing.Bitmap]::new(
                $width,
                $height,
                [System.Drawing.Imaging.PixelFormat]::Format32bppPArgb
            )
            try {
                $graphics = [System.Drawing.Graphics]::FromImage($output)
                try {
                    $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
                    $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                    $wrap = [System.Drawing.Imaging.ImageAttributes]::new()
                    try {
                        $wrap.SetWrapMode([System.Drawing.Drawing2D.WrapMode]::TileFlipXY)
                        $destination = [System.Drawing.Rectangle]::new(0, 0, $width, $height)
                        $graphics.DrawImage(
                            $source,
                            $destination,
                            0,
                            0,
                            $source.Width,
                            $source.Height,
                            [System.Drawing.GraphicsUnit]::Pixel,
                            $wrap
                        )
                    }
                    finally {
                        $wrap.Dispose()
                    }
                }
                finally {
                    $graphics.Dispose()
                }
                $output.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
            }
            finally {
                $output.Dispose()
            }
        }
        finally {
            $source.Dispose()
        }
    }
    finally {
        $loaded.Dispose()
    }
}

function New-QaSheet([string]$CharacterId, [System.IO.FileInfo[]]$Files, [string]$OutputPath) {
    $uniqueFiles = @($Files | ForEach-Object {
        [PSCustomObject]@{ File = $_; Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    } | Sort-Object Hash -Unique)

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
            $graphics.Clear([System.Drawing.Color]::FromArgb(255, 218, 238, 255))
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $font = [System.Drawing.Font]::new('Yu Gothic UI', 10)
            $textBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 25, 35, 55))
            try {
                for ($index = 0; $index -lt $uniqueFiles.Count; $index++) {
                    $column = $index % $columns
                    $row = [Math]::Floor($index / $columns)
                    $left = $column * $cellWidth
                    $top = $row * $cellHeight
                    $background = if (($column + $row) % 2 -eq 0) {
                        [System.Drawing.Color]::FromArgb(255, 255, 166, 211)
                    }
                    else {
                        [System.Drawing.Color]::FromArgb(255, 151, 225, 211)
                    }
                    $backgroundBrush = [System.Drawing.SolidBrush]::new($background)
                    try {
                        $graphics.FillRectangle($backgroundBrush, $left, $top, $cellWidth, $cellHeight)
                    }
                    finally {
                        $backgroundBrush.Dispose()
                    }

                    $image = [System.Drawing.Image]::FromFile($uniqueFiles[$index].File.FullName)
                    try {
                        $availableWidth = $cellWidth - 20
                        $availableHeight = $cellHeight - 38
                        $ratio = [Math]::Min($availableWidth / $image.Width, $availableHeight / $image.Height)
                        $drawWidth = [int]($image.Width * $ratio)
                        $drawHeight = [int]($image.Height * $ratio)
                        $drawLeft = $left + [int](($cellWidth - $drawWidth) / 2)
                        $drawTop = $top + 4
                        $graphics.DrawImage($image, $drawLeft, $drawTop, $drawWidth, $drawHeight)
                    }
                    finally {
                        $image.Dispose()
                    }
                    $relative = $uniqueFiles[$index].File.FullName.Substring($repositoryRoot.Length + 1)
                    $graphics.DrawString($relative, $font, $textBrush, $left + 6, $top + $cellHeight - 30)
                }
            }
            finally {
                $font.Dispose()
                $textBrush.Dispose()
            }
        }
        finally {
            $graphics.Dispose()
        }
        $sheet.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $sheet.Dispose()
    }
}

$allJobs = @()
foreach ($characterId in $CharacterIds) {
    $files = @(Get-TargetFiles -CharacterId $characterId)
    if ($files.Count -eq 0) {
        throw "No target PNG files: $characterId"
    }

    foreach ($file in $files) {
        $relative = $file.FullName.Substring($repositoryRoot.Length + 1)
        $backupPath = Join-Path $backupRoot $relative
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $backupPath) | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $backupPath -Force
    }

    $groups = $files | ForEach-Object {
        [PSCustomObject]@{ File = $_; Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    } | Group-Object Hash

    $jobIndex = 0
    foreach ($group in $groups) {
        $representative = $group.Group[0].File
        $photoshopOutput = Join-Path $processedRoot "$characterId-$jobIndex-photoshop.png"
        $finalOutput = Join-Path $processedRoot "$characterId-$jobIndex-final.png"
        $allJobs += [PSCustomObject]@{
            CharacterId = $characterId
            Source = $representative.FullName
            PhotoshopOutput = $photoshopOutput
            FinalOutput = $finalOutput
            Destinations = @($group.Group | ForEach-Object { $_.File.FullName })
            ResizeScale = if ($representative.FullName -match '[\\/]boot[\\/]') { 1 } else { $Scale }
            AlphaThreshold = if ($characterId -eq 'rin-tha3') { 32 } else { 8 }
        }
        $jobIndex++
    }
}

$jsxPath = Join-Path $workRoot 'photoshop-defringe.jsx'
$jsx = @(
    '#target photoshop',
    'app.displayDialogs = DialogModes.NO;',
    'var jobs = ['
)
foreach ($job in $allJobs) {
    $source = Convert-ToJsPath $job.Source
    $output = Convert-ToJsPath $job.PhotoshopOutput
    $jsx += ('{{ input: "{0}", output: "{1}" }},' -f $source, $output)
}
$jsx += @(
    '];',
    'for (var index = 0; index < jobs.length; index++) {',
    '    var documentRef = app.open(new File(jobs[index].input));',
    '    var descriptor = new ActionDescriptor();',
    '    descriptor.putInteger(charIDToTypeID("Wdth"), 1);',
    '    executeAction(charIDToTypeID("Dfrg"), descriptor, DialogModes.NO);',
    '    var options = new PNGSaveOptions();',
    '    options.compression = 6;',
    '    options.interlaced = false;',
    '    documentRef.saveAs(new File(jobs[index].output), options, true, Extension.LOWERCASE);',
    '    documentRef.close(SaveOptions.DONOTSAVECHANGES);',
    '}'
)
Set-Content -LiteralPath $jsxPath -Value ($jsx -join "`r`n") -Encoding UTF8

if (Get-Process -Name 'Photoshop' -ErrorAction SilentlyContinue) {
    throw 'Photoshop is already running. Close it before starting this isolated batch.'
}

Write-Host "Finishing $($allJobs.Count) deduplicated images in Photoshop."
$photoshop = $null
try {
    $photoshop = New-Object -ComObject Photoshop.Application
    $photoshop.DisplayDialogs = 3
    $photoshop.DoJavaScriptFile($jsxPath) | Out-Null
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

foreach ($job in $allJobs) {
    Convert-AlphaAwareImage `
        -InputPath $job.PhotoshopOutput `
        -OutputPath $job.FinalOutput `
        -AlphaThreshold $job.AlphaThreshold `
        -ResizeScale $job.ResizeScale
    foreach ($destination in $job.Destinations) {
        Copy-Item -LiteralPath $job.FinalOutput -Destination $destination -Force
    }
}

foreach ($characterId in $CharacterIds) {
    $files = @(Get-TargetFiles -CharacterId $characterId)
    $qaPath = Join-Path $qaRoot "$characterId-colored-background.png"
    New-QaSheet -CharacterId $characterId -Files $files -OutputPath $qaPath
    Write-Host "QA: $qaPath"
}

Write-Host "Original image backup: $backupRoot"
Write-Host 'Finished the 2D character images.'
