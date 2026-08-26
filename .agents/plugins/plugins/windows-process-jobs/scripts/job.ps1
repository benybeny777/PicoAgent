$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$pluginRoot = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$scriptRoot = Join-Path $pluginRoot 'scripts'
$stateRoot = if ($env:WINDOWS_PROCESS_JOBS_HOME) {
    [IO.Path]::GetFullPath($env:WINDOWS_PROCESS_JOBS_HOME)
} else {
    Join-Path $env:LOCALAPPDATA 'Codex\windows-process-jobs'
}
$maxLogBytes = 10MB
$script:codexQueueAvailable = $null
$script:codexQueuePath = $null
. (Join-Path $scriptRoot 'notifications.ps1')

function Write-JsonAtomic([string]$Path, [object]$Value) {
    $tempPath = "$Path.$PID.tmp"
    [IO.File]::WriteAllText(
        $tempPath,
        ($Value | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::Move($tempPath, $Path, $true)
}

function Read-Json([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "記録がありません: $Path"
    }
    return Get-Content -Raw -Encoding UTF8 -LiteralPath $Path | ConvertFrom-Json
}

function Get-OptionalValue([object]$Value, [string]$Name, [object]$Default = $null) {
    if ($Value -is [Collections.IDictionary] -and $Value.Contains($Name)) { return $Value[$Name] }
    if ($Value -and $Value.PSObject.Properties[$Name]) { return $Value.$Name }
    return $Default
}

function Protect-Request([object]$Request, [string]$Path) {
    Add-Type -AssemblyName System.Security.Cryptography.ProtectedData
    $plain = [Text.Encoding]::UTF8.GetBytes(($Request | ConvertTo-Json -Depth 8 -Compress))
    try {
        $protected = [Security.Cryptography.ProtectedData]::Protect(
            $plain,
            $null,
            [Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        [IO.File]::WriteAllBytes($Path, $protected)
    } finally {
        [Array]::Clear($plain, 0, $plain.Length)
    }
}

function Unprotect-Request([string]$Path) {
    Add-Type -AssemblyName System.Security.Cryptography.ProtectedData
    $protected = [IO.File]::ReadAllBytes($Path)
    $plain = [Security.Cryptography.ProtectedData]::Unprotect(
        $protected,
        $null,
        [Security.Cryptography.DataProtectionScope]::CurrentUser
    )
    try {
        return [Text.Encoding]::UTF8.GetString($plain) | ConvertFrom-Json
    } finally {
        [Array]::Clear($plain, 0, $plain.Length)
    }
}

function Get-RedactedArguments([string[]]$Arguments) {
    $result = [Collections.Generic.List[string]]::new()
    $hideNext = $false
    foreach ($argument in $Arguments) {
        if ($hideNext) {
            $result.Add('[REDACTED]')
            $hideNext = $false
            continue
        }
        if ($argument -match '^(--?[^=]*(token|password|secret|api[-_]?key)[^=]*)=(.*)$') {
            $result.Add("$($Matches[1])=[REDACTED]")
            continue
        }
        $result.Add($argument)
        if ($argument -match '^--?[^=]*(token|password|secret|api[-_]?key)') {
            $hideNext = $true
        }
    }
    return $result.ToArray()
}

function Get-PwshPath {
    $command = Get-Command pwsh -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $command) {
        throw 'PowerShell 7 (pwsh) が必要です。Windows PowerShell 5.1へは降格しません。'
    }
    return $command.Source
}

function Test-ProcessIdentity([object]$Identity) {
    if (-not $Identity -or -not $Identity.pid -or -not $Identity.started_at_utc) { return $false }
    $process = Get-Process -Id ([int]$Identity.pid) -ErrorAction SilentlyContinue
    if (-not $process) { return $false }
    try {
        $actualStart = $process.StartTime.ToUniversalTime()
        $expectedStart = ([DateTimeOffset]$Identity.started_at_utc).UtcDateTime
        $actualPath = $process.Path
        return [Math]::Abs(($actualStart - $expectedStart).TotalMilliseconds) -lt 100 -and (
            -not $Identity.executable -or
            [string]::Equals(
                [IO.Path]::GetFullPath($actualPath),
                [IO.Path]::GetFullPath([string]$Identity.executable),
                [StringComparison]::OrdinalIgnoreCase
            )
        )
    } catch {
        return $false
    }
}

function Get-JobDirectory([string]$JobId) {
    if ($JobId -notmatch '^[a-zA-Z0-9._-]+$') { throw '不正なジョブIDです' }
    return Join-Path $stateRoot $JobId
}

function Read-Record([string]$JobId) {
    return Read-Json (Join-Path (Get-JobDirectory $JobId) 'record.json')
}

function Resolve-Executable([string]$Value) {
    if ([IO.Path]::IsPathRooted($Value)) {
        $resolved = [IO.Path]::GetFullPath($Value)
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { throw "実行ファイルがありません: $resolved" }
        return $resolved
    }
    $command = Get-Command $Value -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $command) { throw "実行ファイルを解決できません: $Value" }
    return $command.Source
}

function Start-Controller([string]$JobDirectory) {
    $pwsh = Get-PwshPath
    $escapedPwsh = $pwsh.Replace('"', '\"')
    $escapedScript = $PSCommandPath.Replace('"', '\"')
    $escapedDir = $JobDirectory.Replace('"', '\"')
    $commandLine = "`"$escapedPwsh`" -NoProfile -File `"$escapedScript`" worker --job-dir `"$escapedDir`""
    $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
        CommandLine = $commandLine
    }
    if ($created.ReturnValue -ne 0 -or -not $created.ProcessId) {
        throw "管理プロセスを開始できません: Win32_Process.Create=$($created.ReturnValue)"
    }
    return [int]$created.ProcessId
}

function New-Job([object]$Request, [string]$Name, [bool]$Critical, [object]$EstimatedMinutes) {
    New-Item -ItemType Directory -Force -Path $stateRoot | Out-Null
    $id = '{0}-{1}' -f ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')), ([Guid]::NewGuid().ToString('N').Substring(0, 8))
    $jobDir = Join-Path $stateRoot $id
    New-Item -ItemType Directory -Path $jobDir | Out-Null
    Protect-Request $Request (Join-Path $jobDir 'request.bin')
    $record = [ordered]@{
        schema_version = 1
        id = $id
        name = $Name
        status = 'starting'
        critical = $Critical
        created_at_utc = [DateTime]::UtcNow.ToString('o')
        started_at_utc = $null
        finished_at_utc = $null
        controller = $null
        process = $null
        command = [ordered]@{
            executable = $Request.executable
            arguments = @(Get-RedactedArguments @($Request.arguments))
            cwd = $Request.cwd
        }
        exit_code = $null
        error = $null
        estimate = [ordered]@{
            estimated_total_minutes = $EstimatedMinutes
            updated_count = 0
            last_updated_at_utc = $null
        }
        notification = [ordered]@{
            enabled = [bool](Get-OptionalValue $Request 'notify' $true)
            interval_minutes = Get-OptionalValue $Request 'notify_interval_minutes' 10.0
            owner_thread_id = Get-OptionalValue $Request 'owner_thread_id'
            interval_seconds_override = Get-OptionalValue $Request 'notification_interval_seconds_override'
            os_disabled = [bool](Get-OptionalValue $Request 'notification_os_disabled' $false)
            queue_disabled = [bool](Get-OptionalValue $Request 'notification_queue_disabled' $false)
            progress_count = 0
            last_progress_at_utc = $null
            completion_sent_at_utc = $null
            last_error = $null
            last_delivery = $null
        }
        logs = [ordered]@{ stdout = 'stdout.log'; stderr = 'stderr.log'; max_mb = 10 }
    }
    Write-JsonAtomic (Join-Path $jobDir 'record.json') $record
    try {
        $controllerPid = Start-Controller $jobDir
        $record.controller = [ordered]@{ pid = $controllerPid; started_at_utc = $null; executable = (Get-PwshPath) }
        Write-JsonAtomic (Join-Path $jobDir 'record.json') $record
    } catch {
        $record.status = 'failed'
        $record.finished_at_utc = [DateTime]::UtcNow.ToString('o')
        $record.error = $_.Exception.Message
        Write-JsonAtomic (Join-Path $jobDir 'record.json') $record
        throw
    }
    return $record
}

function Parse-Start([string[]]$Arguments) {
    $name = $null
    $cwd = (Get-Location).Path
    $critical = $false
    $estimatedMinutes = $null
    $notify = $true
    $notifyIntervalMinutes = 10.0
    $separator = [Array]::IndexOf($Arguments, '--')
    if ($separator -lt 0 -or $separator -eq $Arguments.Count - 1) {
        throw 'startは「-- 実行ファイル 引数...」を必要とします'
    }
    for ($i = 0; $i -lt $separator; $i++) {
        switch ($Arguments[$i]) {
            '--name' { if (++$i -ge $separator) { throw '--nameの値がありません' }; $name = $Arguments[$i] }
            '--cwd' { if (++$i -ge $separator) { throw '--cwdの値がありません' }; $cwd = $Arguments[$i] }
            '--critical' { $critical = $true }
            '--estimate-minutes' {
                if (++$i -ge $separator) { throw '--estimate-minutesの値がありません' }
                $minutes = [double]$Arguments[$i]
                if ($minutes -le 0) { throw '--estimate-minutesは0より大きい値が必要です' }
                $estimatedMinutes = [Math]::Round($minutes, 1)
            }
            '--notify-interval-minutes' {
                if (++$i -ge $separator) { throw '--notify-interval-minutesの値がありません' }
                $minutes = [double]$Arguments[$i]
                if ($minutes -lt 1) { throw '--notify-interval-minutesは1以上が必要です' }
                $notifyIntervalMinutes = [Math]::Round($minutes, 1)
            }
            '--no-notify' { $notify = $false }
            '--json' { }
            default { throw "不明なstartオプションです: $($Arguments[$i])" }
        }
    }
    $resolvedCwd = (Resolve-Path -LiteralPath $cwd).Path
    $payload = @($Arguments[($separator + 1)..($Arguments.Count - 1)])
    $executable = Resolve-Executable $payload[0]
    $ownerThreadId = @(
        $env:WINDOWS_PROCESS_JOBS_OWNER_THREAD_ID
        $env:CODEX_THREAD_ID
        $env:CODEX_SESSION_ID
        $env:CLAUDE_SESSION_ID
        $env:CLAUDE_CODE_SESSION_ID
    ) | Where-Object { $_ -match '^[a-zA-Z0-9._-]{8,160}$' } | Select-Object -First 1
    $request = [ordered]@{
        executable = $executable
        arguments = @($payload | Select-Object -Skip 1)
        cwd = $resolvedCwd
        notify = $notify
        notify_interval_minutes = $notifyIntervalMinutes
        owner_thread_id = $ownerThreadId
        notification_interval_seconds_override = if ($env:WINDOWS_PROCESS_JOBS_TEST_INTERVAL_SECONDS) {
            [Math]::Max(1, [double]$env:WINDOWS_PROCESS_JOBS_TEST_INTERVAL_SECONDS)
        } else { $null }
        notification_os_disabled = $env:WINDOWS_PROCESS_JOBS_DISABLE_OS_NOTIFICATION -eq '1'
        notification_queue_disabled = $env:WINDOWS_PROCESS_JOBS_DISABLE_CODEX_QUEUE -eq '1'
    }
    if (-not $name) { $name = [IO.Path]::GetFileNameWithoutExtension($executable) }
    return New-Job $request $name $critical $estimatedMinutes
}

function Format-Minutes([double]$Minutes, [bool]$Approximate = $true) {
    if ($Minutes -lt 1) { return '1分未満' }
    $rounded = [int][Math]::Ceiling($Minutes)
    if ($Approximate) { return "約${rounded}分" }
    return "${rounded}分"
}

function Get-Timing([object]$Record) {
    $startValue = if ($Record.started_at_utc) { $Record.started_at_utc } else { $Record.created_at_utc }
    $start = ([DateTimeOffset]$startValue).UtcDateTime
    $end = if ($Record.finished_at_utc) { ([DateTimeOffset]$Record.finished_at_utc).UtcDateTime } else { [DateTime]::UtcNow }
    $elapsedMinutes = [Math]::Max(0, ($end - $start).TotalMinutes)
    $estimatedMinutes = $null
    if ($Record.PSObject.Properties['estimate'] -and $Record.estimate -and $Record.estimate.estimated_total_minutes) {
        $estimatedMinutes = [double]$Record.estimate.estimated_total_minutes
    }
    $remainingMinutes = if ($null -eq $estimatedMinutes) { $null } else { [Math]::Max(0, $estimatedMinutes - $elapsedMinutes) }
    $overrunMinutes = if ($null -eq $estimatedMinutes) { $null } else { [Math]::Max(0, $elapsedMinutes - $estimatedMinutes) }
    $updatedCount = if ($Record.PSObject.Properties['estimate'] -and $Record.estimate -and $Record.estimate.PSObject.Properties['updated_count']) {
        [int]$Record.estimate.updated_count
    } else { 0 }
    return [ordered]@{
        elapsed = Format-Minutes $elapsedMinutes $false
        remaining_estimate = if ($null -eq $estimatedMinutes) {
            '不明'
        } elseif ($Record.status -notin @('starting', 'running')) {
            '完了'
        } elseif ($overrunMinutes -ge 1) {
            "見積りを$(Format-Minutes $overrunMinutes)超過"
        } else {
            "$(Format-Minutes $remainingMinutes)$(if ($updatedCount -gt 0) { '（更新済み）' } else { '' })"
        }
    }
}

function Update-EstimateIfNeeded([object]$Record) {
    if ($Record.status -notin @('starting', 'running')) { return $false }
    if (-not $Record.PSObject.Properties['estimate'] -or -not $Record.estimate -or -not $Record.estimate.estimated_total_minutes) {
        return $false
    }
    $startValue = if ($Record.started_at_utc) { $Record.started_at_utc } else { $Record.created_at_utc }
    $elapsedMinutes = ([DateTime]::UtcNow - ([DateTimeOffset]$startValue).UtcDateTime).TotalMinutes
    $oldTotal = [double]$Record.estimate.estimated_total_minutes
    if ($elapsedMinutes -lt $oldTotal) { return $false }
    $extension = [Math]::Max(5, $oldTotal * 0.25)
    $newTotal = [Math]::Ceiling([Math]::Max($oldTotal * 1.5, $elapsedMinutes + $extension))
    $Record.estimate.estimated_total_minutes = $newTotal
    if (-not $Record.estimate.PSObject.Properties['updated_count']) {
        $Record.estimate | Add-Member -NotePropertyName updated_count -NotePropertyValue 0
    }
    if (-not $Record.estimate.PSObject.Properties['last_updated_at_utc']) {
        $Record.estimate | Add-Member -NotePropertyName last_updated_at_utc -NotePropertyValue $null
    }
    $Record.estimate.updated_count = [int]$Record.estimate.updated_count + 1
    $Record.estimate.last_updated_at_utc = [DateTime]::UtcNow.ToString('o')
    return $true
}

function Get-Tail([string]$Path, [int]$Lines) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return @() }
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        $count = [int][Math]::Min($stream.Length, 256KB)
        [void]$stream.Seek(-$count, [IO.SeekOrigin]::End)
        $buffer = [byte[]]::new($count)
        $read = $stream.Read($buffer, 0, $count)
        $text = [Text.Encoding]::UTF8.GetString($buffer, 0, $read)
        $parts = @($text -split "\r?\n")
        return @($parts | Select-Object -Last $Lines)
    } finally {
        $stream.Dispose()
    }
}

