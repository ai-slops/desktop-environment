param([string]$CredentialPath = (Join-Path $PSScriptRoot '..\target\vm-session\credential.xml'))
$ErrorActionPreference='Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$status = Join-Path $repo 'target\vm-session\connection.json'
try {
    Import-Module Hyper-V
    $owner=Get-Content -LiteralPath (Join-Path $repo 'target\vm-hyperv\owner.json') -Raw|ConvertFrom-Json
    $vm=Get-VM -Id ([guid]$owner.id)
    if($vm.Name -ne 'DesktopEnvironment-AudioTest' -or $owner.name -ne $vm.Name){throw 'VM ownership mismatch'}
    if($vm.State -eq 'Off'){ & (Join-Path $PSScriptRoot 'desktop-test-vm.ps1') -Action Start }
    elseif($vm.State -ne 'Running'){throw 'The test VM is not ready to start.'}
    @{ok=$true;vm=$vm.Name;state='Running';startedUtc=[DateTime]::UtcNow.ToString('o')}|ConvertTo-Json|Set-Content -LiteralPath $status
    & (Join-Path $PSScriptRoot 'desktop-test-vm-session.ps1') -CredentialPath $CredentialPath -LifetimeHours 8
} catch {
    @{ok=$false;error=$_.Exception.Message}|ConvertTo-Json|Set-Content -LiteralPath $status
    throw
}
