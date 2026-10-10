param([Parameter(Mandatory)] [string] $LogPath)
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$log = (Resolve-Path -LiteralPath $LogPath).Path
$responseRoot = [IO.Path]::GetFullPath((Join-Path $workspace 'target/vm-session/responses')) + [IO.Path]::DirectorySeparatorChar
if (-not $log.StartsWith($responseRoot, [StringComparison]::OrdinalIgnoreCase) -or (Get-Item -LiteralPath $log).Length -gt 50MB) { throw 'Expected a bounded local VM worker response log.' }
$cases = @('source-a','source-b','destination-baseline','default-a-to-c','default-b-to-c','default-back-a-to-c','continuous-a-to-b','feedback-paused','feedback-resumed-b','router-stopped','fixed-a-default-b','default-target-a','default-target-b','old-target-a-no-longer-routed')
$received = @{}
$runId = $null
foreach ($line in (Get-Content -LiteralPath $log)) {
    if (-not ($line.StartsWith('VM_AUDIO_EVIDENCE|') -or $line.StartsWith('VM_AUDIO_METRIC|'))) { continue }
    $parts = $line -split '\|', 5
    if ($parts.Count -ne 5 -or $parts[1] -notmatch '^[a-fA-F0-9-]{36}$' -or $parts[2] -notin $cases -or $parts[3] -notmatch '^[a-f0-9]{64}$') { throw 'Invalid evidence envelope.' }
    if ($runId -and $runId -ne $parts[1]) { throw 'Mixed recording runs in one export.' }
    $runId = $parts[1]
    $destination = Join-Path $workspace ('target/vm-audio-evidence/' + $runId)
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    if ($parts[0] -eq 'VM_AUDIO_EVIDENCE') {
        if ($parts[4].Length -gt 2800000) { throw 'Recording exceeds size budget.' }
        $bytes = [Convert]::FromBase64String($parts[4])
        if ($bytes.Length -lt 44 -or $bytes.Length -gt 2MB -or [Text.Encoding]::ASCII.GetString($bytes,0,4) -ne 'RIFF' -or [Text.Encoding]::ASCII.GetString($bytes,8,4) -ne 'WAVE') { throw 'Invalid WAV recording.' }
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $hash = [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','').ToLowerInvariant() } finally { $sha.Dispose() }
        if ($hash -ne $parts[3]) { throw 'Recording SHA256 mismatch.' }
        [IO.File]::WriteAllBytes((Join-Path $destination ($parts[2] + '.wav')), $bytes)
        $received[$parts[2] + '.wav'] = $hash
    } else {
        $metadata = $parts[4] | ConvertFrom-Json
        if ($metadata.wav_sha256 -ne $parts[3] -or $metadata.measurement.case -ne $parts[2]) { throw 'Metric does not match the recording.' }
        [IO.File]::WriteAllText((Join-Path $destination ($parts[2] + '.json')), $parts[4], [Text.UTF8Encoding]::new($false))
        $received[$parts[2] + '.json'] = $parts[3]
    }
}
if ($received.Count -ne 28) { throw 'Expected all 14 recording/metric pairs.' }
$rows = foreach ($case in $cases) {
    if ($received[$case + '.wav'] -ne $received[$case + '.json']) { throw 'Recording/metric hash mismatch.' }
    $metadata = Get-Content -LiteralPath (Join-Path $destination ($case + '.json')) -Raw | ConvertFrom-Json
    $m = $metadata.measurement
    [pscustomobject]@{ case=$case; expected_hz=$m.expected_hz; unwanted_hz=$m.unwanted_hz; expected_amplitude=$m.expected_amplitude; unwanted_amplitude=$m.unwanted_amplitude; unwanted_db=$(if ($m.expected_amplitude -gt 0 -and $m.unwanted_amplitude -gt 0) {20 * [Math]::Log10($m.unwanted_amplitude / $m.expected_amplitude)}); rms=$m.rms; analysis_warmup_ms=$metadata.analysis_warmup_ms; sha256=$metadata.wav_sha256 }
}
$rows | Export-Csv -LiteralPath (Join-Path $destination 'summary.csv') -NoTypeInformation -Encoding UTF8
Write-Output "Imported 14 WAV recordings with verified SHA256 and metrics: $destination"