function Invoke-Worker([string]$JobDirectory) {
    $recordPath = Join-Path $JobDirectory 'record.json'
    $record = Read-Json $recordPath
    $self = Get-Process -Id $PID
    $record.controller = [ordered]@{
        pid = $PID
        started_at_utc = $self.StartTime.ToUniversalTime().ToString('o')
        executable = $self.Path
    }
    try {
        $request = Unprotect-Request (Join-Path $JobDirectory 'request.bin')
        Add-Type -Path (Join-Path $scriptRoot 'WindowsJobRuntime.cs')
        $managed = [ManagedWindowsJob]::Start(
            (Get-PwshPath),
            (Join-Path $scriptRoot 'payload.ps1'),
            $JobDirectory,
            (Join-Path $JobDirectory 'stdout.log'),
            (Join-Path $JobDirectory 'stderr.log'),
            $maxLogBytes
        )
        try {
            $record.status = 'running'
            $record.started_at_utc = [DateTime]::UtcNow.ToString('o')
            $record.process = [ordered]@{
                pid = $managed.ProcessId
                started_at_utc = $managed.ProcessStartTimeUtc
                executable = (Get-PwshPath)
            }
            Write-JsonAtomic $recordPath $record
            $cancelled = $false
            while (-not $managed.WaitForExit(250)) {
                Invoke-ProgressNotification $record $JobDirectory $recordPath
                if (Test-Path -LiteralPath (Join-Path $JobDirectory 'cancel.request') -PathType Leaf) {
                    $cancelled = $true
                    $managed.RequestCancel(5000)
                    break
                }
            }
            Invoke-ProgressNotification $record $JobDirectory $recordPath
            $outcome = $managed.Finish($cancelled)
            $record.exit_code = $outcome.ExitCode
            $record.status = if ($outcome.Cancelled) { 'cancelled' } elseif ($outcome.ExitCode -eq 0) { 'succeeded' } else { 'failed' }
        } finally {
            $managed.Dispose()
        }
    } catch {
        $record.status = 'failed'
        $record.error = $_.Exception.ToString()
    } finally {
        $record.finished_at_utc = [DateTime]::UtcNow.ToString('o')
        Write-JsonAtomic $recordPath $record
        if ($record.PSObject.Properties['notification'] -and $record.notification -and $record.notification.enabled -and -not $record.notification.completion_sent_at_utc) {
            $published = Publish-Notification $record $JobDirectory 'completion'
            $record.notification.last_error = $published.error
            $record.notification.last_delivery = $published
            $record.notification.completion_sent_at_utc = [DateTime]::UtcNow.ToString('o')
            Write-JsonAtomic $recordPath $record
        }
    }
}

