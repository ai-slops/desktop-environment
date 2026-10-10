param(
    [string] $Workspace = (Join-Path $env:USERPROFILE 'DesktopEnvironment/source'),
    [switch] $RunTests
)
$ErrorActionPreference = 'Stop'

# This installer must never run on the broadcasting host.
$computer = Get-CimInstance Win32_ComputerSystem
$isGuest = ($computer.Model -match 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -or
    ($computer.Manufacturer -match '^QEMU$|^VMware|^innotek GmbH$')
if (-not $isGuest) { throw 'Guest setup is only permitted inside a Windows VM.' }
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run guest setup from an Administrator PowerShell inside the VM.'
}
if (-not [Environment]::Is64BitProcess) { throw 'Use x64 PowerShell.' }
if (-not (Test-Path -LiteralPath (Join-Path $Workspace 'Cargo.toml'))) { throw 'Extract the source ZIP into Workspace first.' }
$Workspace = (Resolve-Path -LiteralPath $Workspace).Path
$cache = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/guest-setup'
New-Item -ItemType Directory -Path $cache -Force | Out-Null
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'

$miseDirectory = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/bin'
$mise = Join-Path $miseDirectory 'mise.exe'
New-Item -ItemType Directory -Path $miseDirectory -Force | Out-Null
# Pinned official release, also used on the development host. No App Installer
# registration is needed for PowerShell Direct sessions.
$miseHash = '18a35dddfc8be4af44ff983333eee94fa482e53586536dcf625f0d66fedf0026'
if (-not (Test-Path -LiteralPath $mise) -or (Get-FileHash -LiteralPath $mise -Algorithm SHA256).Hash -ne $miseHash) {
    Write-Output 'Downloading mise 2026.9.18 in the guest.'
    $download = Join-Path $cache 'mise-download.exe'
    Invoke-WebRequest -UseBasicParsing 'https://github.com/jdx/mise/releases/download/v2026.9.18/mise-v2026.9.18-windows-x64.exe' -OutFile $download
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $miseHash) { throw 'mise checksum mismatch.' }
    Move-Item -LiteralPath $download -Destination $mise -Force
}
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $miseDirectory) {
    [Environment]::SetEnvironmentVariable('Path', ($miseDirectory + ';' + $userPath), 'User')
}
$env:PATH = $miseDirectory + ';' + $env:PATH

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vcInstallation = $null
if (Test-Path -LiteralPath $vswhere) {
    $vcInstallation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
}
if (-not $vcInstallation) {
    Write-Output 'Installing Microsoft C++ Build Tools and recommended Windows SDK in the guest.'
    $installer = Join-Path $cache 'vs_buildtools.exe'
    Invoke-WebRequest -UseBasicParsing 'https://aka.ms/vs/17/release/vs_buildtools.exe' -OutFile $installer
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw 'C++ installer is not validly signed by Microsoft.'
    }
    $installation = Start-Process -FilePath $installer -WindowStyle Hidden -ArgumentList @('--quiet', '--wait', '--norestart', '--add', 'Microsoft.VisualStudio.Workload.VCTools', '--includeRecommended') -Wait -PassThru
    if ($installation.ExitCode -notin @(0, 3010)) { throw "C++ installer failed: $($installation.ExitCode)" }
    if ($installation.ExitCode -eq 3010) {
        Write-Output 'Guest restart required. Restart the VM and rerun guest setup; the host must remain running.'
        exit 3010
    }
    $vcInstallation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vcInstallation) { throw 'C++ toolchain was not detected after installation.' }
}

& $mise --version
if ($LASTEXITCODE -ne 0) { throw 'mise cannot start after C++ runtime installation.' }
Push-Location $Workspace
try {
    & $mise trust (Join-Path $Workspace 'mise.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Workspace trust failed.' }
    Write-Output 'Installing the workspace Rust and just tools with mise.'
    & $mise install
    if ($LASTEXITCODE -ne 0) { throw 'mise tool installation failed.' }
    & $mise exec -- cargo --version
    if ($LASTEXITCODE -ne 0) { throw 'Guest cargo verification failed.' }
    if ($RunTests) {
        & $mise exec -- powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Workspace 'tools/test-desktop-control-vm.ps1')
        if ($LASTEXITCODE -ne 0) { throw 'Guest regression checks failed.' }
    }
    Write-Output 'Guest toolchain setup completed.'
} finally { Pop-Location }
