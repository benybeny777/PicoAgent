$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$pluginRoot = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$jobScript = Join-Path $pluginRoot 'scripts\job.ps1'
$pwsh = (Get-Command pwsh -CommandType Application | Select-Object -First 1).Source
$testBase = Join-Path $pluginRoot 'temp-tests'
$testRoot = Join-Path $testBase ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
$env:WINDOWS_PROCESS_JOBS_HOME = $testRoot
$env:WINDOWS_PROCESS_JOBS_DISABLE_OS_NOTIFICATION = '1'
$env:WINDOWS_PROCESS_JOBS_DISABLE_CODEX_QUEUE = '1'
$env:WINDOWS_PROCESS_JOBS_TEST_INTERVAL_SECONDS = '1'
$env:CODEX_THREAD_ID = '00000000-0000-0000-0000-000000000001'

function Invoke-Job([string[]]$Arguments) {
    $text = & $pwsh -NoProfile -File $jobScript @Arguments
    if ($LASTEXITCODE -ne 0) { throw "job.ps1が失敗しました: $text" }
    return $text | ConvertFrom-Json
}

function Wait-Terminal([string]$JobId, [int]$TimeoutSeconds = 30) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        Start-Sleep -Milliseconds 250
        $status = Invoke-Job @('status', $JobId, '--json')
        if ($status.record.status -notin @('starting', 'running')) { return $status }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "ジョブが時間内に終了しません: $JobId"
}

function Assert-Equal($Actual, $Expected, [string]$Message) {
    if ($Actual -ne $Expected) { throw "$Message actual=$Actual expected=$Expected" }
}

