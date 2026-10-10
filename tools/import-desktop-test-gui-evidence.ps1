param([Parameter(Mandatory)] [string] $LogPath)
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$log = (Resolve-Path -LiteralPath $LogPath).Path
$responseRoot = [IO.Path]::GetFullPath((Join-Path $workspace 'target/vm-session/responses')) + [IO.Path]::DirectorySeparatorChar
if (-not $log.StartsWith($responseRoot, [StringComparison]::OrdinalIgnoreCase) -or (Get-Item -LiteralPath $log).Length -gt 50MB) { throw 'Expected a bounded local VM worker response log.' }
$receipt = Get-Content -LiteralPath ([IO.Path]::ChangeExtension($log,'.json')) -Raw | ConvertFrom-Json
if (-not $receipt.ok) { throw 'The VM worker did not report success.' }
$count = 0
foreach ($line in (Get-Content -LiteralPath $log)) {
    if (-not $line.StartsWith('VM_GUI_SCREENSHOT|')) { continue }
    $parts = $line -split '\|',4
    if ($parts.Count -ne 4 -or $parts[1] -notmatch '^[a-fA-F0-9-]{36}$' -or $parts[2] -notmatch '^[a-f0-9]{64}$' -or $parts[3].Length -gt 2800000) { throw 'Invalid GUI evidence envelope.' }
    $bytes = [Convert]::FromBase64String($parts[3])
    if ($bytes.Length -lt 8 -or $bytes.Length -gt 2MB -or [BitConverter]::ToString($bytes,0,8) -ne '89-50-4E-47-0D-0A-1A-0A') { throw 'Invalid PNG viewport.' }
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','').ToLowerInvariant() } finally { $sha.Dispose() }
    if ($hash -ne $parts[2]) { throw 'GUI screenshot SHA256 mismatch.' }
    $destination = Join-Path $workspace ('target/vm-gui-evidence/' + $parts[1])
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $destination 'panel.png'), $bytes)
    Write-Output ('Imported verified native guest GUI viewport: ' + (Join-Path $destination 'panel.png'))
    $count += 1
}
if ($count -ne 1) { throw 'Expected exactly one native GUI viewport.' }
