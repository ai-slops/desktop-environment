param(
    [ValidateSet('Status', 'SyncSource', 'Setup', 'Test', 'WorkspaceTest', 'Stop', 'Shutdown', 'ShutdownAndCleanup')]
    [string] $Action = 'Status'
)
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$workerRoot = Join-Path $workspace 'target/vm-session'
$statusPath = Join-Path $workerRoot 'status.json'
if (-not (Test-Path -LiteralPath $statusPath)) { throw 'Start the scoped elevated VM worker first.' }
$status = Get-Content -LiteralPath $statusPath -Raw | ConvertFrom-Json
# PowerShell 7 converts JSON timestamps to DateTime automatically. Parsing that
# value again via its culture-specific string loses the UTC kind/time zone.
$expiresUtc = if ($status.expiresUtc) { ([DateTime]$status.expiresUtc).ToUniversalTime() } else { [DateTime]::MinValue }
if (-not $status.pid -or $expiresUtc -le [DateTime]::UtcNow) {
    throw 'VM worker is stopped or expired; start it again with UAC.'
}
$owner = Get-Content -LiteralPath (Join-Path $workspace 'target/vm-hyperv/owner.json') -Raw | ConvertFrom-Json
if ($status.vmId -ne $owner.id) { throw 'Worker VM identity does not match this checkout.' }
$requestId = [guid]::NewGuid().ToString()
$requestDirectory = Join-Path $workerRoot 'requests'
$temporary = Join-Path $requestDirectory ($requestId + '.tmp')
$requestPath = Join-Path $requestDirectory ($requestId + '.json')
@{ action = $Action } | ConvertTo-Json | Set-Content -LiteralPath $temporary
Move-Item -LiteralPath $temporary -Destination $requestPath
[ordered]@{
    id = $requestId; action = $Action
    result = Join-Path $workerRoot "responses/$requestId.json"
    log = Join-Path $workerRoot "responses/$requestId.log"
} | ConvertTo-Json
