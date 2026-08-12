param(
    [Parameter(Mandatory = $true)]
    [string[]]$CharacterIds
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$python = @('run', '--with', 'pillow', 'python')

function Invoke-Checked([string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Command が終了コード $LASTEXITCODE で失敗しました。" }
}

Push-Location $repositoryRoot
try {
    # 順序を変えない。差分合成をPhotoshop後に行うと高精細レイヤーを低解像度で上書きする。
    Invoke-Checked 'uv' ($python + @('scripts/build-sharp-expression-layers.py', '--prepare-photoshop') + $CharacterIds)
    Invoke-Checked 'uv' ($python + @('scripts/build-extended-expression-layers.py', '--prepare-photoshop') + $CharacterIds)
    & (Join-Path $PSScriptRoot 'finish-sharp-expression-assets.ps1') -CharacterIds $CharacterIds -Scale 2
    foreach ($characterId in $CharacterIds) {
        Invoke-Checked 'uv' ($python + @('scripts/build-character-animations.py', '--character-dir', "characters/$characterId"))
    }
    Invoke-Checked 'cargo' @('xtask', 'sync-characters')
}
finally {
    Pop-Location
}
