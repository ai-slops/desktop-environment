$ErrorActionPreference = 'Stop'
$computer = Get-CimInstance Win32_ComputerSystem
if (($computer.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU|HVM domU|Parallels') -and
    ($computer.Manufacturer -notmatch '^QEMU$|^VMware|^innotek GmbH$')) { throw 'Guest cleanup is only permitted inside a Windows VM.' }

# These roots belong to the current guest account. Never remove installed tools,
# driver fixtures, build caches, user settings, or manually named source checkouts.
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $env:USERPROFILE 'DesktopEnvironment'))
$testRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'DesktopEnvironment'))
foreach ($root in @($sourceRoot, $testRoot)) {
    if ((Test-Path -LiteralPath $root) -and ((Get-Item -LiteralPath $root).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Refusing linked guest test roots.' }
}
$targets = @()
if (Test-Path -LiteralPath $sourceRoot) {
    $targets += @(Get-ChildItem -LiteralPath $sourceRoot -Directory |
        Where-Object Name -Match '^source-[a-f0-9]{64}$' | ForEach-Object FullName)
    $targets += Join-Path $sourceRoot 'source.zip'
    $targets += Join-Path $sourceRoot 'setup-guest.ps1'
}
$targets += Join-Path $testRoot 'audio-signal-tests'
$targets += Join-Path $testRoot 'interactive-tests'
foreach ($target in $targets) {
    $absolute = [IO.Path]::GetFullPath($target)
    if (-not ($absolute.StartsWith($sourceRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or
        $absolute.StartsWith($testRoot + '\', [StringComparison]::OrdinalIgnoreCase))) { throw 'Cleanup path escaped guest test roots.' }
    if (Test-Path -LiteralPath $absolute) {
        # Reject nested links as well as a linked root before any recursive removal.
        $items = @((Get-Item -LiteralPath $absolute))
        $items += @(Get-ChildItem -LiteralPath $absolute -Recurse -Force)
        if ($items | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }) { throw 'Refusing cleanup of linked test artifacts.' }
        Remove-Item -LiteralPath $absolute -Recurse -Force
        Write-Output "Removed guest test artifact: $absolute"
    }
}