try {
    $claudeEnvFile = Join-Path $testRoot 'claude-env.sh'
    $env:CLAUDE_ENV_FILE = $claudeEnvFile
    '{"hook_event_name":"SessionStart","session_id":"claude-session-test-0001"}' | node (Join-Path $pluginRoot 'hooks\session-hook.mjs')
    if ((Get-Content -Raw -Encoding UTF8 -LiteralPath $claudeEnvFile) -notmatch "WINDOWS_PROCESS_JOBS_OWNER_THREAD_ID='claude-session-test-0001'") {
        throw 'Claude Codeの所有セッション引き継ぎを作成できません'
    }

    $success = Invoke-Job @(
        'start', '--name', 'success', '--cwd', $pluginRoot, '--estimate-minutes', '5', '--',
        $pwsh, '-NoProfile', '-Command',
        "Write-Output 'success-output'; [Console]::Error.WriteLine('success-error')"
    )
    $successResult = Wait-Terminal $success.id
    Assert-Equal $successResult.record.status 'succeeded' '成功状態が違います'
    Assert-Equal $successResult.timing.elapsed '1分未満' '経過時間表示が違います'
    Assert-Equal $successResult.timing.remaining_estimate '完了' '残り時間表示が違います'
    if ($successResult.timing.PSObject.Properties.Name -match 'seconds') { throw '利用者向け時間表示に秒が含まれています' }
    if ($successResult.stdout_tail -notcontains 'success-output') { throw '標準出力を取得できません' }
    if ($successResult.stderr_tail -notcontains 'success-error') { throw '標準エラーを取得できません' }
    $successEvents = @(Get-ChildItem -LiteralPath (Join-Path (Join-Path $testRoot $success.id) 'events') -Filter '*.json')
    $successEventValues = @($successEvents | ForEach-Object { Get-Content -Raw -Encoding UTF8 -LiteralPath $_.FullName | ConvertFrom-Json })
    if (@($successEventValues | Where-Object kind -eq 'completion').Count -ne 1) { throw '成功完了通知イベントが1件ではありません' }
    Assert-Equal $successResult.record.notification.owner_thread_id $env:CODEX_THREAD_ID '通知先スレッドが保存されていません'
    Assert-Equal $successResult.record.notification.last_delivery.event_saved $true '通知イベント保存結果が記録されていません'

    $progress = Invoke-Job @(
        'start', '--name', 'progress', '--cwd', $pluginRoot, '--estimate-minutes', '5', '--',
        $pwsh, '-NoProfile', '-Command', 'Start-Sleep -Seconds 5'
    )
    $progressResult = Wait-Terminal $progress.id
    Assert-Equal $progressResult.record.status 'succeeded' '進捗通知ジョブが成功しません'
    if ($progressResult.record.notification.progress_count -lt 2) { throw '定期進捗通知が複数回作成されません' }
    $progressEvents = @(Get-ChildItem -LiteralPath (Join-Path (Join-Path $testRoot $progress.id) 'events') -Filter '*.json')
    if (@($progressEvents | ForEach-Object { Get-Content -Raw -Encoding UTF8 -LiteralPath $_.FullName | ConvertFrom-Json } | Where-Object kind -eq 'progress').Count -lt 2) {
        throw '進捗通知イベントが不足しています'
    }

    $hook = Join-Path $pluginRoot 'hooks\notification-hook.mjs'
    $hookInput = [ordered]@{ hook_event_name = 'UserPromptSubmit'; thread_id = $env:CODEX_THREAD_ID } | ConvertTo-Json -Compress
    $hookOutput = $hookInput | node $hook | ConvertFrom-Json
    if ($hookOutput.hookSpecificOutput.additionalContext -notmatch 'バックグラウンドで実行中') { throw 'フックが進捗通知を会話へ返しません' }

    $quiet = Invoke-Job @(
        'start', '--name', 'quiet', '--cwd', $pluginRoot, '--no-notify', '--',
        $pwsh, '-NoProfile', '-Command', "Write-Output 'quiet-output'"
    )
    $quietResult = Wait-Terminal $quiet.id
    Assert-Equal $quietResult.record.status 'succeeded' '通知なしジョブが成功しません'
    if (Test-Path -LiteralPath (Join-Path (Join-Path $testRoot $quiet.id) 'events')) { throw '通知なしジョブがイベントを作成しました' }

    $failure = Invoke-Job @(
        'start', '--name', 'failure', '--cwd', $pluginRoot, '--',
        $pwsh, '-NoProfile', '-Command', 'exit 7'
    )
    $failureResult = Wait-Terminal $failure.id
    Assert-Equal $failureResult.record.status 'failed' '失敗状態が違います'
    Assert-Equal $failureResult.record.exit_code 7 '終了コードが違います'

    $childScript = @'
$child = Start-Process pwsh -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 60' -PassThru
Write-Output ('child=' + $child.Id)
Wait-Process -Id $child.Id
'@
    $cancel = Invoke-Job @(
        'start', '--name', 'cancel-child', '--cwd', $pluginRoot, '--',
        $pwsh, '-NoProfile', '-Command', $childScript
    )
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $childPid = $null
    do {
        Start-Sleep -Milliseconds 250
        $tail = Invoke-Job @('tail', $cancel.id, '--lines', '10', '--json')
        foreach ($line in @($tail.stdout)) {
            if ($line -match '^child=(\d+)$') { $childPid = [int]$Matches[1] }
        }
    } while (-not $childPid -and [DateTime]::UtcNow -lt $deadline)
    if (-not $childPid) { throw '子プロセスPIDを取得できません' }
    Invoke-Job @('cancel', $cancel.id, '--json') | Out-Null
    $cancelResult = Wait-Terminal $cancel.id 30
    Assert-Equal $cancelResult.record.status 'cancelled' '取消状態が違います'
    if (Get-Process -Id $childPid -ErrorAction SilentlyContinue) { throw '取消後も子プロセスが残っています' }

    $rerun = Invoke-Job @('rerun', $success.id, '--json')
    if ($rerun.id -eq $success.id) { throw '再実行が新しいジョブIDを作っていません' }
    $rerunResult = Wait-Terminal $rerun.id
    Assert-Equal $rerunResult.record.status 'succeeded' '再実行が成功しません'

    $overrun = Invoke-Job @(
        'start', '--name', 'estimate-overrun', '--cwd', $pluginRoot, '--estimate-minutes', '0.1', '--',
        $pwsh, '-NoProfile', '-Command', 'Start-Sleep -Seconds 8'
    )
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $updatedEstimate = $null
    do {
        Start-Sleep -Milliseconds 250
        $overrunStatus = Invoke-Job @('status', $overrun.id, '--json')
        if ($overrunStatus.record.estimate.updated_count -gt 0) {
            $updatedEstimate = $overrunStatus
            break
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $updatedEstimate) { throw '見積り超過後に残り時間が更新されません' }
    if ($updatedEstimate.timing.remaining_estimate -notmatch '更新済み') { throw '更新済み表示がありません' }
    if ($updatedEstimate.timing.PSObject.Properties.Name -match 'seconds') { throw '更新後の時間表示に秒が含まれています' }
    $overrunResult = Wait-Terminal $overrun.id
    Assert-Equal $overrunResult.record.status 'succeeded' '見積り超過ジョブが成功しません'

    $large = Invoke-Job @(
        'start', '--name', 'bounded-log', '--cwd', $pluginRoot, '--',
        $pwsh, '-NoProfile', '-Command', "[Console]::Out.Write(('x' * 11000000))"
    )
    $largeResult = Wait-Terminal $large.id 60
    Assert-Equal $largeResult.record.status 'succeeded' '大量出力ジョブが成功しません'
    $largeLog = Join-Path (Join-Path $testRoot $large.id) 'stdout.log'
    if ((Get-Item -LiteralPath $largeLog).Length -gt 10MB) { throw 'ログ容量上限を超えています' }

    [ordered]@{
        success = $success.id
        progress = $progress.id
        quiet = $quiet.id
        failure = $failure.id
        cancelled = $cancel.id
        rerun = $rerun.id
        estimate_overrun = $overrun.id
        bounded_log = $large.id
    } | ConvertTo-Json -Compress
} finally {
    $resolvedBase = [IO.Path]::GetFullPath($testBase)
    $resolvedTarget = [IO.Path]::GetFullPath($testRoot)
    if ($resolvedTarget.StartsWith($resolvedBase + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        [IO.Directory]::Delete($resolvedTarget, $true)
    }
}
