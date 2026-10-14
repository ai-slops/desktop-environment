param(
    [int]$Port = 4178,
    [switch]$Prepare,
    [switch]$NoBrowser,
    [string]$ConfigRoot,
    [string]$BinariesDirectory,
    [string]$InstallRoot,
    [ValidateSet('audio','cable','windows','diagnostics')] [string]$Panel,
    [switch]$Development
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($Port -lt 1 -or $Port -gt 65535) { throw 'Port must be between 1 and 65535.' }
if(!$Development -and !$BinariesDirectory){
    if(!$InstallRoot){$InstallRoot=if($env:XDG_DATA_HOME){Join-Path $env:XDG_DATA_HOME 'desktop-workspace'}else{Join-Path $env:LOCALAPPDATA 'Programs\DesktopWorkspace'}}
    $installed=Join-Path $InstallRoot 'DesktopWorkspace.exe'
    if($Prepare -or !(Test-Path -LiteralPath $installed)){ & (Join-Path $PSScriptRoot 'install-desktop-workspace.ps1') -InstallRoot $InstallRoot }
    $nativeArguments=@('--port',"$Port")
    if($NoBrowser){$nativeArguments+='--daemon'}
    if($ConfigRoot){$nativeArguments+=@('--config-root',('"'+[IO.Path]::GetFullPath($ConfigRoot).TrimEnd('\')+'"'))}
    if($Panel){$nativeArguments+=@('--panel',$Panel)}
    Start-Process -FilePath $installed -ArgumentList $nativeArguments -WindowStyle Hidden
    Write-Output "Installed Desktop Workspace launched: $installed"
    return
}
$web = Join-Path $repo 'apps\desktop-workspace'
$work = Join-Path $repo 'target\desktop-workspace'
$build = Join-Path $repo 'target\desktop-workspace-build'
if ($Port -lt 1 -or $Port -gt 65535) { throw 'Port must be between 1 and 65535.' }
$url = "http://127.0.0.1:$Port"
function Native-State {
    $bootstrap = Invoke-RestMethod "$url/api/bootstrap" -TimeoutSec 3
    if ($bootstrap.version -ne 1 -or $bootstrap.mode -ne 'native') { throw 'This port belongs to another service.' }
    $body = @{ id = [guid]::NewGuid().ToString(); command = @{ kind = 'snapshot' } } | ConvertTo-Json -Compress
    $response = Invoke-RestMethod "$url/api/command" -Method Post -ContentType 'application/json' -Headers @{ 'X-Workspace-Token' = $bootstrap.token } -Body $body -TimeoutSec 10
    if (!$response.ok) { throw $response.error.message }
    if ($ConfigRoot -and [IO.Path]::GetFullPath($response.data.state.configRoot) -ne [IO.Path]::GetFullPath($ConfigRoot)) { throw 'An existing workspace is using a different settings directory.' }
    return $response.data.state
}
$occupied = $false
try { $null = Invoke-WebRequest "$url/api/bootstrap" -UseBasicParsing -TimeoutSec 2; $occupied = $true } catch { }
if ($occupied) {
    if ($Prepare) { throw 'Stop the development service with 서비스 중지 before preparing a new development build.' }
    $state = Native-State
    Write-Host "Desktop Workspace already running: $url"
    if (!$NoBrowser) { Start-Process $url }
    return
}
$nodeCommand = Get-Command node -ErrorAction SilentlyContinue
$node = if ($nodeCommand) { $nodeCommand.Source } else { Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin\node.exe' }
if (!(Test-Path -LiteralPath $node)) { throw 'Node.js 22.12 or newer is required.' }
& $node --input-type=module -e "const [a,b]=process.versions.node.split('.').map(Number);if(a<22||(a===22&&b<12))process.exit(1)"
if ($LASTEXITCODE -ne 0) { throw 'Node.js 22.12 or newer is required.' }
if (!$BinariesDirectory -and !$Prepare -and (Test-Path -LiteralPath (Join-Path $work 'binaries.json'))) {
    $BinariesDirectory = (Get-Content -LiteralPath (Join-Path $work 'binaries.json') -Raw | ConvertFrom-Json).directory
}
if (!$BinariesDirectory) { $BinariesDirectory = Join-Path $build 'debug' }
$BinariesDirectory = [IO.Path]::GetFullPath($BinariesDirectory)
$names = @('desktop-workspace-host.exe', 'window-manager.exe', 'display-relay.exe')
$missing = @($names | Where-Object { !(Test-Path -LiteralPath (Join-Path $BinariesDirectory $_)) })
if ($Prepare -or $missing.Count) {
    if ($BinariesDirectory -ne (Join-Path $build 'debug')) { throw 'Build the requested BinariesDirectory first, or omit this argument to build automatically.' }
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (!$cargo) { throw 'Install the Rust toolchain or provide -BinariesDirectory with the three built executables.' }
    Push-Location $repo
    try {
        & $cargo.Source build --locked -j 1 --target-dir $build -p desktop-workspace-host -p window-manager -p display-relay
        if ($LASTEXITCODE -ne 0) { throw 'Native build failed.' }
    } finally { Pop-Location }
}
if ($Prepare -or !(Test-Path (Join-Path $web 'dist\index.html'))) {
    $pnpmCommand = Get-Command pnpm -ErrorAction SilentlyContinue
    $pnpm = if ($pnpmCommand) { $pnpmCommand.Source } else { Join-Path $env:USERPROFILE '.cache\codex-runtimes\codex-primary-runtime\dependencies\bin\fallback\pnpm.cmd' }
    if (!(Test-Path -LiteralPath $pnpm)) { throw 'pnpm is required to build the web UI.' }
    Push-Location $web
    try {
        & $pnpm install --frozen-lockfile
        if ($LASTEXITCODE -ne 0) { throw 'Web dependency install failed.' }
        & $pnpm build
        if ($LASTEXITCODE -ne 0) { throw 'Web build failed.' }
    } finally { Pop-Location }
}
# Run copies, so a later build cannot overwrite an executable in use.
$runtimeKey = ($names | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $BinariesDirectory $_) -Algorithm SHA256).Hash.Substring(0, 12) }) -join '-'
$runtime = Join-Path $work ('runs\' + $runtimeKey)
$null = New-Item -ItemType Directory -Path $runtime -Force
foreach ($name in $names) {
    $destination = Join-Path $runtime $name
    if (!(Test-Path -LiteralPath $destination)) { Copy-Item -LiteralPath (Join-Path $BinariesDirectory $name) -Destination $destination }
}
@{ directory = $BinariesDirectory } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $work 'binaries.json') -Encoding UTF8
$stdout = Join-Path $runtime "server-$Port.log"
$stderr = Join-Path $runtime "server-$Port-error.log"
function Quote-Argument([string]$value) {
    if ($value.Contains('"')) { throw 'An argument contains an unsupported quote.' }
    return '"' + $value.TrimEnd('\') + '"'
}
$arguments = @((Quote-Argument (Join-Path $web 'server.mjs')), '--port', "$Port", '--host-exe', (Quote-Argument (Join-Path $runtime 'desktop-workspace-host.exe')))
if ($ConfigRoot) { $arguments += @('--config-root', (Quote-Argument ([IO.Path]::GetFullPath($ConfigRoot)))) }
$serverProcess = Start-Process -FilePath $node -ArgumentList $arguments -WorkingDirectory $web -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
$record = @{ pid = $serverProcess.Id; url = $url; runtime = $runtime; executable = $node; started = $serverProcess.StartTime.ToUniversalTime().ToString('o') }
$record | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $work "server-$Port.json") -Encoding UTF8
$ready = $false
for ($attempt = 0; $attempt -lt 50; $attempt++) {
    if ($serverProcess.HasExited) { throw "Workspace stopped. Read $stderr" }
    try { $state = Native-State; $ready = $true; break } catch { Start-Sleep -Milliseconds 200 }
}
if (!$ready) { Stop-Process -Id $serverProcess.Id -ErrorAction SilentlyContinue; throw "Workspace did not become ready. Read $stderr" }
Write-Host "Desktop Workspace ready: $url"
Write-Host "Settings: $($state.configRoot)"
Write-Host "Logs: $runtime"
Write-Host 'Closing browser tabs preserves jobs. Use 앱 종료 to stop the service and its jobs.'
if (!$NoBrowser) { Start-Process $url }
