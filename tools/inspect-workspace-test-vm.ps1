param([string]$CredentialPath=(Join-Path $PSScriptRoot '..\target\vm-session\credential.xml'))
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$destination=Join-Path $repo 'target\vm-session\inspection.json'
Import-Module Hyper-V
$owner=Get-Content -LiteralPath (Join-Path $repo 'target\vm-hyperv\owner.json') -Raw|ConvertFrom-Json
$vm=Get-VM -Id ([guid]$owner.id)
if($vm.Name -ne 'DesktopEnvironment-AudioTest' -or $owner.name -ne $vm.Name){throw 'VM ownership mismatch'}
$credential=Import-Clixml -LiteralPath $CredentialPath
$session=New-PSSession -VMId $vm.Id -Credential $credential
try{
    $deadline=[DateTime]::UtcNow.AddMinutes(15)
    while([DateTime]::UtcNow -lt $deadline){
        $data=Invoke-Command -Session $session -ScriptBlock {
            if((Get-CimInstance Win32_ComputerSystem).Model -ne 'Virtual Machine'){throw 'Expected test guest'}
            $root=Join-Path $env:LOCALAPPDATA 'DesktopEnvironment\workspace-vm-results'
            $log=Get-ChildItem -LiteralPath $root -Filter output.log -Recurse -ErrorAction SilentlyContinue|Sort-Object LastWriteTime -Descending|Select-Object -First 1
            $key=Get-Item 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
            @{desktop=@(Get-Process explorer -ErrorAction SilentlyContinue).Count;log=if($log){@(Get-Content -LiteralPath $log.FullName -Tail 65)}else{@()};processes=@(Get-Process DesktopWorkspaceUI,desktop-workspace-host -ErrorAction SilentlyContinue|Select-Object Id,ProcessName,SessionId);temporaryPasswordRemains=($key.GetValueNames() -contains 'DefaultPassword');utc=[DateTime]::UtcNow.ToString('o')}
        }
        [IO.File]::WriteAllText($destination,($data|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
        Start-Sleep -Seconds 5
    }
}finally{Remove-PSSession $session}
