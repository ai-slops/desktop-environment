# Desktop Environment Rust Workspace

Rust workspace scaffold for desktop-environment tooling that targets:

- Windows
- macOS
- Linux, primarily Ubuntu Desktop

This repository is intentionally set up without concrete crates yet. The goal is to provide conventions, shared linting, and a folder layout that scales well when adding multiple GUI apps, platform adapters, window-management helpers, overlays, and capture/output utilities.

Concrete utilities now included:

- `display-relay`: mirror one Windows display into a local control window
- `audio-output-router`: clone the audio of one Windows output device into another output device

## Environment setup

This repo is configured for [`mise`](https://mise.jdx.dev/) so the Rust toolchain can be installed and used consistently across Windows, macOS, and Linux.

Typical flow:

```powershell
mise install
mise tasks ls
mise run verify
```

If you want to use the local Windows binary directly:

```powershell
C:\Users\mjy90\workspace\lib\bin\mise.exe install
C:\Users\mjy90\workspace\lib\bin\mise.exe tasks ls
```

## Design goals

- Keep cross-platform code separate from platform-specific bindings.
- Make GUI apps thin and move behavior into reusable library crates.
- Allow selective use of `unsafe` in small, audited modules.
- Support shipping multiple binaries from one workspace without turning the root into a monolith.

## Recommended layout

```text
.
|- apps/              # End-user binaries and GUI apps
|- libs/              # Cross-platform reusable logic
|- platforms/         # OS-specific adapters and FFI wrappers
|- tools/             # Dev-only helper crates, e.g. xtask
|- docs/              # ADRs, architecture notes, API sketches
|- .cargo/
|- Cargo.toml
```

See [`docs/architecture.md`](/C:/Users/mjy90/workspace/codex/ai-slops/desktop-environment/docs/architecture.md) and [`docs/crate-template.md`](/C:/Users/mjy90/workspace/codex/ai-slops/desktop-environment/docs/crate-template.md) for the actual working rules.

## Workspace rules

- New crates should opt in to workspace metadata with `*.workspace = true` where possible.
- Shared third-party dependencies belong in `[workspace.dependencies]` when at least two crates use them.
- GUI crates live in `apps/`.
- Pure domain logic belongs in `libs/`.
- OS-specific code belongs in `platforms/<os>-*`.
- Dev automation belongs in `tools/xtask`.

## Suggested first crates

- `apps/display-control`
- `apps/pointer-overlay`
- `libs/desktop-core`
- `libs/input-geometry`
- `platforms/windows-capture`
- `platforms/windows-display`
- `platforms/macos-overlay`
- `platforms/linux-overlay`
- `tools/xtask`

## Common commands

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-targets --all-features
```

The same workflow is exposed through `mise` tasks in [`mise.toml`](/C:/Users/mjy90/workspace/codex/ai-slops/desktop-environment/mise.toml).

## Included helper crate

The workspace includes a minimal [`tools/xtask`](/C:/Users/mjy90/workspace/codex/ai-slops/desktop-environment/tools/xtask/Cargo.toml) crate so the scaffold is immediately valid for `cargo check`, `mise run check`, and future automation tasks.
