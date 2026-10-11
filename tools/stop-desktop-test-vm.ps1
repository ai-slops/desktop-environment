param(
    [switch] $CleanupGuest,
    [string] $CredentialPath
)
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run in an Administrator PowerShell or use the scoped VM worker.' }
Import-Module Hyper-V
$owner = Get-Content -LiteralPath (Join-Path $workspace 'target/vm-hyperv/owner.json') -Raw | ConvertFrom-Json
if ($owner.name -ne 'DesktopEnvironment-AudioTest') { throw 'Unexpected VM ownership marker.' }
$vm = Get-VM -Id ([guid]$owner.id)
if ($vm.Name -ne $owner.name) { throw 'VM identity mismatch.' }
if ($vm.State -notin @('Running', 'Off')) { throw "Unexpected VM state: $($vm.State)" }
$cleanupError = $null
if ($vm.State -eq 'Running') {
    if ($CleanupGuest) {
        try {
            if (-not $CredentialPath) { $CredentialPath = Join-Path $workspace 'target/vm-session/credential.xml' }
            $credential = Import-Clixml -LiteralPath $CredentialPath
            if ($credential -isnot [PSCredential]) { throw 'Expected a DPAPI-protected credential file from this host account.' }
            Invoke-Command -VMId $vm.Id -Credential $credential -FilePath (Join-Path $PSScriptRoot 'cleanup-desktop-test-guest.ps1')
        } catch { $cleanupError = $_ }
    }
    # Normal guest shutdown only. Never force power-off or delete the VM/disk.
    Stop-VM -VM $vm -Confirm:$false
} elseif ($CleanupGuest) { Write-Warning 'VM is already off; guest artifacts were preserved. It was not started for cleanup.' }
$state = (Get-VM -Id $vm.Id).State.ToString()
if ($state -ne 'Off') { throw "VM shutdown not complete: $state" }
Write-Output "Owned test VM is Off: $($vm.Name)"
if ($cleanupError) { throw "VM is Off, but guest cleanup failed: $($cleanupError.Exception.Message)" }
