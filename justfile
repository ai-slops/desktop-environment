set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    just --list

# Build/package for development without installing or starting an application.
build:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/install-desktop-workspace.ps1 -BuildOnly

check:
    cargo check --workspace --all-targets

fmt:
    cargo fmt --all

# Install/update the application. Running UI windows and jobs remain active.
desktop-workspace-install:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/install-desktop-workspace.ps1
