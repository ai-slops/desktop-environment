# New Crate Template

Use this when adding a new crate.

## Library crate

```toml
[package]
name = "desktop-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
anyhow.workspace = true
thiserror.workspace = true
tracing.workspace = true
```

## Binary crate

```toml
[package]
name = "pointer-overlay"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
desktop-core = { path = "../../libs/desktop-core" }
```

## Suggested source layout

```text
src/
|- lib.rs or main.rs
|- error.rs
|- model.rs
|- service.rs
|- platform.rs      # only if the crate is itself the adapter boundary
```

## Checklist

- Put reusable logic in `libs/*` before adding it directly to an app.
- Keep `main.rs` focused on wiring.
- If native APIs require `unsafe`, wrap them behind safe types.
- Add a crate-level doc comment describing responsibilities.
- Prefer `tracing` over ad-hoc `println!`.
- Add tests next to pure logic as early as possible.
