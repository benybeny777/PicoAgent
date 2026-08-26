$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$titleIndex = [Array]::IndexOf($args, '--title')
$messageIndex = [Array]::IndexOf($args, '--message')
if ($titleIndex -lt 0 -or $titleIndex + 1 -ge $args.Count -or $messageIndex -lt 0 -or $messageIndex + 1 -ge $args.Count) {
    throw '--titleと--messageが必要です'
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$icon = [Windows.Forms.NotifyIcon]::new()
try {
    $icon.Icon = [Drawing.SystemIcons]::Information
    $icon.Visible = $true
    $icon.BalloonTipIcon = [Windows.Forms.ToolTipIcon]::Info
    $icon.BalloonTipTitle = [string]$args[$titleIndex + 1]
    $icon.BalloonTipText = [string]$args[$messageIndex + 1]
    $icon.ShowBalloonTip(5000)
    Start-Sleep -Seconds 6
} finally {
    $icon.Visible = $false
    $icon.Dispose()
}
