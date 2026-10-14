set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    just --list

# Manage existing windows with independent saved views and scoped transitions.
window-manager:
    cargo run -p window-manager

# Reveal only windows recorded as hidden by the manager.
window-recovery:
    cargo run -p window-manager -- --recover

# Build both tools and open the shared GUI with saved presets.
desktop-control:
    just desktop-workspace

# Open only the cable and quiet-monitoring settings.
audio-cable:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-desktop-workspace.ps1 -Panel cable

# Install immutable application versions outside build output; open windows stay running.
desktop-workspace-install:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/install-desktop-workspace.ps1

# Open the installed native UI; its daemon continues after the window closes.
desktop-workspace:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-desktop-workspace.ps1

desktop-workspace-daemon:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/start-desktop-workspace.ps1 -NoBrowser

desktop-workspace-stop:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/stop-desktop-workspace.ps1

# Legacy egui front ends for advanced features.
desktop-control-native:
    cargo run -p desktop-control

audio-cable-native:
    cargo run -p audio-cable

# Run Audio Repeater with interactive source/target selection.
audio-repeater:
    cargo run -p audio-output-router

# List available audio output devices.
audio-devices:
    cargo run -p audio-output-router -- list-audio-devices
