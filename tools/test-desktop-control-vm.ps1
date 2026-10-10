param(
    [ValidateRange(1, 4)] [int] $BuildJobs = 1
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
$cargo = Get-Command cargo -ErrorAction Stop
Push-Location $workspace
try {
    # A guest-local target directory avoids replacing binaries in a shared host checkout.
    $guestTarget = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/vm-test-target'
    & $cargo.Source test --target-dir $guestTarget -p windows-audio-router -p desktop-presets -j $BuildJobs
    if ($LASTEXITCODE -ne 0) { throw 'Audio/preset regression tests failed.' }
    & $cargo.Source build --target-dir $guestTarget -p desktop-control -p audio-output-router -p display-relay -j $BuildJobs
    if ($LASTEXITCODE -ne 0) { throw 'Desktop Control guest build failed.' }

    $binaryDirectory = Join-Path $guestTarget 'debug'
    # This enumerates endpoints only; it does not start cloning or change defaults.
    & (Join-Path $binaryDirectory 'audio-output-router.exe') list-audio-devices
    if ($LASTEXITCODE -ne 0) { throw 'Guest audio enumeration failed. Configure guest audio endpoints first.' }
    Write-Output "Guest binaries: $binaryDirectory"
    Write-Output 'Automated regressions passed. Live A/B/C endpoint switching still requires the VM-only acceptance procedure in docs/desktop-control-vm-testing.md.'
} finally {
    Pop-Location
}
