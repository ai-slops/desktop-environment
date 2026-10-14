param([string]$InstallRoot,[string]$PackageRoot,[switch]$SkipBuild,[switch]$BuildOnly)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$defaultInstall=!$InstallRoot
if(!$InstallRoot){$InstallRoot=if($env:XDG_DATA_HOME){Join-Path $env:XDG_DATA_HOME 'desktop-workspace'}else{Join-Path $env:LOCALAPPDATA 'Programs\DesktopWorkspace'}}
$InstallRoot=[IO.Path]::GetFullPath($InstallRoot)
if(!$PackageRoot){$PackageRoot=Join-Path $repo 'target\desktop-workspace-package'}
$PackageRoot=[IO.Path]::GetFullPath($PackageRoot)
if(!$SkipBuild){
    $cargoCommand=Get-Command cargo -ErrorAction SilentlyContinue
    $cargo=if($cargoCommand){$cargoCommand.Source}else{Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'}
    if(!(Test-Path -LiteralPath $cargo)){throw 'Rust cargo is required; run mise install or install Rust first'}
    $node=(Get-Command node -ErrorAction Stop).Source
    $pnpm=(Get-Command pnpm -ErrorAction Stop).Source
    Push-Location $repo
    try{ & $cargo build -j 1 --target-dir target/desktop-workspace-build -p desktop-workspace -p desktop-workspace-host -p window-manager -p display-relay
        if($LASTEXITCODE){throw 'Native build failed'}
        Push-Location (Join-Path $repo 'apps\desktop-workspace')
        try{ & $pnpm install --frozen-lockfile; if($LASTEXITCODE){throw 'Dependency install failed'}
            & $node node_modules/electron/install.js; if($LASTEXITCODE){throw 'Electron download failed'}
            & $pnpm build; if($LASTEXITCODE){throw 'Web build failed'}
            & $node package-desktop.mjs $PackageRoot; if($LASTEXITCODE){throw 'Package build failed'}
        }finally{Pop-Location}
    }finally{Pop-Location}
}
if($BuildOnly){Write-Output "Prepared package: $PackageRoot";return}
$package=Get-Content -LiteralPath (Join-Path $PackageRoot 'package.json') -Raw|ConvertFrom-Json
if($package.version -notmatch '^v[a-f0-9]{24}$'){throw 'Invalid package version'}
$destination=Join-Path $InstallRoot ('versions\'+$package.version)
$null=New-Item -ItemType Directory -Path $destination -Force
# Immutable version directories keep builds and updates away from active images.
if(!(Test-Path -LiteralPath (Join-Path $destination '.installed.json'))){
    Get-ChildItem -LiteralPath (Join-Path $PackageRoot ('payload\'+$package.version)) | ForEach-Object {Copy-Item -LiteralPath $_.FullName -Destination $destination -Recurse -Force -ErrorAction Stop}
    [IO.File]::WriteAllText((Join-Path $destination '.installed.json'),('{"version":"'+$package.version+'"}'),[Text.UTF8Encoding]::new($false))
}
$launcher=Join-Path $InstallRoot 'DesktopWorkspace.exe'
$stagedLauncher=Join-Path $InstallRoot ('launcher-'+[guid]::NewGuid().ToString('N')+'.exe')
Copy-Item -LiteralPath (Join-Path $PackageRoot 'DesktopWorkspace.exe') -Destination $stagedLauncher
Move-Item -LiteralPath $stagedLauncher -Destination $launcher -Force
$manifest=Join-Path $InstallRoot ('current-'+[guid]::NewGuid().ToString('N')+'.json')
[IO.File]::WriteAllText($manifest,(@{version=$package.version}|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
Move-Item -LiteralPath $manifest -Destination (Join-Path $InstallRoot 'current.json') -Force
if($defaultInstall){
    $shortcutPath=Join-Path ([Environment]::GetFolderPath('StartMenu')) 'Programs\Desktop Workspace.lnk'
    $shell=New-Object -ComObject WScript.Shell
    $shortcut=$shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath=$launcher
    $shortcut.WorkingDirectory=$InstallRoot
    $shortcut.Save()
}
Write-Output "Installed Desktop Workspace: $launcher"
