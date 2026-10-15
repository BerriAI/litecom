param([string]$Binary = "target/release/litecom.exe")

# Exercise real Task Scheduler states without connecting an account or model.
$ErrorActionPreference = "Stop"
$binaryPath = (Resolve-Path $Binary).Path
$connection = "qa-status-" + [guid]::NewGuid().ToString("N")
$taskName = "org.litecom." + $connection
$configPath = Join-Path ([IO.Path]::GetTempPath()) $connection
New-Item -ItemType Directory -Path $configPath | Out-Null

function Worker-IsRunning {
    $status = & $binaryPath --home $configPath --connection $connection service status
    if ($LASTEXITCODE -ne 0) { throw "Litecom could not read the task state." }
    return ($status -eq "Background worker is running.")
}

function Wait-ForState([bool]$running) {
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        if ((Worker-IsRunning) -eq $running) { return }
        Start-Sleep -Seconds 1
    }
    throw "Litecom did not report the expected running state: $running"
}

try {
    $shell = (Get-Command powershell.exe).Source
    $action = New-ScheduledTaskAction -Execute $shell -Argument '-NoLogo -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 60"'
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 2)
    # No trigger: this test task cannot run on a later sign-in.
    Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal -Settings $settings | Out-Null
    Wait-ForState $false
    Start-ScheduledTask -TaskName $taskName
    Wait-ForState $true
    & $binaryPath --home $configPath --connection $connection service stop
    if ($LASTEXITCODE -ne 0) { throw "Litecom could not stop the test task." }
    Wait-ForState $false
    & $binaryPath --home $configPath --connection $connection service remove
    if ($LASTEXITCODE -ne 0) { throw "Litecom could not remove the test task." }
    Wait-ForState $false
    if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
        throw "The test task remains installed."
    }
    & $binaryPath --home $configPath --connection $connection service remove
    if ($LASTEXITCODE -ne 0) { throw "Repeating removal must succeed when the task is already absent." }
    Write-Output "PASS: installed/stopped, running, stop, removal, and repeated removal states."
} finally {
    Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $configPath -ErrorAction SilentlyContinue
}
