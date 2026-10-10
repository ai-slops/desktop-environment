param(
    [Parameter(Mandatory)] [string] $CredentialPath,
    [ValidateRange(1, 12)] [int] $LifetimeHours = 8
)
$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Start this VM worker with UAC elevation once.' }
Import-Module Hyper-V
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$workerRoot = Join-Path $workspace 'target/vm-session'
$requests = Join-Path $workerRoot 'requests'
$responses = Join-Path $workerRoot 'responses'
New-Item -ItemType Directory -Path $requests, $responses -Force | Out-Null
$owner = Get-Content -LiteralPath (Join-Path $workspace 'target/vm-hyperv/owner.json') -Raw | ConvertFrom-Json
$vm = Get-VM -Id ([guid]$owner.id)
if ($vm.Name -ne 'DesktopEnvironment-AudioTest') { throw 'VM ownership mismatch.' }
$credential = Import-Clixml -LiteralPath $CredentialPath
if ($credential -isnot [PSCredential]) { throw 'Expected a Windows DPAPI-protected credential file.' }
$mutex = [Threading.Mutex]::new($false, ('Local\DesktopEnvironmentVmWorker-' + $vm.Id))
if (-not $mutex.WaitOne(0)) { $mutex.Dispose(); throw 'The worker for this VM is already running.' }
$session = $null
$guestSource = $null
$deadline = [DateTime]::UtcNow.AddHours($LifetimeHours)
$stopping = $false
try {
    while (-not $stopping -and [DateTime]::UtcNow -lt $deadline) {
        @{ pid = $PID; vmId = $vm.Id.ToString(); expiresUtc = $deadline.ToString('o'); heartbeatUtc = [DateTime]::UtcNow.ToString('o') } |
            ConvertTo-Json | Set-Content -LiteralPath (Join-Path $workerRoot 'status.json')
        foreach ($requestFile in @(Get-ChildItem -LiteralPath $requests -Filter '*.json' -File | Sort-Object LastWriteTime, Name)) {
            if ($requestFile.BaseName -notmatch '^[a-fA-F0-9-]{36}$') { continue }
            $requestId = $requestFile.BaseName
            $logPath = Join-Path $responses ($requestId + '.log')
            $result = @{ id = $requestId; ok = $false }
            try {
                $request = Get-Content -LiteralPath $requestFile.FullName -Raw | ConvertFrom-Json
                # A closed action set: requests cannot supply host commands, paths,
                # guest code, credentials or a different VM identity.
                if ($request.action -notin @('Status', 'SyncSource', 'Setup', 'Test', 'WorkspaceTest', 'Stop')) { throw 'Unknown VM worker action.' }
                $result.action = $request.action
                $currentVm = Get-VM -Id $vm.Id
                if ($currentVm.Name -ne $owner.name) { throw 'Owned VM identity changed.' }
                if ($request.action -eq 'Stop') { $stopping = $true }
                elseif ($request.action -eq 'Status') {
                    $result.state = $currentVm.State.ToString()
                    $result.guestSource = [string]$guestSource
                } else {
                    if ($currentVm.State -ne 'Running') { throw 'Start the owned VM before submitting guest operations.' }
                    if (-not $session -or $session.State -ne 'Opened') {
                        if ($session) { Remove-PSSession $session }
                        $session = New-PSSession -VMId $vm.Id -Credential $credential
                    }
                    if ($request.action -eq 'SyncSource') {
                        $zip = Join-Path $workspace 'target/desktop-control-vm-source.zip'
                        $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
                        $guestRoot = Invoke-Command -Session $session -ScriptBlock {
                            $computer = Get-CimInstance Win32_ComputerSystem
                            if ($computer.Model -ne 'Virtual Machine') { throw 'Expected Hyper-V Windows guest.' }
                            $root = Join-Path $env:USERPROFILE 'DesktopEnvironment'
                            New-Item -ItemType Directory -Path $root -Force | Out-Null
                            $root
                        }
                        Copy-Item -LiteralPath $zip -Destination "$guestRoot\source.zip" -ToSession $session
                        Copy-Item -LiteralPath (Join-Path $workspace 'tools/setup-desktop-test-guest.ps1') -Destination "$guestRoot\setup-guest.ps1" -ToSession $session
                        $guestSource = [string](Invoke-Command -Session $session -ArgumentList $guestRoot, $hash -ScriptBlock {
                            param($root, $expectedHash)
                            if ((Get-FileHash -LiteralPath "$root\source.zip" -Algorithm SHA256).Hash -ne $expectedHash) { throw 'Guest source transfer checksum mismatch.' }
                            $source = Join-Path $root ('source-' + $expectedHash)
                            if (-not (Test-Path -LiteralPath $source)) { Expand-Archive -LiteralPath "$root\source.zip" -DestinationPath $source }
                            $source
                        })
                        $result.guestSource = $guestSource
                    } else {
                        if (-not $guestSource) { throw 'Submit SyncSource before guest setup/tests.' }
                        Invoke-Command -Session $session -ArgumentList $guestSource, $request.action -ScriptBlock {
                            param($source, $action)
                            $ErrorActionPreference = 'Stop'
                            $computer = Get-CimInstance Win32_ComputerSystem
                            if ($computer.Model -ne 'Virtual Machine') { throw 'Expected Hyper-V Windows guest.' }
                            function Invoke-GuestTool {
                                param([string] $File, [string[]] $Arguments)
                                $previousPreference = $ErrorActionPreference
                                $ErrorActionPreference = 'Continue'
                                try {
                                    & $File @Arguments 2>&1 | ForEach-Object { $_.ToString() }
                                    $toolExitCode = $LASTEXITCODE
                                } finally { $ErrorActionPreference = $previousPreference }
                                if ($toolExitCode -ne 0) { throw "Guest command exited with $toolExitCode" }
                            }
                            $mise = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/bin/mise.exe'
                            if ($action -eq 'Setup') {
                                $setup = Join-Path $env:USERPROFILE 'DesktopEnvironment/setup-guest.ps1'
                                Invoke-GuestTool powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $setup, '-Workspace', $source)
                            } else {
                                Push-Location $source
                                try {
                                    if ($action -eq 'Test') {
                                        Invoke-GuestTool $mise @('exec', '--', 'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $source 'tools/test-desktop-control-vm.ps1'))
                                    } else {
                                        Invoke-GuestTool $mise @('exec', '--', 'powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $source 'tools/test-desktop-control-vm.ps1'))
                                    }
                                } finally { Pop-Location }
                            }
                        } *>&1 | Out-File -LiteralPath $logPath -Encoding utf8
                    }
                }
                $result.ok = $true
            } catch {
                $_ | Out-String | Add-Content -LiteralPath $logPath
                $result.error = $_.Exception.Message
            }
            $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $responses ($requestId + '.json'))
            Remove-Item -LiteralPath $requestFile.FullName
            if ($stopping) { break }
        }
        if (-not $stopping) { Start-Sleep -Seconds 2 }
    }
} finally {
    if ($session) { Remove-PSSession $session }
    @{ stoppedUtc = [DateTime]::UtcNow.ToString('o'); vmId = $vm.Id.ToString() } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $workerRoot 'status.json')
    $mutex.ReleaseMutex()
    $mutex.Dispose()
}
