param(
    [ValidateRange(1, 4)] [int] $BuildJobs = 1,
    [switch] $DesktopControlOnly,
    [switch] $InteractiveChild,
    [string] $ResultPath
)
$ErrorActionPreference = 'Stop'

# Fail closed before running any native tests or opening any audio endpoints.
$computer = Get-CimInstance Win32_ComputerSystem
$isGuest = ($computer.Model -match 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -or
    ($computer.Manufacturer -match '^QEMU$|^VMware|^innotek GmbH$')
if (-not $isGuest) {
    throw 'Run this script inside a Windows VM. Host audio/display tests are disabled.'
}

$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$sessionId = (Get-Process -Id $PID).SessionId
function Invoke-TestTool {
    param([string] $File, [string[]] $Arguments)
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $File @Arguments 2>&1 | ForEach-Object { $_.ToString() }
        $toolExitCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousPreference }
    if ($toolExitCode -ne 0) { throw "Guest test tool exited with $toolExitCode" }
}
function Invoke-SignalAcceptance {
    $cargo = Get-Command cargo -ErrorAction Stop
    $guestTarget = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/vm-test-target'
    $binaryDirectory = Join-Path $guestTarget 'debug'
    # Run native regressions before optional GPU viewport checks so a renderer
    # failure in another app does not prevent window-management validation.
    if (-not $DesktopControlOnly) {
        Invoke-TestTool $cargo.Source @('test', '--target-dir', $guestTarget, '--workspace', '--all-targets', '--all-features', '--no-fail-fast', '-j', "$BuildJobs")
        Invoke-TestTool $cargo.Source @('build', '--target-dir', $guestTarget, '-p', 'window-manager', '--features', 'ui-smoke', '-j', "$BuildJobs")
        $managerGuiRoot = Join-Path $env:LOCALAPPDATA ('DesktopEnvironment/window-manager-gui-tests/' + [guid]::NewGuid())
        New-Item -ItemType Directory -Path $managerGuiRoot -Force | Out-Null
        $managerPng = Join-Path $managerGuiRoot 'panel.png'
        $previousManagerScreenshot = $env:WINDOW_MANAGER_SCREENSHOT
        $managerProcess = $null
        try {
            # A missing configuration exercises real first-launch onboarding.
            $env:WINDOW_MANAGER_SCREENSHOT = $managerPng
            $managerConfig = Join-Path $managerGuiRoot 'settings.json'
            $managerErrors = Join-Path $managerGuiRoot 'errors.log'
            $managerProcess = Start-Process -FilePath (Join-Path $binaryDirectory 'window-manager.exe') -ArgumentList @('--config', ('"' + $managerConfig + '"')) -RedirectStandardError $managerErrors -PassThru
            $null = $managerProcess.Handle
            if (-not $managerProcess.WaitForExit(20000)) { throw 'Window Manager first-launch GUI capture timed out.' }
            if ($managerProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $managerPng)) {
                if (Test-Path -LiteralPath $managerErrors) { Get-Content -LiteralPath $managerErrors -Tail 30 }
                throw 'Window Manager first-launch GUI did not render.'
            }
            $png = [IO.File]::ReadAllBytes($managerPng)
            if ($png.Length -gt 2MB) { throw 'Window Manager screenshot exceeds evidence budget.' }
            $hash = (Get-FileHash -LiteralPath $managerPng -Algorithm SHA256).Hash.ToLowerInvariant()
            Write-Output ('VM_WINDOW_MANAGER_GUI_SCREENSHOT|' + $hash + '|' + [Convert]::ToBase64String($png))
            Write-Output 'PASS: Window Manager first-launch viewport rendered without control-monitor designation.'
        } finally {
            $env:WINDOW_MANAGER_SCREENSHOT = $previousManagerScreenshot
            if ($managerProcess -and -not $managerProcess.HasExited) { $managerProcess.Kill(); $managerProcess.WaitForExit() }
        }
    }
    Push-Location $workspace
    Invoke-TestTool $cargo.Source @('build', '--target-dir', $guestTarget, '-p', 'audio-output-router', '-j', "$BuildJobs")
    # Real, recorded signal acceptance uses only the dedicated guest fixtures.
    $engine = $null
    $probeVariables = @('DESKTOP_AUDIO_VM_PROBE', 'DESKTOP_AUDIO_PROBE_OUTPUT', 'DESKTOP_AUDIO_DEFAULT_HELPER', 'DESKTOP_AUDIO_ROUTER')
    $previousProbeEnvironment = @{}
    foreach ($variable in $probeVariables) { $previousProbeEnvironment[$variable] = [Environment]::GetEnvironmentVariable($variable) }
    try {
        if (-not (Get-Process -Name voicemeeterpro,voicemeeterpro64 -ErrorAction SilentlyContinue)) {
            $enginePath = Join-Path ${env:ProgramFiles(x86)} 'VB/Voicemeeter/voicemeeterpro.exe'
            if (-not (Test-Path -LiteralPath $enginePath)) { throw 'Install the VM Voicemeeter Banana fixture before signal acceptance.' }
            $engine = Start-Process -FilePath $enginePath -WindowStyle Hidden -PassThru
            Start-Sleep -Seconds 4
        }
        $env:DESKTOP_AUDIO_VM_PROBE = '1'
        $env:DESKTOP_AUDIO_PROBE_OUTPUT = Join-Path $env:LOCALAPPDATA ('DesktopEnvironment/audio-signal-tests/' + [guid]::NewGuid())
        $env:DESKTOP_AUDIO_DEFAULT_HELPER = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/audio-fixtures/SoundVolumeView/SoundVolumeView.exe'
        $env:DESKTOP_AUDIO_ROUTER = Join-Path $binaryDirectory 'audio-output-router.exe'
        Invoke-TestTool $cargo.Source @('test', '--target-dir', $guestTarget, '-p', 'windows-audio-router', 'live_cable_gain_acceptance', '-j', "$BuildJobs", '--', '--ignored', '--nocapture', '--test-threads=1')
        Invoke-TestTool $cargo.Source @('test', '--target-dir', $guestTarget, '-p', 'windows-audio-router', 'live_default_switch_signal_acceptance', '-j', "$BuildJobs", '--', '--ignored', '--nocapture', '--test-threads=1')
        Invoke-TestTool $cargo.Source @('test', '--target-dir', $guestTarget, '-p', 'desktop-control', 'gui_live_default_switch_keeps_child_and_symbolic_settings', '-j', "$BuildJobs", '--', '--ignored', '--nocapture', '--test-threads=1')
    } finally {
        foreach ($variable in $probeVariables) { [Environment]::SetEnvironmentVariable($variable, $previousProbeEnvironment[$variable]) }
        if ($engine -and -not $engine.HasExited) { $engine.Kill(); $engine.WaitForExit() }
        Pop-Location
    }
}
if ($sessionId -eq 0) {
    if ($InteractiveChild) { throw 'Interactive task unexpectedly started in session 0.' }
    # Fixture audio is visible here; enhanced desktop sessions expose only Remote Audio.
    Invoke-SignalAcceptance
    # PowerShell Direct runs in session 0, where there is no interactive display.
    # Use this same VM account's existing desktop token, without storing its
    # password in Task Scheduler or running the native tests as Administrator.
    $jobId = [guid]::NewGuid().ToString()
    $jobRoot = Join-Path $env:LOCALAPPDATA "DesktopEnvironment/interactive-tests/$jobId"
    New-Item -ItemType Directory -Path $jobRoot -Force | Out-Null
    $resultFile = Join-Path $jobRoot 'result.json'
    $logFile = Join-Path $jobRoot 'output.log'
    $scriptPath = $PSCommandPath.Replace("'", "''")
    $quotedResult = $resultFile.Replace("'", "''")
    $quotedLog = $logFile.Replace("'", "''")
    $onlyArgument = if ($DesktopControlOnly) { ' -DesktopControlOnly' } else { '' }
    $command = "& '$scriptPath' -BuildJobs $BuildJobs -InteractiveChild -ResultPath '$quotedResult'$onlyArgument *>&1 | Out-File -LiteralPath '$quotedLog' -Encoding utf8"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $taskName = 'DesktopEnvironment-Test-' + $jobId
    $taskAction = New-ScheduledTaskAction -Execute "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -EncodedCommand $encoded"
    $taskPrincipal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited
    $taskSettings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 45) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    $registered = $false
    try {
        Register-ScheduledTask -TaskName $taskName -Action $taskAction -Principal $taskPrincipal -Settings $taskSettings | Out-Null
        $registered = $true
        Start-ScheduledTask -TaskName $taskName
        Write-Output "Running native tests on the guest desktop; log: $logFile"
        $deadline = [DateTime]::UtcNow.AddMinutes(45)
        $startDeadline = [DateTime]::UtcNow.AddSeconds(30)
        $seenRunning = $false
        while (-not (Test-Path -LiteralPath $resultFile)) {
            $task = Get-ScheduledTask -TaskName $taskName
            if ($task.State -eq 'Running') { $seenRunning = $true }
            if (-not $seenRunning -and [DateTime]::UtcNow -gt $startDeadline) { throw 'Log into the VM desktop as the test account, then retry. No interactive test session started.' }
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Interactive guest tests exceeded 45 minutes.' }
            Start-Sleep -Seconds 2
        }
        if (Test-Path -LiteralPath $logFile) { Get-Content -LiteralPath $logFile }
        $result = Get-Content -LiteralPath $resultFile -Raw | ConvertFrom-Json
        if (-not $result.ok) { throw "Interactive guest tests failed: $($result.error)" }
        return
    } finally {
        if ($registered) {
            if ((Get-ScheduledTask -TaskName $taskName).State -eq 'Running') { Stop-ScheduledTask -TaskName $taskName }
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
        }
    }
}