function Get-StatusRecord([string]$JobId) {
    $record = Read-Record $JobId
    if ($record.status -in @('starting', 'running') -and $record.controller.started_at_utc) {
        if (-not (Test-ProcessIdentity $record.controller)) {
            $record.status = 'failed'
            $record.finished_at_utc = [DateTime]::UtcNow.ToString('o')
            $record.error = '管理プロセスの識別情報が一致せず、ジョブを実行中と確認できません'
            Write-JsonAtomic (Join-Path (Get-JobDirectory $JobId) 'record.json') $record
        }
    }
    if (Update-EstimateIfNeeded $record) {
        Write-JsonAtomic (Join-Path (Get-JobDirectory $JobId) 'record.json') $record
    }
    return $record
}

function Invoke-Status([string[]]$Arguments) {
    $jobId = @($Arguments | Where-Object { $_ -notlike '--*' } | Select-Object -First 1)
    if ($jobId.Count -gt 0) {
        $record = Get-StatusRecord $jobId[0]
        $jobDir = Get-JobDirectory $jobId[0]
        return [ordered]@{
            record = $record
            timing = Get-Timing $record
            stdout_tail = @(Get-Tail (Join-Path $jobDir 'stdout.log') 4)
            stderr_tail = @(Get-Tail (Join-Path $jobDir 'stderr.log') 4)
        }
    }
    if (-not (Test-Path -LiteralPath $stateRoot)) { return @() }
    return @(Get-ChildItem -LiteralPath $stateRoot -Directory | ForEach-Object {
        try {
            $recent = Read-Json (Join-Path $_.FullName 'record.json')
            [ordered]@{ record = $recent; timing = Get-Timing $recent }
        } catch { }
    } | Sort-Object created_at_utc -Descending | Select-Object -First 20)
}

