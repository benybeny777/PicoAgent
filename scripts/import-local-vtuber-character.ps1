param(
    [Parameter(Mandatory = $true)][string]$SourceCharacterRoot,
    [Parameter(Mandatory = $true)][string]$CharacterId
)

$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$source = [IO.Path]::GetFullPath($SourceCharacterRoot)
$characters = Join-Path $repo 'characters'
$target = [IO.Path]::GetFullPath((Join-Path $characters $CharacterId))
if (-not $target.StartsWith($characters + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "キャラクターIDが不正です: $CharacterId"
}
if (-not (Test-Path -LiteralPath (Join-Path $target 'character.json'))) {
    throw "更新先のキャラクターがありません: $target"
}

$current = Get-Content -LiteralPath (Join-Path $source 'rig-current.json') -Raw | ConvertFrom-Json
$generation = [string]$current.generation
if ($generation -notmatch '^g_[0-9a-f]{32}$') { throw '生成IDが不正です' }
$generated = Join-Path (Join-Path $source 'rig-generations') $generation
$rigSource = Join-Path $generated 'rig.json'
$partsSource = Join-Path $generated 'parts'
$previewSource = Join-Path (Join-Path $source 'source') 'input.png'
foreach ($required in @($rigSource, $partsSource, $previewSource)) {
    if (-not (Test-Path -LiteralPath $required)) { throw "生成物が不足しています: $required" }
}

$rig = Get-Content -LiteralPath $rigSource -Raw | ConvertFrom-Json
if ($rig.profile -ne 'lvs-anime25d-v1' -or [int]$rig.schema_version -ne 3) {
    throw '対応していないLocalVTuberStudioリグです'
}
foreach ($layer in $rig.layers.PSObject.Properties) {
    $url = [string]$layer.Value.url
    if ($url -notmatch '^/assets/rig2d/parts/([^/]+\.png)$') { throw "リグ内の素材URLが不正です: $url" }
    if (-not (Test-Path -LiteralPath (Join-Path $partsSource $Matches[1]))) { throw "リグ参照素材がありません: $url" }
}

$incoming = Join-Path $target 'rig2d.incoming'
if (Test-Path -LiteralPath $incoming) { Remove-Item -LiteralPath $incoming -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $incoming 'parts') | Out-Null
Copy-Item -LiteralPath $rigSource -Destination (Join-Path $incoming 'rig.json')
Copy-Item -Path (Join-Path $partsSource '*.png') -Destination (Join-Path $incoming 'parts')

$rigTarget = Join-Path $target 'rig2d'
if (Test-Path -LiteralPath $rigTarget) { Remove-Item -LiteralPath $rigTarget -Recurse -Force }
Move-Item -LiteralPath $incoming -Destination $rigTarget
Copy-Item -LiteralPath $previewSource -Destination (Join-Path $target 'preview.png') -Force

$manifestPath = Join-Path $target 'character.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
foreach ($obsolete in @('animations', 'animationQualityMode', 'frameGeneration', 'runtimeLayers', 'runtimeLayerEncoding', 'frameEncoding', 'boot')) {
    $manifest.PSObject.Properties.Remove($obsolete)
}
$manifest | Add-Member -NotePropertyName sourceFraming -NotePropertyValue 'native-full' -Force
$manifest | Add-Member -NotePropertyName meshRig -NotePropertyValue ([ordered]@{ rig = 'rig2d/rig.json' }) -Force
$hash = (Get-FileHash -LiteralPath (Join-Path $rigTarget 'rig.json') -Algorithm SHA256).Hash.ToLowerInvariant().Substring(0, 16)
$manifest | Add-Member -NotePropertyName assetVersion -NotePropertyValue "lvs-schema3-$hash" -Force
$temporary = "$manifestPath.incoming"
$manifest | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $temporary -Encoding utf8
Move-Item -LiteralPath $temporary -Destination $manifestPath -Force

$loaded = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($loaded.id -ne $CharacterId -or $loaded.meshRig.rig -ne 'rig2d/rig.json') {
    throw '取込後のcharacter.jsonを検証できません'
}
Write-Output "$CharacterId <- $generation ($hash)"
