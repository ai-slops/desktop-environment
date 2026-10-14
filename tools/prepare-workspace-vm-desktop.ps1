param([string]$CredentialPath=(Join-Path $PSScriptRoot '..\target\vm-session\credential.xml'))
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$status=Join-Path $repo 'target\vm-session\desktop.json'
try{
    Import-Module Hyper-V
    $owner=Get-Content -LiteralPath (Join-Path $repo 'target\vm-hyperv\owner.json') -Raw|ConvertFrom-Json
    $vm=Get-VM -Id ([guid]$owner.id)
    if($vm.Name -ne 'DesktopEnvironment-AudioTest' -or $owner.name -ne $vm.Name){throw 'VM ownership mismatch'}
    $credential=Import-Clixml -LiteralPath $CredentialPath
    if($credential -isnot [PSCredential]){throw 'Expected DPAPI credential'}
    $session=New-PSSession -VMId $vm.Id -Credential $credential
    try{
        $prepared=Invoke-Command -Session $session -ArgumentList $credential.UserName,$credential.GetNetworkCredential().Password -ScriptBlock {
            param($user,$password)
            $ErrorActionPreference='Stop'
            if((Get-CimInstance Win32_ComputerSystem).Model -ne 'Virtual Machine'){throw 'Expected owned Windows guest'}
            if(Get-Process explorer -ErrorAction SilentlyContinue){return $false}
            $key='HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
            $properties=@('AutoAdminLogon','DefaultUserName','DefaultDomainName','AutoLogonCount')
            if((Get-Item -LiteralPath $key).GetValueNames() -contains 'DefaultPassword'){throw 'Existing guest autologon password detected; preserving it'}
            $root=Join-Path $env:LOCALAPPDATA 'DesktopEnvironment\vm-desktop-bootstrap';$null=New-Item -ItemType Directory -Path $root -Force
            $backup=@{};foreach($name in $properties){$registry=Get-Item -LiteralPath $key;if($registry.GetValueNames() -contains $name){$backup[$name]=@{value=$registry.GetValue($name);kind=$registry.GetValueKind($name).ToString()}}}
            $backup|Export-Clixml -LiteralPath (Join-Path $root 'registry-backup.xml')
            $cleanup=Join-Path $root 'restore.ps1'
            $script=@'
$ErrorActionPreference='Stop'
$key='HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
$backup=Import-Clixml -LiteralPath (Join-Path $PSScriptRoot 'registry-backup.xml')
foreach($name in @('AutoAdminLogon','DefaultUserName','DefaultDomainName','AutoLogonCount','DefaultPassword')){Remove-ItemProperty -LiteralPath $key -Name $name -ErrorAction SilentlyContinue}
foreach($name in $backup.Keys){New-ItemProperty -LiteralPath $key -Name $name -Value $backup[$name].value -PropertyType $backup[$name].kind -Force|Out-Null}
Unregister-ScheduledTask -TaskName 'DesktopEnvironment-Workspace-Logon-Restore' -Confirm:$false
Write-Output 'Temporary test VM autologon settings restored.'
'@
            [IO.File]::WriteAllText($cleanup,$script,[Text.UTF8Encoding]::new($false))
            $localUser=$user.Split('\')[-1]
            $action=New-ScheduledTaskAction -Execute "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -Argument ('-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+$cleanup+'"')
            $trigger=New-ScheduledTaskTrigger -AtLogOn -User $localUser
            $principal=New-ScheduledTaskPrincipal -UserId $localUser -LogonType Interactive -RunLevel Highest
            Register-ScheduledTask -TaskName 'DesktopEnvironment-Workspace-Logon-Restore' -Action $action -Trigger $trigger -Principal $principal -Force|Out-Null
            New-ItemProperty -LiteralPath $key -Name AutoAdminLogon -Value '1' -PropertyType String -Force|Out-Null
            New-ItemProperty -LiteralPath $key -Name DefaultUserName -Value $localUser -PropertyType String -Force|Out-Null
            New-ItemProperty -LiteralPath $key -Name DefaultDomainName -Value $env:COMPUTERNAME -PropertyType String -Force|Out-Null
            New-ItemProperty -LiteralPath $key -Name DefaultPassword -Value $password -PropertyType String -Force|Out-Null
            New-ItemProperty -LiteralPath $key -Name AutoLogonCount -Value 1 -PropertyType DWord -Force|Out-Null
            return $true
        }
        if($prepared){try{Invoke-Command -Session $session -ScriptBlock {Restart-Computer -Force}}catch{}}
    }finally{Remove-PSSession $session -ErrorAction SilentlyContinue}
    @{ok=$true;restarted=[bool]$prepared;vm=$vm.Name}|ConvertTo-Json|Set-Content -LiteralPath $status
}catch{@{ok=$false;error=$_.Exception.Message}|ConvertTo-Json|Set-Content -LiteralPath $status;throw}