function Invoke-Tail([string[]]$Arguments) {
    if ($Arguments.Count -lt 1) { throw 'tailはジョブIDを必要とします' }
    $lines = 40
    for ($i = 1; $i -lt $Arguments.Count; $i++) {
        if ($Arguments[$i] -eq '--lines' -and ++$i -lt $Arguments.Count) { $lines = [Math]::Clamp([int]$Arguments[$i], 1, 1000) }
    }
    $jobDir = Get-JobDirectory $Arguments[0]
    return [ordered]@{
        id = $Arguments[0]
        stdout = @(Get-Tail (Join-Path $jobDir 'stdout.log') $lines)
        stderr = @(Get-Tail (Join-Path $jobDir 'stderr.log') $lines)
    }
}

function Invoke-Result([string[]]$Arguments) {
    if ($Arguments.Count -lt 1) { throw 'resultはジョブIDを必要とします' }
    $record = Get-StatusRecord $Arguments[0]
    if ($record.status -in @('starting', 'running')) { throw "ジョブはまだ終了していません: $($record.status)" }
    $jobDir = Get-JobDirectory $Arguments[0]
    return [ordered]@{
        record = $record
        timing = Get-Timing $record
        stdout_tail = @(Get-Tail (Join-Path $jobDir 'stdout.log') 200)
        stderr_tail = @(Get-Tail (Join-Path $jobDir 'stderr.log') 200)
    }
}

