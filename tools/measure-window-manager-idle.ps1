param(
    [string]$Executable = "$PSScriptRoot/../target/release/window-manager.exe",
    [ValidateRange(10, 300)][int]$Samples = 30,
    [string]$Output = "$PSScriptRoot/../target/window-manager-idle.json"
)
$ErrorActionPreference = 'Stop'
$binary = (Resolve-Path -LiteralPath $Executable).Path
$fixtureRoot = [IO.Path]::GetFullPath("$PSScriptRoot/../target/idle-fixtures")
$null = New-Item -ItemType Directory -Path $fixtureRoot -Force
$fixture = Join-Path $fixtureRoot ([Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $fixture
$config = Join-Path $fixture 'config.json'
$owned = $null
try {
    & $binary --init --config $config | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Fixture initialization failed' }
    $start = [Diagnostics.ProcessStartInfo]::new($binary)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.ArgumentList.Add('--session')
    $start.ArgumentList.Add('--config')
    $start.ArgumentList.Add($config)
    $owned = [Diagnostics.Process]::Start($start)
    $identity = $owned.StartTime.ToUniversalTime()
    # This is an empty, read-only session with no bound windows, providers or shortcuts.
    # Keep stdin open; idle measurements issue no commands and drain both output streams.
    $stdout = $owned.StandardOutput.ReadToEndAsync()
    $stderr = $owned.StandardError.ReadToEndAsync()
    Start-Sleep -Seconds 5
    $cpu = [Collections.Generic.List[double]]::new()
    $working = [Collections.Generic.List[long]]::new()
    $private = [Collections.Generic.List[long]]::new()
    $owned.Refresh()
    $priorCpu = $owned.TotalProcessorTime.TotalMilliseconds
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $priorWall = $clock.Elapsed.TotalMilliseconds
    for ($index = 0; $index -lt $Samples; $index++) {
        Start-Sleep -Seconds 1
        $owned.Refresh()
        if ($owned.HasExited -or $owned.StartTime.ToUniversalTime() -ne $identity) {
            throw 'Owned fixture process ended during measurement'
        }
        $nowCpu = $owned.TotalProcessorTime.TotalMilliseconds
        $nowWall = $clock.Elapsed.TotalMilliseconds
        $cpu.Add(100 * ($nowCpu - $priorCpu) / ($nowWall - $priorWall))
        $working.Add($owned.WorkingSet64)
        $private.Add($owned.PrivateMemorySize64)
        $priorCpu = $nowCpu
        $priorWall = $nowWall
    }
    function Distribution($values) {
        $sorted = @($values | Sort-Object)
        return [ordered]@{
            median = $sorted[[Math]::Ceiling($sorted.Count * 0.50) - 1]
            p95 = $sorted[[Math]::Ceiling($sorted.Count * 0.95) - 1]
            p99 = $sorted[[Math]::Ceiling($sorted.Count * 0.99) - 1]
            worst = $sorted[-1]
        }
    }
    $owned.StandardInput.Close()
    if (-not $owned.WaitForExit(5000)) { throw 'Read-only session did not stop after stdin EOF' }
    if ($owned.ExitCode -ne 0) { throw "Fixture exited with status $($owned.ExitCode)" }
    $null = $stdout.GetAwaiter().GetResult()
    $null = $stderr.GetAwaiter().GetResult()
    [ordered]@{
        measured_utc = [DateTime]::UtcNow.ToString('o')
        executable_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
        fixture = 'empty read-only local session; no GUI, bound windows, providers or hotkeys'
        samples = $Samples
        interval_seconds = 1
        warmup_seconds = 5
        cpu_percent_one_logical_core = (Distribution $cpu)
        working_set_bytes = (Distribution $working)
        private_bytes = (Distribution $private)
        graceful_eof_shutdown = $true
        rendering_readiness = 'not applicable; no GUI'
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Output -Encoding utf8
} finally {
    if ($null -ne $owned) {
        # Kill only the exact Process object created above, never a name/PID search result.
        if (-not $owned.HasExited -and $owned.StartTime.ToUniversalTime() -eq $identity) {
            $owned.Kill()
            $null = $owned.WaitForExit(5000)
        }
        $owned.Dispose()
    }
    $resolved = [IO.Path]::GetFullPath($fixture)
    if (-not $resolved.StartsWith($fixtureRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing cleanup outside fixture root'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
