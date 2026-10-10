$ErrorActionPreference = 'Stop'
$computer = Get-CimInstance Win32_ComputerSystem
if (($computer.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -and ($computer.Manufacturer -notmatch '^QEMU$|^VMware|^innotek GmbH$')) { throw 'Audio evidence export is VM-only.' }
$root = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment/audio-signal-tests'
if (-not (Test-Path -LiteralPath $root)) { return }
$cases = @('source-a','source-b','destination-baseline','default-a-to-c','default-b-to-c','default-back-a-to-c','feedback-paused','feedback-resumed-b','router-stopped','fixed-a-default-b','default-target-a','default-target-b','old-target-a-no-longer-routed')
foreach ($directory in @(Get-ChildItem -LiteralPath $root -Directory | Sort-Object CreationTimeUtc -Descending)) {
    if ($directory.Name -notmatch '^[a-fA-F0-9-]{36}$' -or ($directory.Attributes -band [IO.FileAttributes]::ReparsePoint)) { continue }
    $records = @()
    foreach ($case in $cases) {
        $wave = Join-Path $directory.FullName ($case + '.wav')
        $metric = Join-Path $directory.FullName ($case + '.json')
        if (-not (Test-Path -LiteralPath $wave) -or -not (Test-Path -LiteralPath $metric)) { break }
        $file = Get-Item -LiteralPath $wave
        $metricFile = Get-Item -LiteralPath $metric
        if ($file.Length -lt 44 -or $file.Length -gt 2MB -or $metricFile.Length -gt 8192 -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -or ($metricFile.Attributes -band [IO.FileAttributes]::ReparsePoint)) { break }
        $measurement = Get-Content -LiteralPath $metric -Raw | ConvertFrom-Json
        if ($measurement.case -ne $case -or $null -eq $measurement.rms -or [double]::IsNaN([double]$measurement.rms) -or [double]::IsInfinity([double]$measurement.rms)) { break }
        if ($case -in @('destination-baseline','feedback-paused','router-stopped')) {
            if ($measurement.rms -gt 0.001) { break }
        } elseif ($measurement.expected_amplitude -lt 0.03 -or $measurement.unwanted_amplitude -gt $measurement.expected_amplitude * 0.01) { break }
        $bytes = [IO.File]::ReadAllBytes($wave)
        if ([Text.Encoding]::ASCII.GetString($bytes,0,4) -ne 'RIFF' -or [Text.Encoding]::ASCII.GetString($bytes,8,4) -ne 'WAVE') { break }
        $hash = (Get-FileHash -LiteralPath $wave -Algorithm SHA256).Hash.ToLowerInvariant()
        $records += [pscustomobject]@{ name=$case; bytes=$bytes; hash=$hash; measurement=$measurement }
    }
    if ($records.Count -ne $cases.Count) { continue }
    foreach ($record in $records) {
        Write-Output ('VM_AUDIO_EVIDENCE|' + $directory.Name + '|' + $record.name + '|' + $record.hash + '|' + [Convert]::ToBase64String($record.bytes))
        $metadata = @{ measurement=$record.measurement; analysis_warmup_ms=250; capture_channel=0; wav_sha256=$record.hash } | ConvertTo-Json -Depth 5 -Compress
        Write-Output ('VM_AUDIO_METRIC|' + $directory.Name + '|' + $record.name + '|' + $record.hash + '|' + $metadata)
    }
    Write-Output ('Exported 13 verified synthetic guest recordings: ' + $directory.Name)
    return
}
Write-Output 'No complete passing signal recording set to export yet.'