function Invoke-Cancel([string[]]$Arguments) {
    if ($Arguments.Count -lt 1) { throw 'cancelはジョブIDを必要とします' }
    $record = Get-StatusRecord $Arguments[0]
    if ($record.status -notin @('starting', 'running')) { return $record }
    if ($record.critical -and $Arguments -notcontains '--force') {
        throw 'criticalジョブの取消には--forceが必要です'
    }
    if (-not (Test-ProcessIdentity $record.controller)) {
        throw '管理プロセスのPID識別に失敗したため取消しません'
    }
    [IO.File]::WriteAllText(
        (Join-Path (Get-JobDirectory $Arguments[0]) 'cancel.request'),
        [DateTime]::UtcNow.ToString('o'),
        [Text.UTF8Encoding]::new($false)
    )
    return [ordered]@{ id = $Arguments[0]; cancel_requested = $true }
}

function Invoke-Rerun([string[]]$Arguments) {
    if ($Arguments.Count -lt 1) { throw 'rerunはジョブIDを必要とします' }
    $record = Get-StatusRecord $Arguments[0]
    if ($record.status -in @('starting', 'running')) { throw '実行中のジョブは再実行できません' }
    $request = Unprotect-Request (Join-Path (Get-JobDirectory $Arguments[0]) 'request.bin')
    $estimatedMinutes = if ($record.PSObject.Properties['estimate']) {
        $record.estimate.estimated_total_minutes
    } else { $null }
    return New-Job $request $record.name ([bool]$record.critical) $estimatedMinutes
}

if ($args.Count -lt 1) { throw '操作を指定してください: start/status/tail/result/cancel/rerun' }
$action = $args[0]
$remaining = if ($args.Count -gt 1) { @($args[1..($args.Count - 1)]) } else { @() }

if ($action -eq 'worker') {
    $index = [Array]::IndexOf($remaining, '--job-dir')
    if ($index -lt 0 -or $index + 1 -ge $remaining.Count) { throw 'workerは--job-dirを必要とします' }
    Invoke-Worker $remaining[$index + 1]
    exit 0
}

$result = switch ($action) {
    'start' { Parse-Start $remaining }
    'status' { Invoke-Status $remaining }
    'tail' { Invoke-Tail $remaining }
    'result' { Invoke-Result $remaining }
    'cancel' { Invoke-Cancel $remaining }
    'rerun' { Invoke-Rerun $remaining }
    default { throw "不明な操作です: $action" }
}
ConvertTo-Json -InputObject $result -Depth 12 -Compress
