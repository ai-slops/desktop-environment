param([switch] $Install)
$ErrorActionPreference = 'Stop'
$computer = Get-CimInstance Win32_ComputerSystem
if (($computer.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -and
    ($computer.Manufacturer -notmatch '^QEMU$|^VMware|^innotek GmbH$')) { throw 'Audio fixtures may only be prepared inside a Windows VM.' }
$root = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/audio-fixtures'
New-Item -ItemType Directory -Path $root -Force | Out-Null
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$packages = @(
    @{ name='VBCABLE'; url='https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip'; setup='VBCABLE_Setup_x64.exe'; device='VB-Audio Virtual Cable' },
    @{ name='Voicemeeter'; url='https://download.vb-audio.com/Download_CABLE/VoicemeeterSetup_v2130.zip'; setup='VoicemeeterSetup.exe'; device='VB-Audio.*VoiceMeeter|VoiceMeeter.*VAIO' }
)
$changed = $false
foreach ($package in $packages) {
    $marker = Join-Path $root ($package.name + '-installed.json')
    if (Test-Path -LiteralPath $marker) { Write-Output "$($package.name) already installed by this fixture; preserving it."; continue }
    if (Get-CimInstance Win32_PnPEntity | Where-Object Name -Match $package.device) {
        Write-Output "$($package.name) already present; preserving the existing installation."
        continue
    }
    $zip = Join-Path $root ($package.name + '.zip')
    if (-not (Test-Path -LiteralPath $zip)) {
        Write-Output "Downloading $($package.name) from its official publisher."
        Invoke-WebRequest -UseBasicParsing $package.url -OutFile $zip
    }
    $directory = Join-Path $root $package.name
    Expand-Archive -LiteralPath $zip -DestinationPath $directory -Force
    $setup = @(Get-ChildItem -LiteralPath $directory -Recurse -File -Filter $package.setup)
    if ($setup.Count -ne 1) { throw "Expected one $($package.setup) installer." }
    $signature = Get-AuthenticodeSignature -LiteralPath $setup[0].FullName
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'BUREL|VB.?AUDIO') {
        throw "Invalid publisher signature for $($package.name)."
    }
    Write-Output "Prepared signed $($package.name) installer: $($setup[0].FullName)"
    if (-not $Install) { continue }
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Guest audio driver installation requires guest Administrator rights.' }
    # Personal development fixture, not part of the application's installer.
    # Publisher setup handles its own signed driver and root device registration.
    $installation = Start-Process -FilePath $setup[0].FullName -WorkingDirectory $setup[0].DirectoryName -WindowStyle Hidden -ArgumentList @('-i', '-h') -PassThru
    if (-not $installation.WaitForExit(180000)) { throw "Guest $($package.name) installer did not complete within three minutes; inspect it in the guest." }
    if ($installation.ExitCode -notin @(0, 3010)) { throw "Guest $($package.name) installer exit: $($installation.ExitCode)" }
    @{ package=$package.name; sha256=(Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash; signer=$signature.SignerCertificate.Subject; boot=(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString('o') } |
        ConvertTo-Json | Set-Content -LiteralPath $marker
    $changed = $true
}
if ($changed) { 'Audio fixture installation completed. Restart ONLY this Windows guest before testing.' }
return $changed
