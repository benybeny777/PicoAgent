$ErrorActionPreference = 'Stop'

$jobDir = $null
for ($i = 0; $i -lt $args.Count; $i++) {
    if ($args[$i] -eq '--job-dir' -and $i + 1 -lt $args.Count) {
        $jobDir = $args[$i + 1]
        break
    }
}
if (-not $jobDir) { throw '--job-dir が必要です' }

$deadline = [DateTime]::UtcNow.AddSeconds(60)
$goPath = Join-Path $jobDir 'go.signal'
while (-not (Test-Path -LiteralPath $goPath)) {
    if ([DateTime]::UtcNow -ge $deadline) { exit 125 }
    Start-Sleep -Milliseconds 100
}

Add-Type -AssemblyName System.Security.Cryptography.ProtectedData
$protected = [IO.File]::ReadAllBytes((Join-Path $jobDir 'request.bin'))
$plain = [Security.Cryptography.ProtectedData]::Unprotect(
    $protected,
    $null,
    [Security.Cryptography.DataProtectionScope]::CurrentUser
)
$request = [Text.Encoding]::UTF8.GetString($plain) | ConvertFrom-Json
[Array]::Clear($plain, 0, $plain.Length)

Set-Location -LiteralPath $request.cwd
try {
    & $request.executable @($request.arguments)
    if ($null -eq $LASTEXITCODE) { exit 0 }
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
