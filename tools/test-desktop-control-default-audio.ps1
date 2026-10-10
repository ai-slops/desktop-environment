param([string] $BinaryDirectory = (Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/vm-test-target/debug'))
$ErrorActionPreference = 'Stop'
$computer = Get-CimInstance Win32_ComputerSystem
if (($computer.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -and ($computer.Manufacturer -notmatch '^QEMU$|^VMware|^innotek GmbH$')) { throw 'Default audio routing smoke checks are VM-only.' }
$router = Join-Path $BinaryDirectory 'audio-output-router.exe'
function Get-Outputs {
    $lines = & $router list-audio-devices
    if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate guest render endpoints.' }
    foreach ($line in $lines) {
        $parts = $line -split "`t", 2
        if ($parts.Count -eq 2) {
            [pscustomobject]@{ id=$parts[0]; name=($parts[1] -replace ' \(default\)$',''); default=$parts[1].EndsWith(' (default)') }
        }
    }
}
$outputs = @(Get-Outputs)
$a = @($outputs | Where-Object name -EQ 'Voicemeeter Input (VB-Audio Voicemeeter VAIO)')
$b = @($outputs | Where-Object name -EQ 'Voicemeeter AUX Input (VB-Audio Voicemeeter VAIO)')
$c = @($outputs | Where-Object name -EQ 'CABLE Input (VB-Audio Virtual Cable)')
if ($a.Count -ne 1 -or $b.Count -ne 1 -or $c.Count -ne 1) { throw 'Need the three active VB-Audio fixture endpoints; no other endpoints will be used.' }
$original = @($outputs | Where-Object default)
if ($original.Count -ne 1) { throw 'Expected one guest default render endpoint to restore.' }
$root = Join-Path $env:LOCALAPPDATA ('DesktopEnvironment/audio-switch-tests/' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root -Force | Out-Null
$toolRoot = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/audio-fixtures/SoundVolumeView'
$tool = Join-Path $toolRoot 'SoundVolumeView.exe'
if (-not (Test-Path -LiteralPath $tool)) {
    New-Item -ItemType Directory -Path $toolRoot -Force | Out-Null
    $zip = Join-Path $toolRoot 'publisher.zip'
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -UseBasicParsing 'https://www.nirsoft.net/utils/soundvolumeview-x64.zip' -OutFile $zip
    Expand-Archive -LiteralPath $zip -DestinationPath $toolRoot -Force
    Write-Output "NirSoft guest helper SHA256: $((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash)"
}
function Set-GuestDefault([string] $Id) {
    $change = Start-Process -FilePath $tool -WindowStyle Hidden -ArgumentList @('/SetDefault', $Id, '0') -Wait -PassThru
    if ($change.ExitCode -ne 0) { throw "Default change helper exited with $($change.ExitCode)" }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        if (@(Get-Outputs | Where-Object { $_.default -and $_.id -eq $Id }).Count -eq 1) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Guest eConsole default did not change to the requested endpoint.'
}
$child = $null
$originalLog = $env:RUST_LOG
$stdout = Join-Path $root 'router.log'
$stderr = Join-Path $root 'router-errors.log'
function Wait-RoutingLog([string] $Text, [int] $AfterLength) {
    $deadline = [DateTime]::UtcNow.AddSeconds(12)
    do {
        [string] $log = ''
        if (Test-Path -LiteralPath $stdout) { $log = [string](Get-Content -LiteralPath $stdout -Raw) }
        if ($log -and $log.Length -gt $AfterLength) {
            [string] $newOutput = $log.Substring($AfterLength)
            if ($newOutput -and $newOutput.Contains($Text)) { return }
        }
        if ($child.HasExited) {
            if (Test-Path -LiteralPath $stdout) { Get-Content -LiteralPath $stdout | Write-Output }
            if (Test-Path -LiteralPath $stderr) { Get-Content -LiteralPath $stderr | Write-Output }
            throw "Owned router exited before reconnect; inspect $stdout and $stderr"
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "No expected routing event '$Text'; inspect $stdout and $stderr"
}
try {
    Set-GuestDefault $a[0].id
    $env:RUST_LOG = 'debug'
    $child = Start-Process -FilePath $router -WindowStyle Hidden -ArgumentList @('route','default',$c[0].id) -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    Wait-RoutingLog 'Started source capture and target render streams' 0
    $offset = ([string](Get-Content -LiteralPath $stdout -Raw)).Length
    Set-GuestDefault $b[0].id
    Wait-RoutingLog "Audio routing resumed from $($b[0].name) to $($c[0].name)" $offset
    if (@(Get-Outputs | Where-Object id -EQ $a[0].id).Count -ne 1) { throw 'A disappeared; this does not reproduce switching while A remains active.' }
    Write-Output 'PASS: A remains active; live default A -> B restarted WASAPI capture/render on B -> C without restarting the router.'
    $offset = ([string](Get-Content -LiteralPath $stdout -Raw)).Length
    Set-GuestDefault $c[0].id
    Wait-RoutingLog 'audio feedback prevention' $offset
    $offset = ([string](Get-Content -LiteralPath $stdout -Raw)).Length
    Set-GuestDefault $b[0].id
    Wait-RoutingLog "Audio routing resumed from $($b[0].name) to $($c[0].name)" $offset
    Write-Output 'PASS: selecting C paused for feedback prevention, then selecting B resumed B -> C.'
    $child.Kill(); $child.WaitForExit(); $child = $null

    Set-GuestDefault $a[0].id
    $stdout = Join-Path $root 'fixed-source.log'
    $stderr = Join-Path $root 'fixed-source-errors.log'
    $child = Start-Process -FilePath $router -WindowStyle Hidden -ArgumentList @('route',$a[0].id,$c[0].id) -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    Wait-RoutingLog 'Started source capture and target render streams' 0
    $offset = ([string](Get-Content -LiteralPath $stdout -Raw)).Length
    Set-GuestDefault $b[0].id
    Start-Sleep -Seconds 1
    $fixedLog = [string](Get-Content -LiteralPath $stdout -Raw)
    if ($child.HasExited -or $fixedLog.Substring($offset).Contains('reconnecting')) { throw 'Fixed endpoint routing changed or exited when only the default changed.' }
    if (@(Get-Outputs | Where-Object id -EQ $a[0].id).Count -ne 1) { throw 'The fixed A source is no longer active.' }
    Write-Output 'PASS: fixed A -> C remained running across the A -> B default change.'
    $child.Kill(); $child.WaitForExit(); $child = $null

    Set-GuestDefault $a[0].id
    $stdout = Join-Path $root 'default-target.log'
    $stderr = Join-Path $root 'default-target-errors.log'
    $child = Start-Process -FilePath $router -WindowStyle Hidden -ArgumentList @('route',$c[0].id,'default') -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    Wait-RoutingLog 'Started source capture and target render streams' 0
    $offset = ([string](Get-Content -LiteralPath $stdout -Raw)).Length
    Set-GuestDefault $b[0].id
    Wait-RoutingLog "Audio routing resumed from $($c[0].name) to $($b[0].name)" $offset
    Write-Output 'PASS: fixed C -> default A reconnected to C -> B without restarting the router.'
    Write-Output "Routing evidence: $root"
    Write-Output 'This checks actual endpoint notifications and stream reconnection, not captured signal fidelity. Tone/capture acceptance remains required.'
} finally {
    if ($child -and -not $child.HasExited) { $child.Kill(); $child.WaitForExit() }
    $env:RUST_LOG = $originalLog
    Set-GuestDefault $original[0].id
}
