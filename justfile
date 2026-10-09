set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    just --list

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