Push-Location $workspace
$result = @{ ok = $false; sessionId = $sessionId }
try {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        $mise = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/bin/mise.exe'
        $cargoPath = & $mise which cargo
        if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve guest cargo with mise.' }
        $cargo = Get-Command ([string]$cargoPath) -ErrorAction Stop
        $env:PATH = (Split-Path $cargo.Source -Parent) + ';' + $env:PATH
    }
    # A guest-local target directory avoids replacing binaries in a shared host checkout.
    $guestTarget = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/vm-test-target'
    $binaryDirectory = Join-Path $guestTarget 'debug'
    # Render the real native GUI on the guest desktop. The existing ui-smoke
    # feature captures only its own viewport and closes without starting routing.
    Invoke-TestTool $cargo.Source @('build', '--target-dir', $guestTarget, '-p', 'desktop-control', '--features', 'ui-smoke', '-j', "$BuildJobs")
    $guiId = [guid]::NewGuid().ToString()
    $guiRoot = Join-Path $env:LOCALAPPDATA ('DesktopEnvironment/gui-render-tests/' + $guiId)
    New-Item -ItemType Directory -Path $guiRoot -Force | Out-Null
    $screenshot = Join-Path $guiRoot 'panel.png'
    $previousScreenshot = $env:DESKTOP_CONTROL_SCREENSHOT
    $guiProcess = $null
    try {
        $env:DESKTOP_CONTROL_SCREENSHOT = $screenshot
        $config = Join-Path $guiRoot 'presets.json'
        $guiErrors = Join-Path $guiRoot 'errors.log'
        $guiProcess = Start-Process -FilePath (Join-Path $binaryDirectory 'desktop-control.exe') -ArgumentList @('--config', ('"' + $config + '"')) -RedirectStandardError $guiErrors -PassThru
        $null = $guiProcess.Handle # Retain the process handle until exit status is read.
        if (-not $guiProcess.WaitForExit(20000)) { throw 'Native guest GUI capture timed out.' }
        if ($guiProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $screenshot)) {
            if (Test-Path -LiteralPath $guiErrors) { Get-Content -LiteralPath $guiErrors -Tail 30 }
            throw "Native guest GUI exit=$($guiProcess.ExitCode), viewport exists=$(Test-Path -LiteralPath $screenshot), evidence=$guiRoot"
        }
        $png = [IO.File]::ReadAllBytes($screenshot)
        if ($png.Length -gt 2MB) { throw 'GUI screenshot exceeds evidence budget.' }
        $hash = (Get-FileHash -LiteralPath $screenshot -Algorithm SHA256).Hash.ToLowerInvariant()
        Write-Output ('VM_GUI_SCREENSHOT|' + $guiId + '|' + $hash + '|' + [Convert]::ToBase64String($png))
        Write-Output 'PASS: native Desktop Control viewport rendered and closed in the VM.'
    } finally {
        $env:DESKTOP_CONTROL_SCREENSHOT = $previousScreenshot
        if ($guiProcess -and -not $guiProcess.HasExited) { $guiProcess.Kill(); $guiProcess.WaitForExit() }
    }
    Invoke-TestTool $cargo.Source @('build', '--target-dir', $guestTarget, '-p', 'audio-cable', '--features', 'ui-smoke', '-j', "$BuildJobs")
    $cableGuiId = [guid]::NewGuid().ToString()
    $cableGuiRoot = Join-Path $env:LOCALAPPDATA ('DesktopEnvironment/cable-gui-tests/' + $cableGuiId)
    New-Item -ItemType Directory -Path $cableGuiRoot -Force | Out-Null
    $cablePng = Join-Path $cableGuiRoot 'panel.png'
    $previousCableScreenshot = $env:AUDIO_CABLE_SCREENSHOT
    $cableGuiProcess = $null
    try {
        $env:AUDIO_CABLE_SCREENSHOT = $cablePng
        $cableConfig = Join-Path $cableGuiRoot 'settings.json'
        $cableErrors = Join-Path $cableGuiRoot 'errors.log'
        $cableGuiProcess = Start-Process -FilePath (Join-Path $binaryDirectory 'audio-cable.exe') -ArgumentList @('--config', ('"' + $cableConfig + '"')) -RedirectStandardError $cableErrors -PassThru
        $null = $cableGuiProcess.Handle
        if (-not $cableGuiProcess.WaitForExit(20000)) { throw 'Audio Cable GUI capture timed out.' }
        if ($cableGuiProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $cablePng)) {
            if (Test-Path -LiteralPath $cableErrors) { Get-Content -LiteralPath $cableErrors -Tail 30 }
            throw 'Audio Cable native GUI did not render.'
        }
        $png = [IO.File]::ReadAllBytes($cablePng)
        if ($png.Length -gt 2MB) { throw 'Cable GUI screenshot exceeds evidence budget.' }
        $hash = (Get-FileHash -LiteralPath $cablePng -Algorithm SHA256).Hash.ToLowerInvariant()
        Write-Output ('VM_AUDIO_CABLE_GUI_SCREENSHOT|' + $cableGuiId + '|' + $hash + '|' + [Convert]::ToBase64String($png))
        Write-Output 'PASS: native Audio Cable viewport rendered and closed in the VM.'
    } finally {
        $env:AUDIO_CABLE_SCREENSHOT = $previousCableScreenshot
        if ($cableGuiProcess -and -not $cableGuiProcess.HasExited) { $cableGuiProcess.Kill(); $cableGuiProcess.WaitForExit() }
    }
    Invoke-TestTool $cargo.Source @('test', '--target-dir', $guestTarget, '-p', 'windows-audio-router', '-p', 'desktop-presets', '-j', "$BuildJobs")
    Invoke-TestTool $cargo.Source @('build', '--target-dir', $guestTarget, '-p', 'desktop-control', '-p', 'audio-output-router', '-p', 'display-relay', '-j', "$BuildJobs")

    $binaryDirectory = Join-Path $guestTarget 'debug'
    # This enumerates endpoints only; it does not start cloning or change defaults.
    Invoke-TestTool (Join-Path $binaryDirectory 'audio-output-router.exe') @('list-audio-devices')
    Write-Output "Guest binaries: $binaryDirectory"
    $previousRelayProbe = $env:DESKTOP_RELAY_VM_PROBE
    try {
        $env:DESKTOP_RELAY_VM_PROBE = '1'
        Invoke-TestTool $cargo.Source @('test', '--target-dir', $guestTarget, '-p', 'display-relay', 'relay_pointer_jump_vm_acceptance', '-j', "$BuildJobs", '--', '--ignored', '--nocapture', '--test-threads=1')
    } finally { $env:DESKTOP_RELAY_VM_PROBE = $previousRelayProbe }
    if (-not $InteractiveChild) { Invoke-SignalAcceptance }
    Write-Output 'Workspace regressions passed. The VM worker separately verifies recorded signals and the GUI router lifecycle.'
    $result.ok = $true
} catch {
    $result.error = $_.Exception.Message
    throw
} finally {
    Pop-Location
    if ($ResultPath) { $result | ConvertTo-Json | Set-Content -LiteralPath $ResultPath }
}
