param([switch]$InteractiveChild,[string]$ResultPath)
$ErrorActionPreference='Stop'
trap { Write-Output ('VM test failure: '+$_.Exception.Message); if($ResultPath){[IO.File]::WriteAllText($ResultPath,(@{ok=$false;error=$_.Exception.Message}|ConvertTo-Json),[Text.UTF8Encoding]::new($false))};throw $_ }
$computer=Get-CimInstance Win32_ComputerSystem
if($computer.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU'){throw 'Workspace execution tests require the Windows test VM'}
$source=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$package=Join-Path $source 'target\workspace-test-package'
$metadata=Get-Content -LiteralPath (Join-Path $package 'package.json') -Raw|ConvertFrom-Json
if(!(Test-Path -LiteralPath (Join-Path $package ('payload\'+$metadata.version)))){
    $reuse=Get-Content -LiteralPath (Join-Path $source 'target\workspace-vm-reuse.json') -Raw|ConvertFrom-Json
    if($reuse.source -notmatch '^source-[a-f0-9]{64}$'){throw 'Invalid VM source cache'}
    $oldPackage=Join-Path (Split-Path $source -Parent) ($reuse.source+'\target\workspace-test-package')
    $oldMetadata=Get-Content -LiteralPath (Join-Path $oldPackage 'package.json') -Raw|ConvertFrom-Json
    $newPayload=Join-Path $package ('payload\'+$metadata.version)
    $null=New-Item -ItemType Directory -Path $newPayload -Force
    Get-ChildItem -LiteralPath (Join-Path $oldPackage ('payload\'+$oldMetadata.version)) | ForEach-Object {Copy-Item -LiteralPath $_.FullName -Destination $newPayload -Recurse}
    $resources=Join-Path $newPayload 'resources\app'
    foreach($directory in @('shell','dist')){Get-ChildItem -LiteralPath (Join-Path $source ('apps\desktop-workspace\'+$directory)) | ForEach-Object {Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $resources $directory) -Recurse -Force}}
    Copy-Item -LiteralPath (Join-Path $source 'apps\desktop-workspace\server.mjs') -Destination (Join-Path $resources 'server.mjs') -Force
    Get-ChildItem -LiteralPath (Join-Path $source 'target\workspace-test-bin') | ForEach-Object {Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $resources 'bin') -Force}
}
$electron=Join-Path $package ('payload\'+$metadata.version+'\DesktopWorkspaceUI.exe')
function Run-Tool([string]$file,[string[]]$arguments){
    $previous=$ErrorActionPreference;$ErrorActionPreference='Continue'
    try{& $file @arguments 2>&1|ForEach-Object{$_.ToString()};$exit=$LASTEXITCODE}finally{$ErrorActionPreference=$previous}
    if($exit){throw "VM test tool exited $exit"}
}
if((Get-Process -Id $PID).SessionId -eq 0){
    if($InteractiveChild){throw 'Expected an interactive VM desktop'}
    $id=[guid]::NewGuid().ToString('N');$root=Join-Path $env:LOCALAPPDATA ('DesktopEnvironment\workspace-vm-results\'+$id)
    $null=New-Item -ItemType Directory -Path $root -Force
    $result=Join-Path $root 'result.json';$log=Join-Path $root 'output.log'
    $command="& '$($PSCommandPath.Replace("'","''"))' -InteractiveChild -ResultPath '$($result.Replace("'","''"))' *>&1 | Out-File -LiteralPath '$($log.Replace("'","''"))' -Encoding utf8"
    $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $name='DesktopEnvironment-Workspace-'+$id
    $action=New-ScheduledTaskAction -Execute "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -EncodedCommand $encoded"
    $principal=New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited
    $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 40) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    try{
        Register-ScheduledTask -TaskName $name -Action $action -Principal $principal -Settings $settings|Out-Null;Start-ScheduledTask -TaskName $name
        $deadline=[DateTime]::UtcNow.AddMinutes(40);$startup=[DateTime]::UtcNow.AddSeconds(45)
        $seenRunning=$false
        $printedLines=0
        while(!(Test-Path -LiteralPath $result)){
            if(Test-Path -LiteralPath $log){$lines=@(Get-Content -LiteralPath $log);for($line=$printedLines;$line -lt $lines.Count;$line++){Write-Output $lines[$line]};$printedLines=$lines.Count}
            $taskState=(Get-ScheduledTask -TaskName $name).State
            if($taskState -eq 'Running'){$seenRunning=$true}
            if($seenRunning -and $taskState -ne 'Running'){throw "VM test action ended without a result; log: $log"}
            if(!$seenRunning -and [DateTime]::UtcNow -gt $startup){throw 'Log into the codex VM desktop to run native UI tests'}
            if([DateTime]::UtcNow -gt $deadline){throw 'VM test time limit exceeded'}
            Start-Sleep -Seconds 2
        }
        Get-Content -LiteralPath $log | Select-Object -Skip $printedLines
        $data=Get-Content -LiteralPath $result -Raw|ConvertFrom-Json;if(!$data.ok){throw $data.error}
    }finally{
        if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue;Unregister-ScheduledTask -TaskName $name -Confirm:$false}
    }
    return
}
$result=@{ok=$false};$engine=$null
try{
    # Clean up the known prior test daemon gracefully; never touch production ports.
    $oldOrigin='http://127.0.0.1:4198'
    try{
        $bootstrap=Invoke-RestMethod ($oldOrigin+'/api/bootstrap') -TimeoutSec 2
        if($bootstrap.mode -eq 'native' -and $bootstrap.version -eq 1){
            $headers=@{'X-Workspace-Token'=$bootstrap.token}
            $body=@{id=[guid]::NewGuid().ToString();command=@{kind='snapshot'}}|ConvertTo-Json -Compress
            $snapshot=Invoke-RestMethod ($oldOrigin+'/api/command') -Method Post -ContentType 'application/json' -Headers $headers -Body $body -TimeoutSec 5
            $ownedPrefix=[IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'DesktopEnvironment\workspace-shell-test-state'))+'\'
            if($snapshot.ok -and ([IO.Path]::GetFullPath($snapshot.data.state.configRoot)).StartsWith($ownedPrefix,[StringComparison]::OrdinalIgnoreCase)){
                $body=@{id=[guid]::NewGuid().ToString();command=@{kind='shutdown'}}|ConvertTo-Json -Compress
                $null=Invoke-RestMethod ($oldOrigin+'/api/command') -Method Post -ContentType 'application/json' -Headers $headers -Body $body -TimeoutSec 10
                Write-Output 'Stopped prior VM test daemon through its authenticated shutdown command'
            }
        }
    }catch{Write-Output 'Prior VM test daemon is absent or not reachable'}
    $testRoot=Join-Path $env:LOCALAPPDATA ('DesktopEnvironment\workspace-install-tests\'+[guid]::NewGuid().ToString('N'))
    $installed=Join-Path $testRoot 'installed'
    & (Join-Path $PSScriptRoot 'install-desktop-workspace.ps1') -InstallRoot $installed -PackageRoot $package -SkipBuild
    $env:DESKTOP_WORKSPACE_VM_INSTALL=$installed
    $env:ELECTRON_RUN_AS_NODE='1'
    Push-Location (Join-Path $source 'apps\desktop-workspace')
    try{
        Run-Tool $electron @('--test','src/workspace.test.ts','server.test.mjs')
        $enginePath=Join-Path ${env:ProgramFiles(x86)} 'VB\Voicemeeter\voicemeeterpro.exe'
        if(!(Get-Process -Name voicemeeterpro,voicemeeterpro64 -ErrorAction SilentlyContinue)){
            if(!(Test-Path -LiteralPath $enginePath)){throw 'VM Voicemeeter fixture is missing'}
            $engine=Start-Process -FilePath $enginePath -WindowStyle Hidden -PassThru;Start-Sleep -Seconds 4
        }
        Run-Tool $electron @('electron-vm.test.mjs')
        # Electron test already checks a new version install while both UI/jobs run.
        & (Join-Path $PSScriptRoot 'install-desktop-workspace.ps1') -InstallRoot $installed -PackageRoot $package -SkipBuild
    }finally{Pop-Location}
    $cargo=Get-Command cargo -ErrorAction SilentlyContinue
    if(!$cargo){
        $mise=Join-Path $env:LOCALAPPDATA 'DesktopEnvironment\bin\mise.exe'
        Push-Location $source
        try{$cargoPath=& $mise which cargo;if($LASTEXITCODE){throw 'Unable to locate guest Rust toolchain'}}finally{Pop-Location}
        $cargo=Get-Command ([string]$cargoPath) -ErrorAction Stop
    }
    Write-Output ('VM Rust toolchain: '+$cargo.Source)
    $previousPath=$env:PATH
    $env:PATH=(Split-Path $cargo.Source -Parent)+';'+$env:PATH
    Push-Location $source
    try{
        Run-Tool $cargo.Source @('test','--target-dir',(Join-Path $env:LOCALAPPDATA 'DesktopEnvironment\vm-test-target'),'-p','audio-cable-core','-p','desktop-workspace-service','-j','1')
        Run-Tool $cargo.Source @('build','--target-dir',(Join-Path $env:LOCALAPPDATA 'DesktopEnvironment\vm-test-target'),'-p','audio-output-router','-j','1')
        & (Join-Path $PSScriptRoot 'test-desktop-control-vm.ps1') -AudioOnly
    }finally{Pop-Location;$env:PATH=$previousPath}
    $result.ok=$true
}catch{$result.error=$_.Exception.Message;throw}
finally{
    if($engine -and !$engine.HasExited){$engine.Kill();$engine.WaitForExit()}
    $env:ELECTRON_RUN_AS_NODE=$null
    if($ResultPath){[IO.File]::WriteAllText($ResultPath,($result|ConvertTo-Json),[Text.UTF8Encoding]::new($false))}
}
