param([int]$Port = 4178)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$url = "http://127.0.0.1:$Port"
try {
    $bootstrap = Invoke-RestMethod "$url/api/bootstrap" -TimeoutSec 3
    if ($bootstrap.mode -ne 'native' -or $bootstrap.version -ne 1) { throw 'The port belongs to another service.' }
    $body = @{ id = [guid]::NewGuid().ToString(); command = @{ kind = 'shutdown' } } | ConvertTo-Json -Compress
    $response = Invoke-RestMethod "$url/api/command" -Method Post -ContentType 'application/json' -Headers @{ 'X-Workspace-Token' = $bootstrap.token } -Body $body -TimeoutSec 15
    if (!$response.ok) { throw $response.error.message }
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        try { $null = Invoke-WebRequest "$url/api/bootstrap" -UseBasicParsing -TimeoutSec 1 } catch { break }
        Start-Sleep -Milliseconds 100
    }
    Write-Host 'Desktop Workspace and its jobs stopped.'
    return
} catch { $shutdownError = $_.Exception.Message }
# Recover only the process recorded by our launcher, after checking PID reuse.
$recordPath = Join-Path $repo "target\desktop-workspace\server-$Port.json"
if (!(Test-Path -LiteralPath $recordPath)) { throw "Cannot stop the workspace: $shutdownError" }
$record = Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json
$serverProcess = Get-Process -Id $record.pid -ErrorAction SilentlyContinue
if (!$serverProcess) { Write-Host 'Desktop Workspace is already stopped.'; return }
if ($serverProcess.Path -ne $record.executable -or $serverProcess.StartTime.ToUniversalTime().ToString('o') -ne $record.started) { throw 'The recorded PID now belongs to a different process; refusing to stop it.' }
Stop-Process -Id $serverProcess.Id
Write-Host 'Stopped the recorded workspace server. Its inherited pipes close and native owned jobs terminate.'
