$ErrorActionPreference = 'Stop'

[Console]::OutputEncoding = [Text.UTF8Encoding]::new()
Write-Host 'PowerShell 7（pwsh）が見つかりません。' -ForegroundColor Red
Write-Host 'PicoAgentの開発用入口にはPowerShell 7が必要です。'
Write-Host 'https://aka.ms/powershell から導入して、もう一度実行してください。'
[void](Read-Host 'Enterを押して閉じる')
exit 1
