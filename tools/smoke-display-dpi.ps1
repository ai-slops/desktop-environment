param([string] $BinaryDirectory = (Join-Path $PSScriptRoot '../target/desktop-control/debug'))
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class DpiSmoke {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct MonitorInfo {
        public uint Size; public Rect Monitor, Work; public uint Flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string Name;
    }
    public struct State { public int Width, Height; public uint Dpi; }
    private delegate bool MonitorCallback(IntPtr monitor, IntPtr dc, ref Rect rect, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumDisplayMonitors(IntPtr dc, IntPtr clip, MonitorCallback callback, IntPtr data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] private static extern IntPtr GetWindowLongPtr(IntPtr window, int index);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool AdjustWindowRectExForDpi(ref Rect rect, uint style, bool menu, uint extendedStyle, uint dpi);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder title, int length);
    public static string Title(IntPtr window) { var title = new StringBuilder(300); GetWindowText(window, title, title.Capacity); return title.ToString(); }
    public static MonitorInfo[] Monitors() {
        var monitors = new List<MonitorInfo>();
        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (IntPtr monitor, IntPtr dc, ref Rect rect, IntPtr data) => {
            var info = new MonitorInfo { Size = (uint)Marshal.SizeOf<MonitorInfo>() };
            if (!GetMonitorInfo(monitor, ref info)) throw new System.ComponentModel.Win32Exception();
            monitors.Add(info); return true;
        }, IntPtr.Zero);
        return monitors.ToArray();
    }
    public static State Read(IntPtr window) {
        if (!GetClientRect(window, out var rect)) throw new System.ComponentModel.Win32Exception();
        return new State { Width = rect.Right - rect.Left, Height = rect.Bottom - rect.Top, Dpi = GetDpiForWindow(window) };
    }
    public static void SizeClient(IntPtr window, int width, int height) {
        var rect = new Rect { Right = width, Bottom = height };
        if (!AdjustWindowRectExForDpi(ref rect, unchecked((uint)GetWindowLongPtr(window, -16).ToInt64()), false,
                unchecked((uint)GetWindowLongPtr(window, -20).ToInt64()), GetDpiForWindow(window))) throw new System.ComponentModel.Win32Exception();
        if (!SetWindowPos(window, IntPtr.Zero, 0, 0, rect.Right - rect.Left, rect.Bottom - rect.Top, 0x16)) throw new System.ComponentModel.Win32Exception();
    }
    public static void Move(IntPtr window, int x, int y) {
        if (!SetWindowPos(window, IntPtr.Zero, x, y, 0, 0, 0x15)) throw new System.ComponentModel.Win32Exception();
    }
}
'@
$previousDpi = [DpiSmoke]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$binaryDirectory = (Resolve-Path -LiteralPath $BinaryDirectory).Path
$binary = Join-Path $binaryDirectory 'display-relay.exe'
$smokeRoot = (New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot ('../target/dpi-smoke-' + [Guid]::NewGuid().ToString('N')))).FullName
$statePath = Join-Path $smokeRoot 'relay-window.json'
$displayName = @(& $binary list)[-1].Split("`t")[0]
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
function Start-Mirror {
    $id = [Guid]::NewGuid().ToString('N')
    $process = Start-Process -FilePath $binary -ArgumentList @('mirror', $displayName, '--fps', '15', '--window-state-file', ('"' + $statePath + '"')) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $smokeRoot "$id.out") -RedirectStandardError (Join-Path $smokeRoot "$id.err")
    $owned.Add($process)
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw "Mirror exited early: $($process.ExitCode)" }
        if ([DpiSmoke]::Title($process.MainWindowHandle).StartsWith('Relay ')) { Start-Sleep -Milliseconds 300; return $process }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Owned mirror did not create a window'
}
function Assert-Size($process, [string] $label) {
    Start-Sleep -Milliseconds 700
    $process.Refresh()
    if ($process.HasExited) { throw 'Mirror exited during DPI check' }
    $state = [DpiSmoke]::Read($process.MainWindowHandle)
    Write-Host "$label : DPI=$($state.Dpi), client=$($state.Width)x$($state.Height)"
    if ($state.Width -ne 960 -or $state.Height -ne 540) { throw 'DPI changed the physical mirror size' }
    return $state.Dpi
}
try {
    $monitors = @([DpiSmoke]::Monitors() | Where-Object { $_.Work.Right - $_.Work.Left -ge 1100 -and $_.Work.Bottom - $_.Work.Top -ge 700 })
    if ($monitors.Count -eq 0) { throw 'No monitor has room for this smoke check' }
    $process = Start-Mirror
    [DpiSmoke]::SizeClient($process.MainWindowHandle, 960, 540)
    $dpis = [Collections.Generic.HashSet[uint32]]::new()
    foreach ($monitor in ($monitors + $monitors)) {
        [DpiSmoke]::Move($process.MainWindowHandle, $monitor.Work.Left + 50, $monitor.Work.Top + 50)
        $dpi = Assert-Size $process $monitor.Name
        [void] $dpis.Add([uint32]$dpi)
    }
    $saved = (Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json).windows.PSObject.Properties[$displayName].Value
    if (($saved.normal_client_size -join ',') -ne '960,540') { throw 'Physical client size was not saved' }
    if (-not $process.CloseMainWindow() -or -not $process.WaitForExit(5000)) { throw 'Owned mirror did not close' }
    $process = Start-Mirror
    [void] (Assert-Size $process 'Reopened')
    if ($dpis.Count -lt 2) { Write-Output 'Only one DPI is currently configured; cross-DPI movement remains unverified.' }
    Write-Output "Passed movement and reopen checks. Observed DPI values: $($dpis -join ', '). Logs: $smokeRoot"
} finally {
    foreach ($process in $owned) {
        $process.Refresh()
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
    }
    [void] [DpiSmoke]::SetThreadDpiAwarenessContext($previousDpi)
}
