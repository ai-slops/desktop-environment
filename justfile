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
    cargo build --target-dir target/desktop-control -p desktop-control -p audio-output-router -p display-relay
    cargo run --target-dir target/desktop-control -p desktop-control

# Run Audio Repeater with interactive source/target selection.
audio-repeater:
    cargo run -p audio-output-router

# List available audio output devices.
audio-devices:
    cargo run -p audio-output-router -- list-audio-devices
