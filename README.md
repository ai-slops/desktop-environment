# Desktop Environment Rust Workspace

Rust workspace scaffold for desktop-environment tooling that targets:

- Windows
- macOS
- Linux, primarily Ubuntu Desktop

The workspace provides conventions, shared linting, and a folder layout for GUI apps, platform adapters, window-management helpers, overlays, and capture/output utilities.

Concrete utilities now included:

- `window-manager`: independent saved arrangements of existing Windows application windows, scoped transitions, nested groups, preservation previews, and independent visibility recovery
- `desktop-control`: a shared Windows GUI for saving audio/display presets and managing multiple mirror windows
- `display-relay`: mirror one Windows display into a local control window
- `audio-output-router`: clone the audio of one Windows output device into another output device

## Saved-settings GUI

Run `just window-manager` or `cargo run -p window-manager` for window management.
Create a monitor region under **화면 / 영역**, add existing windows under **창 목록**,
and commit **전환 미리보기 → 계획 적용**. Startup and editing never auto-apply layouts.
This is a Windows feasibility implementation with a usable manual interface; the
complete P1 acceptance/compatibility gate is still open. See the
[window-manager guide and coverage](docs/window-manager.md) and the
[provided specification](docs/window-management-system-specification.md). The
[acceptance/support matrix](docs/window-manager-conformance.md) and
[reference measurements](docs/window-manager-performance.md) identify tested fixtures
and remaining application/hardware gates.

Run `just desktop-control` (or `mise run desktop-control`) to build both tools and open
the control panel. Choose audio outputs and a display, enable the tools you need,
and save a named preset. You can load it later and start both tools with one click.
The last saved settings are restored when the GUI opens; routing and mirroring
start only when requested. **미러링 추가** adds another window configuration; each row
has **창 열기** and **다시 열기** controls, so closing one mirror does not interrupt the
others. **저장하고 시작** starts enabled tools that are stopped. Closing the GUI stops
the tools it launched.

After building, double-click `target\desktop-control\debug\desktop-control.exe`. Keep
`audio-output-router.exe` and `display-relay.exe` beside it when copying the app.
Settings are stored in `%LOCALAPPDATA%\DesktopEnvironment\presets.json`.
See [the GUI guide](docs/desktop-control.md) for details.

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
