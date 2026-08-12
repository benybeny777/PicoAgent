param(
    [Parameter(Mandatory = $true)][string]$InputPath,
    [Parameter(Mandatory = $true)][string]$OutputPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (Get-Process -Name "Photoshop" -ErrorAction SilentlyContinue) {
    throw "Close Photoshop before running the isolated batch."
}

$inputFull = (Resolve-Path -LiteralPath $InputPath).Path
$outputFull = [IO.Path]::GetFullPath($OutputPath)
$outputDir = Split-Path -Parent $outputFull
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$dateStamp = Get-Date -Format "yyyyMMdd-HHmmss"
$workRoot = Join-Path $repositoryRoot (Join-Path "temp\photoshop-extract-character" $dateStamp)
New-Item -ItemType Directory -Force -Path $workRoot | Out-Null
$jsxPath = Join-Path $workRoot "extract-subject.jsx"

function Convert-ToJsPath([string]$Path) {
    return $Path.Replace([char]92, [char]47)
}

$jsx = @"
#target photoshop
app.displayDialogs = DialogModes.NO;
var source = app.open(new File("$(Convert-ToJsPath $inputFull)"));
try {
  var subjectDescriptor = new ActionDescriptor();
  subjectDescriptor.putBoolean(stringIDToTypeID("sampleAllLayers"), false);
  executeAction(stringIDToTypeID("autoCutout"), subjectDescriptor, DialogModes.NO);
  source.selection.copy();
  var result = app.documents.add(source.width, source.height, source.resolution, "character-transparent", NewDocumentMode.RGB, DocumentFill.TRANSPARENT);
  try {
    result.paste();
    var options = new PNGSaveOptions(); options.compression = 6; options.interlaced = false;
    result.saveAs(new File("$(Convert-ToJsPath $outputFull)"), options, true, Extension.LOWERCASE);
  } finally { result.close(SaveOptions.DONOTSAVECHANGES); }
} finally { source.close(SaveOptions.DONOTSAVECHANGES); }
"@
Set-Content -LiteralPath $jsxPath -Value $jsx -Encoding UTF8

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

if (-not (Test-Path -LiteralPath $outputFull -PathType Leaf)) {
    throw "Transparent PNG was not generated: $outputFull"
}
Write-Output "[photoshop-extract-character] $outputFull"
