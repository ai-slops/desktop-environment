# Architecture Rules

## 1. Crate boundaries

Use the following split consistently:

- `apps/*`: application entrypoints, GUI bootstrapping, CLI parsing, config file wiring.
- `libs/*`: domain logic that should compile on every supported OS.
- `platforms/*`: OS integrations, native APIs, event taps, display APIs, compositor hooks, accessibility APIs, FFI.
- `tools/*`: developer-only crates such as `xtask`, code generators, fixture builders.

Keep platform crates small and explicit. If a crate name includes an OS, it should expose a narrow API that the rest of the workspace depends on through traits or simple facades.

## 2. Dependency direction

Preferred direction:

```text
apps -> libs -> platforms
tools -> libs
```

Avoid:

- `platforms/*` depending on `apps/*`
- GUI code inside `libs/*`
- one app crate importing another app crate

If multiple apps need the same behavior, extract it into `libs/*`.

## 3. Platform isolation

Use `cfg` only at boundaries when possible:

- put Windows Desktop Duplication API code in `platforms/windows-*`
- put macOS-specific overlay or event tap code in `platforms/macos-*`
- put X11/Wayland-specific code behind Linux-facing platform crates

Avoid scattering `#[cfg(target_os = "...")]` throughout application logic. One concentrated adapter layer is easier to reason about and test.

## 4. Unsafe code policy

Unsafe is allowed only when required for system APIs or FFI.

Rules:

- isolate unsafe in dedicated modules
- document invariants immediately above the unsafe block or function
- prefer safe wrapper types over exposing raw handles across crate boundaries
- add at least one test, integration probe, or debug assertion around each unsafe abstraction

## 5. GUI guidance

Keep the GUI shell thin:

- window creation
- renderer/framework boot
- event loop hookup
- presentation state wiring

Move business logic, pointer math, display topology, animation rules, and capture policy into `libs/*`.

## 6. Naming

Use descriptive crate names:

- `desktop-core`
- `pointer-overlay`
- `windows-display`
- `linux-overlay`

Avoid vague names like `common`, `utils`, or `platform`.

## 7. Testing strategy

- unit tests in `libs/*`
- OS-specific integration tests in each platform crate
- snapshot or golden tests for geometry/state transforms where useful
- manual smoke-test commands implemented in `tools/xtask`

For native integrations that are hard to CI reliably, keep core logic testable in pure Rust and minimize the native surface area.

## 8. CI expectations

Every crate should eventually pass:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo check --workspace --all-targets --all-features`
- `cargo test --workspace --all-targets --all-features`

## 9. Feature flags

Use features for optional integrations, not for fundamental crate identity.

Good examples:

- `tray-icon`
- `serde`
- `mock-native`

Avoid feature matrices that switch an app between entirely different platforms. Prefer separate platform crates instead.
