function Get-NotificationIntervalSeconds([object]$Record) {
    if ($Record.notification.PSObject.Properties['interval_seconds_override'] -and $Record.notification.interval_seconds_override) {
        return [Math]::Max(1, [double]$Record.notification.interval_seconds_override)
    }
    return [Math]::Max(60, [double]$Record.notification.interval_minutes * 60)
}

function Show-WindowsNotification([string]$Title, [string]$Message) {
    if ($env:WINDOWS_PROCESS_JOBS_DISABLE_OS_NOTIFICATION -eq '1') { return $false }
    try {
        [void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
        [void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
        $xml = [Windows.Data.Xml.Dom.XmlDocument]::new()
        $safeTitle = [Security.SecurityElement]::Escape($Title)
        $safeMessage = [Security.SecurityElement]::Escape($Message)
        $xml.LoadXml("<toast><visual><binding template='ToastGeneric'><text>$safeTitle</text><text>$safeMessage</text></binding></visual></toast>")
        $toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
        [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Windows Process Jobs').Show($toast)
        return $true
    } catch {
        return $false
    }
}

function Test-CodexQueueAvailable {
    if ($env:WINDOWS_PROCESS_JOBS_DISABLE_CODEX_QUEUE -eq '1') { return $false }
    if ($script:codexQueueAvailable -ne $null) { return $script:codexQueueAvailable }
    $script:codexQueueAvailable = $false
    $command = Get-Command codex -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $command) { return $false }
    try {
        $versionText = (& $command.Source --version 2>$null | Select-Object -First 1)
        if ($versionText -match '(\d+)\.(\d+)\.(\d+)' -and ([Version]$Matches[0]) -ge [Version]'0.149.0') {
            $script:codexQueuePath = $command.Source
            $script:codexQueueAvailable = $true
        }
    } catch { }
    return $script:codexQueueAvailable
}

function Send-CodexQueue([string]$ThreadId, [string]$Message) {
    if (-not $ThreadId -or -not (Test-CodexQueueAvailable)) { return $false }
    try {
        $start = [Diagnostics.ProcessStartInfo]::new()
        $start.FileName = $script:codexQueuePath
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        foreach ($value in @('queue', '--thread', $ThreadId, '--message', $Message)) { [void]$start.ArgumentList.Add($value) }
        $process = [Diagnostics.Process]::Start($start)
        if (-not $process.WaitForExit(30000)) {
            try { $process.Kill($true) } catch { }
            return $false
        }
        return $process.ExitCode -eq 0
    } catch {
        return $false
    }
}

function Publish-Notification([object]$Record, [string]$JobDirectory, [string]$Kind) {
    if (-not $Record.notification -or -not $Record.notification.enabled) {
        return [ordered]@{ success = $true; event_saved = $false; os_sent = $false; queue_sent = $false; error = $null }
    }
    $timing = Get-Timing $Record
    $message = if ($Kind -eq 'progress') {
        "Windows Process Jobs: $($Record.id) はバックグラウンドで実行中です。経過$($timing.elapsed)、残り目安$($timing.remaining_estimate)。"
    } else {
        $outcome = switch ($Record.status) {
            'succeeded' { '完了しました' }
            'cancelled' { '取り消されました' }
            default { "失敗しました（終了コード$($Record.exit_code)）" }
        }
        "Windows Process Jobs: $($Record.id) は$outcome。経過$($timing.elapsed)。"
    }
    try {
        $eventDirectory = Join-Path $JobDirectory 'events'
        New-Item -ItemType Directory -Force -Path $eventDirectory | Out-Null
        $event = [ordered]@{
            schema_version = 1
            id = $Record.id
            owner_thread_id = $Record.notification.owner_thread_id
            kind = $Kind
            status = $Record.status
            elapsed = $timing.elapsed
            remaining_estimate = $timing.remaining_estimate
            exit_code = $Record.exit_code
            created_at_utc = [DateTime]::UtcNow.ToString('o')
            message = $message
        }
        $eventPath = Join-Path $eventDirectory ("$([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fff'))-$([Guid]::NewGuid().ToString('N').Substring(0, 8)).json")
        Write-JsonAtomic $eventPath $event
        $osSent = Show-WindowsNotification 'Windows Process Jobs' $message
        $queueSent = Send-CodexQueue $Record.notification.owner_thread_id $message
        return [ordered]@{ success = $true; event_saved = $true; os_sent = $osSent; queue_sent = $queueSent; error = $null }
    } catch {
        return [ordered]@{ success = $false; event_saved = $false; os_sent = $false; queue_sent = $false; error = $_.Exception.Message }
    }
}

function Invoke-ProgressNotification([object]$Record, [string]$JobDirectory, [string]$RecordPath) {
    if (-not $Record.PSObject.Properties['notification'] -or -not $Record.notification -or -not $Record.notification.enabled) { return }
    $lastValue = if ($Record.notification.last_progress_at_utc) { $Record.notification.last_progress_at_utc } else { $Record.started_at_utc }
    if (-not $lastValue) { return }
    $elapsed = ([DateTime]::UtcNow - ([DateTimeOffset]$lastValue).UtcDateTime).TotalSeconds
    if ($elapsed -lt (Get-NotificationIntervalSeconds $Record)) { return }
    [void](Update-EstimateIfNeeded $Record)
    $published = Publish-Notification $Record $JobDirectory 'progress'
    $Record.notification.last_error = $published.error
    $Record.notification.last_delivery = $published
    $Record.notification.progress_count = [int]$Record.notification.progress_count + 1
    $Record.notification.last_progress_at_utc = [DateTime]::UtcNow.ToString('o')
    Write-JsonAtomic $RecordPath $Record
}
