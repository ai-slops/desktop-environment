set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    just --list

# Run Audio Repeater with interactive source/target selection.
audio-repeater:
    cargo run -p audio-output-router

# List available audio output devices.
audio-devices:
    cargo run -p audio-output-router -- list-audio-devices
