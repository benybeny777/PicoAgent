param(
    [Parameter(Mandatory = $true)]
    [string]$Tha3Root,
    [Parameter(Mandatory = $true)]
    [string]$UvPath
)

$ErrorActionPreference = "Stop"
$runtime = Join-Path $Tha3Root "runtime"
$stage = Join-Path $Tha3Root ("runtime-installing-" + $PID)
$cache = Join-Path $env:TEMP "picoagent-tha3-uv-cache"
$pythonInstall = Join-Path $stage "python"
$venv = Join-Path $stage "venv"
$python = Join-Path $venv "Scripts\python.exe"
$torch = Join-Path $venv "Lib\site-packages\torch\__init__.py"
$runtimeVersion = "python3.11-torch2.10.0-cu128-v1"

function Test-RuntimeReady([string]$Path) {
    $marker = Join-Path $Path "picoagent-runtime-version.txt"
    return (Test-Path -LiteralPath (Join-Path $Path "venv\Scripts\python.exe")) -and
        (Test-Path -LiteralPath (Join-Path $Path "venv\Lib\site-packages\torch\__init__.py")) -and
        (Test-Path -LiteralPath $marker) -and
        ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $runtimeVersion)
}

if (Test-RuntimeReady $runtime) {
    Write-Output "THA3 runtime is ready."
    exit 0
}

try {
    Get-ChildItem -LiteralPath $Tha3Root -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "runtime-installing-*" -or $_.Name -like "runtime-old-*" } |
        Remove-Item -Recurse -Force
    if (Test-Path -LiteralPath $stage) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    $env:UV_CACHE_DIR = $cache
    $env:UV_PYTHON_INSTALL_DIR = $pythonInstall

    & $UvPath python install 3.11 --install-dir $pythonInstall --no-bin --no-registry
    if ($LASTEXITCODE -ne 0) { throw "Failed to install the bundled Python." }
    & $UvPath venv --python 3.11 --managed-python --relocatable $venv
    if ($LASTEXITCODE -ne 0) { throw "Failed to create the bundled Python environment." }
    & $UvPath pip install --python $python --torch-backend cu128 `
        "torch==2.10.0" "numpy==2.2.6" "pillow==11.3.0" "matplotlib==3.10.5"
    if ($LASTEXITCODE -ne 0) { throw "Failed to install CUDA PyTorch." }
    if (-not (Test-Path -LiteralPath $torch)) { throw "THA3 runtime validation failed." }
    Set-Content -LiteralPath (Join-Path $stage "picoagent-runtime-version.txt") `
        -Value $runtimeVersion -Encoding ASCII -NoNewline

    $old = Join-Path $Tha3Root ("runtime-old-" + $PID)
    if (Test-Path -LiteralPath $runtime) {
        Move-Item -LiteralPath $runtime -Destination $old
    }
    Move-Item -LiteralPath $stage -Destination $runtime
    if (Test-Path -LiteralPath $old) {
        try {
            Remove-Item -LiteralPath $old -Recurse -Force
        } catch {
            Write-Warning "The previous THA3 runtime could not be removed: $old"
        }
    }
    Write-Output "THA3 runtime installation completed."
} finally {
    if (Test-Path -LiteralPath $stage) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    if (Test-Path -LiteralPath $cache) {
        Remove-Item -LiteralPath $cache -Recurse -Force
    }
}
