# Pure filesystem checks: no VM is started and no host system state is changed.
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$targetRoot = [IO.Path]::GetFullPath((Join-Path $workspace 'target'))
$fixtureRoot = [IO.Path]::GetFullPath((Join-Path $targetRoot ('vm-cleanup-check-' + [guid]::NewGuid())))
if (-not $fixtureRoot.StartsWith($targetRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid fixture root.' }
$previousProfile = $env:USERPROFILE
$previousLocal = $env:LOCALAPPDATA
try {
    # The helper resolves only this child process's isolated test account paths.
    $env:USERPROFILE = Join-Path $fixtureRoot 'user'
    $env:LOCALAPPDATA = Join-Path $fixtureRoot 'local'
    $sourceRoot = Join-Path $env:USERPROFILE 'DesktopEnvironment'
    $testRoot = Join-Path $env:LOCALAPPDATA 'DesktopEnvironment'
    $plan = & (Join-Path $PSScriptRoot 'desktop-test-vm.ps1') -Action Plan -IsoPath '' | ConvertFrom-Json
    if ($plan.memoryGiB -ne 32 -or $plan.processors -ne 12 -or $plan.iso) { throw 'Planning without Downloads failed.' }
    $downloads = Join-Path $env:USERPROFILE 'Downloads'
    New-Item -ItemType Directory -Path $downloads -Force | Out-Null
    foreach ($name in @('Win11_A_x64.iso', 'Win11_B_x64.iso')) { Set-Content -LiteralPath (Join-Path $downloads $name) -Value 'fixture' }
    $refused = $false
    try { & (Join-Path $PSScriptRoot 'desktop-test-vm.ps1') -Action Setup -IsoPath '' } catch { $refused = $_.Exception.Message.StartsWith('Multiple Windows ISOs found.') }
    if (-not $refused) { throw 'Ambiguous ISOs were not rejected before VM access.' }
    $snapshot = Join-Path $sourceRoot ('source-' + ('a' * 64))
    $removed = @(
        (Join-Path $snapshot 'tools/test.txt'),
        (Join-Path $sourceRoot 'source.zip'),
        (Join-Path $sourceRoot 'setup-guest.ps1'),
        (Join-Path $testRoot 'audio-signal-tests/recording.wav'),
        (Join-Path $testRoot 'interactive-tests/output.log')
    )
    $preserved = @(
        (Join-Path $sourceRoot 'source/my-code.txt'),
        (Join-Path $sourceRoot 'source-custom/my-code.txt'),
        (Join-Path $testRoot 'bin/mise.exe'),
        (Join-Path $testRoot 'vm-test-target/cache.txt'),
        (Join-Path $testRoot 'audio-fixtures/driver.txt'),
        (Join-Path $testRoot 'audio-cable/settings.json')
    )
    foreach ($file in ($removed + $preserved)) {
        New-Item -ItemType Directory -Path (Split-Path $file -Parent) -Force | Out-Null
        Set-Content -LiteralPath $file -Value 'fixture'
    }
    function Get-CimInstance { param($ClassName) [pscustomobject]@{ Model = 'Physical PC'; Manufacturer = 'Fixture' } }
    $refused = $false
    try { & (Join-Path $PSScriptRoot 'cleanup-desktop-test-guest.ps1') } catch { $refused = $_.Exception.Message -eq 'Guest cleanup is only permitted inside a Windows VM.' }
    if (-not $refused) { throw 'Physical host guard did not reject cleanup.' }
    foreach ($file in ($removed + $preserved)) {
        if (-not (Test-Path -LiteralPath $file)) { throw 'Host rejection changed a fixture file.' }
    }
    function Get-CimInstance { param($ClassName) [pscustomobject]@{ Model = 'Virtual Machine'; Manufacturer = 'Fixture' } }
    & (Join-Path $PSScriptRoot 'cleanup-desktop-test-guest.ps1') | Out-Null
    foreach ($file in $removed) {
        if (Test-Path -LiteralPath $file) { throw "Generated artifact was not removed: $file" }
    }
    foreach ($file in $preserved) {
        if (-not (Test-Path -LiteralPath $file)) { throw "Persistent artifact was removed: $file" }
    }
    & (Join-Path $PSScriptRoot 'cleanup-desktop-test-guest.ps1') | Out-Null
    Write-Output 'PASS: portable ISO planning, host guard, generated artifact cleanup, persistent data preservation, repeated cleanup.'
} finally {
    $env:USERPROFILE = $previousProfile
    $env:LOCALAPPDATA = $previousLocal
    if (Test-Path -LiteralPath $fixtureRoot) { Remove-Item -LiteralPath $fixtureRoot -Recurse -Force }
}
