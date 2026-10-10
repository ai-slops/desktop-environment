param(
    [ValidateSet('Plan', 'Setup', 'Start', 'Status', 'Stop', 'Console')]
    [string] $Action = 'Plan',
    [string] $IsoPath = $env:DESKTOP_TEST_VM_ISO,
    [ValidateRange(4, 64)] [int] $MemoryGiB = 32,
    [ValidateRange(2, 32)] [int] $Processors = 12,
    [ValidateRange(1, 100)] [int] $CpuMaximum = 40
)
$ErrorActionPreference = 'Stop'
$vmName = 'DesktopEnvironment-AudioTest'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$vmRoot = Join-Path $workspace 'target/vm-hyperv'
$markerPath = Join-Path $vmRoot 'owner.json'
$diskPath = Join-Path $vmRoot 'windows.vhdx'
if (-not $IsoPath) {
    $iso = Get-ChildItem (Join-Path $env:USERPROFILE 'Downloads') -Filter 'Win11*_x64.iso' -File |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($iso) { $IsoPath = $iso.FullName }
}
if ($Action -eq 'Plan') {
    [ordered]@{
        backend = 'Hyper-V'; name = $vmName; memoryGiB = $MemoryGiB
        processors = $Processors; cpuMaximumPercent = $CpuMaximum
        diskGiB = 96; disk = $diskPath; iso = $IsoPath
        secureBoot = $true; virtualTPM = $true; autoStart = $false
        network = 'Existing Default Switch only'; hostAudioPassthrough = $false
    } | ConvertTo-Json
    return
}

$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Hyper-V management needs an Administrator PowerShell. No host feature or group membership was changed.'
}
Import-Module Hyper-V
if ((Get-Service vmms).Status -ne 'Running') { throw 'Hyper-V must already be running; this script will not enable features or reboot.' }
$vm = Get-VM | Where-Object Name -EQ $vmName
if ($vm) {
    if (-not (Test-Path -LiteralPath $markerPath)) { throw 'Existing VM has no workspace ownership marker. Refusing to modify it.' }
    $owner = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    if ($owner.id -ne $vm.Id.ToString()) { throw 'VM identity differs from the workspace marker.' }
} elseif ($Action -ne 'Setup') {
    throw 'Create the VM first with mise run vm-setup.'
}

switch ($Action) {
    'Setup' {
        if (-not $IsoPath -or -not (Test-Path -LiteralPath $IsoPath -PathType Leaf)) { throw 'Set DESKTOP_TEST_VM_ISO to a Windows 11 x64 ISO.' }
        $IsoPath = (Resolve-Path -LiteralPath $IsoPath).Path
        if ($vm -and $vm.State -ne 'Off') { throw 'Shut down this test VM before changing its resources.' }
        $switch = Get-VMSwitch | Where-Object Name -EQ 'Default Switch'
        if (-not $switch) { throw 'Existing Default Switch not found. No host network switches were created.' }
        if (-not $vm) {
            if (Test-Path -LiteralPath $diskPath) { throw 'A disk already exists without a registered owned VM. Preserve it and inspect manually.' }
            New-Item -ItemType Directory -Path $vmRoot -Force | Out-Null
            $vm = New-VM -Name $vmName -Generation 2 -MemoryStartupBytes ($MemoryGiB * 1GB) -Path $vmRoot `
                -NewVHDPath $diskPath -NewVHDSizeBytes 96GB -SwitchName $switch.Name
            @{ id = $vm.Id.ToString(); name = $vmName; provisioned = $false } | ConvertTo-Json | Set-Content -LiteralPath $markerPath
        }
        Set-VMMemory -VM $vm -DynamicMemoryEnabled $false -StartupBytes ($MemoryGiB * 1GB)
        Set-VMProcessor -VM $vm -Count $Processors -Maximum $CpuMaximum -Reserve 0 -RelativeWeight 10
        Set-VM -VM $vm -AutomaticStartAction Nothing -AutomaticStopAction ShutDown -AutomaticCheckpointsEnabled $false
        Set-VMFirmware -VM $vm -EnableSecureBoot On -SecureBootTemplate MicrosoftWindows
        if (-not (Get-VMSecurity -VM $vm).TpmEnabled) {
            Set-VMKeyProtector -VM $vm -NewLocalKeyProtector
            Enable-VMTPM -VM $vm
        }
        $dvd = Get-VMDvdDrive -VM $vm | Select-Object -First 1
        if ($dvd) { Set-VMDvdDrive -VMDvdDrive $dvd -Path $IsoPath }
        else { $dvd = Add-VMDvdDrive -VM $vm -Path $IsoPath -Passthru }
        Set-VMFirmware -VM $vm -FirstBootDevice $dvd
        @{ id = $vm.Id.ToString(); name = $vmName; provisioned = $true; memoryGiB = $MemoryGiB; processors = $Processors; cpuMaximum = $CpuMaximum } |
            ConvertTo-Json | Set-Content -LiteralPath $markerPath
        Write-Output 'Test VM created/configured and left off. Run vm-start, then vm-console to install Windows in the guest.'
    }
    'Start' {
        $owner = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
        if (-not $owner.provisioned) { throw 'VM setup is incomplete; run vm-setup again.' }
        $available = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory * 1KB
        if ($available -lt ($vm.MemoryStartup + 4GB)) { throw 'Insufficient available RAM for this guest plus a 4 GiB host reserve.' }
        Start-VM -VM $vm
    }
    'Status' {
        $processor = Get-VMProcessor -VM $vm
        [ordered]@{ name = $vm.Name; id = $vm.Id.ToString(); state = $vm.State.ToString(); memoryGiB = $vm.MemoryStartup / 1GB; processors = $processor.Count; cpuMaximumPercent = $processor.Maximum } | ConvertTo-Json
    }
    'Stop' { Stop-VM -VM $vm }
    'Console' {
        # Explicit console action is the interactive VM window requested by the user.
        Start-Process (Join-Path $env:WINDIR 'System32/vmconnect.exe') -ArgumentList @('localhost', $vmName)
    }
}
