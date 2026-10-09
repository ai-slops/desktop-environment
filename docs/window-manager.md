# Multi-view Window Manager

The implementation lives in three crates:

- `libs/window-manager-core`: platform-neutral configuration, identity, queries, formulas, layout evaluation, scope/claim validation, Visit exceptions, and immutable transition plans.
- `platforms/windows-window-manager`: ordinary-user Win32 enumeration, physical geometry observation, lifetime tokens, native event notification, final placement submission, hotkeys, and visibility recovery.
- `apps/window-manager`: Korean-accessible egui controls, background command execution, local CLI, persistence wiring, and an independently launched recovery helper.

The supplied [specification](window-management-system-specification.md) is the product baseline. This release implements the central P0 behavior and a manual control surface; it does **not** claim that the complete P0 compatibility gate or P1 AT-01–AT-56/VG-01–VG-10 gate has passed.

## Run

```powershell
cargo run -p window-manager
# Or: just window-manager / mise run window-manager
```

After building, run `target\debug\window-manager.exe`. No companion tool is required: the same executable starts its hidden recovery helper. Configuration defaults to `%LOCALAPPDATA%\DesktopEnvironment\window-manager.json`. `--config <absolute-or-relative-path>` selects an isolated store.

1. Open **화면 / 영역**, choose an actual monitor, and create a named region. Bounds are fractions of that monitor's work area. Two regions on a portrait monitor are separate stable slots, not additional physical monitors.
2. In **창 목록**, select **배치에 추가** for existing applications. Management is opt-in. Choose the target Group in **배치 / 그룹** before adding to a nested Group.
3. Choose horizontal/vertical split, grid, free placement, semantic tabs, or responsive tabs. Save structure explicitly. A tab can contain a whole nested Group. Removing references or unwrapping a Group never closes applications.
4. Select the View and destination slot. **전환 미리보기** reports movement-only, resizing, visibility, focus, unchanged counts, missing bindings, and conflicts; **계획 적용** performs the final changes.
5. Select **전환 시 유지** in the inventory and choose size preservation, keep-here, or temporary bring. These create Visit-local exceptions; ordinary saved geometry remains independent. **저장된 배치로 복원** is an explicit force-restore action.
6. For independent remembered sizes, preview the target View and select **현재 크기만 저장** or **현재 위치만 저장**. A saved property affects only the addressed Placement/slot/display/variant context. Formula rules remain separate from local overrides. Free placement can use saved positions; initial grid/split placement needs no geometry authoring.

Opening an already active target is idempotent. Switching one slot leaves unrelated claims and presentations alone. The planner blocks conflicting claims, slot overlaps, maintained-visible descendants hidden behind tabs, protected geometry changes, stale previews, and preserved windows that cannot fit. A hard fit failure leaves the prior layout instead of silently shrinking.

The UI supports explicit focus requests; ordinary arrangement and hotkey recalls preserve focus. Denied activation is reported separately from settled geometry. No input simulation is used. Fixed `Win+Ctrl+1` through `Win+Ctrl+9` commands store View and slot IDs. Restart the application after changing shortcut bindings; registration errors are displayed. Commands arriving during application queue the latest target for overlapping scopes while retaining disjoint pending work.

## Recovery and state

Hiding is disabled by default. Enable it for a Window reference only after checking that application's hide/restore behavior. Unsupported hiding leaves the window reachable and reports the limitation. Owned dialogs block hiding their owner.

Before any hide, a flushed, atomically replaced recovery journal records the prior visible state and validated HWND/process-creation/session/window-property lifetime. The helper observes the manager process's **actual exit signal**, not merely successful PID lookup. After exit it reveals eligible hidden windows without moving them, requesting focus, or restoring user-minimized windows. A failed reveal remains journaled for another attempt.

The GUI offers pause and reveal. Recovery also works independently:

```powershell
target\debug\window-manager.exe --recover
target\debug\window-manager.exe --recover --config .\my-layouts.json
```

Native submission is asynchronous and distinct from settlement. The worker checks final geometry/visibility for up to 900ms, including exact observed client size for strict preservation. Divergence or rejected operations pause enforcement, release affected claims, and attempt visibility recovery. Applications are never restarted or terminated. There is no automatic loop fighting a rejected application rectangle.

Authored state contains no HWNDs. Startup never auto-applies layouts or matches a reference solely by a title. Existing references missing a live binding are placeholders; select one and explicitly connect an inventory candidate. Geometry observed from native events is runtime evidence and never automatically saved as a manual edit. The current UI requires explicit property saving.

Configuration edits are revisioned and written through flushed temporary-file replacement. The prior valid document is saved to `.json.bak`. A corrupt/unsupported document starts the UI in read-only safe mode and is not overwritten. Restore an existing validated backup after closing the GUI:

```powershell
target\debug\window-manager.exe --restore-backup --config .\my-layouts.json
```

GUI instances and configuration-writing CLI commands share an exclusive same-store lock. Structural configuration edits have a bounded 50-action undo history. Native geometry undo and arbitrary user-drag autosave remain work for P1.

## Formulas and packages

Formulas use explicit local logical dimensions at 96 DPI. Inputs are `available_width`, `available_height`, and immediate child `count`. Arithmetic, comparisons, Boolean logic, lazy `condition ? a : b`, and `min`, `max`, `clamp`, `floor`, `ceil`, `abs`, `count()` are supported. Power uses `^`.

Examples:

```text
gap     = available_width < 720 ? 4 : 8
columns = max(1, floor(count ^ 0.5))
width   = min(available_width * 0.30, 600)
```

Source length is bounded to 4096 bytes, tokens to 512, and parse/evaluation nesting to 32. There is no filesystem/network/process/credential/window/input access. Unknown symbols, invalid syntax, non-finite results, divide-by-zero, negative dimensions, and invalid grid column counts fail explicitly. Syntax/function/name validation checks all branches before saving a draft. Value/fit errors are validated using the actual layout snapshot before application.

The GUI edits Group gaps/column rules. Placement width/height formulas, context geometry, variant ratios, height breakpoints, multi-root Views, and Collections can also be authored in the strict JSON model. Their native effects still go through the same planner.

Use `--check` to inspect stable IDs. Export redacts live identities, titles, application hints, tags, and display geometry, replacing resources with role placeholders:

```powershell
target\debug\window-manager.exe --check --config .\my-layouts.json
target\debug\window-manager.exe --export VIEW_ID .\layout-package.json --config .\my-layouts.json
target\debug\window-manager.exe --import .\layout-package.json --workspace WORKSPACE_ID --map preview=WINDOW_ID --map editor=EDITOR_WINDOW_ID --config .\my-layouts.json
```

Close the GUI before importing. Every required role must be mapped explicitly. Duplicate IDs, missing mappings, unknown fields/schema/language versions, resource limits, and invalid structure/formulas reject installation. Import creates independent new node/View IDs and never applies a live layout. Local display mapping remains an explicit separate selection. A copied arrangement still shares the actual application's content.

## Verification and support boundaries

Automated coverage includes independent copied preferences/property saving; transient preservation/restore/leave-return; idempotent recall; hard fit/scope failures; claim conflicts including post-preview claims; semantic alternatives referencing one window; responsive fold/unfold and retained ratios; keep-here reservations; disjoint scope generations; pure bounded formulas; three-valued unknown query results and explicit exclusion precedence; strict control fields; package mappings; durable configuration/backup; and generated 0/1/4/8/16-child layouts over multiple viewport widths with unique final mutations and no saved-state drift.

Real Win32 tests use disposable native STATIC windows to verify exact client-size preservation under move-only submission, destroyed-lifetime rejection, journaled reveal without geometry changes, and independently recovering a still-live hidden window after a separate manager-parent fixture is killed. The ignored `helper_parent_fixture` is launched by the integration test; it is not an untested product behavior.

Test host: Windows build `10.0.26200.0`, ordinary user privileges. GUI smoke verification renders Korean text and enumerates windows without applying anything. These checks establish fixture behavior, **not** browser/editor/game/broadcaster compatibility or redraw readiness. Per-window render readiness and actual capture/output state remain `unknown`/unverified. Logs contain opaque IDs, scope/revisions/counts/results, not titles or screenshots. The latest redacted result is retained beside the journal; there is no unbounded history or capture cache.

Remaining release gates / P1 work:

- Real editor/browser/terminal/game/broadcaster matrix; elevated windows, Korean IME/modal interaction, mixed DPI, hotkey collision, hung-app and rapid supersession measurements, and load/performance distributions.
- Capability-selected show-state changes, explicit off-screen rescue, robust topology fallback/reconnect restoration, and monitor identity confirmation when Windows cannot identify a monitor uniquely. Current behavior blocks unavailable/ambiguous mappings and never overwrites saved topology preferences.
- Full scoped native undo, manual-drag provenance/autosave, independent protection lifetimes, focus-prioritized responsive folding, more flexible preserve-size fitting/wrapping, all three Group-preservation modes, and independent partial-component application. Current executor uses bounded asynchronous per-window final submission; batched `DeferWindowPos` is not implemented.
- Dynamic Collection/selector materialization and staged membership changes, richer inline property/variant/package editors, Workspace remembered-target recall, all exact-scope expansion/drag/copy/reveal commands, and public CLI transition/session/event interfaces. Collection query and independent include/exclude semantics are present in the core model; they do not currently automate Group membership.
- Private-designated control-window placement/fallback and provider-based attention/output protection contracts. Current public designation is informational; there is no verified capture protection, broadcast integration, reliable AI task-state inference, or automatic application control.

P2 linked templates, executable extensions, app-internal providers, remote APIs, and observation mirrors are not implemented. These limitations are explicit; placeholders or fixture tests do not satisfy their release gates.

## Developer verification

The new crates pass strict Clippy. At this implementation checkpoint, full-workspace strict Clippy remains blocked by pre-existing warnings in `display-relay-core` geometry conversions and `windows-audio-router` Boolean branching. Those files were not changed. Full-workspace type checking and automated tests pass; the unchanged ignored tests are interactive fixtures and the intentionally child-launched recovery fixture.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-targets --all-features
```

Native and atomic-replace tests require a normal desktop user token; an AppContainer sandbox may deny file replacement even inside its own temporary directory. Tests manipulate only disposable fixture windows/processes, not the user's existing applications.

Native API contracts were checked against Microsoft's [SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos), [SetPropW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setpropw), and [SetWinEventHook](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook) documentation. API availability is not evidence of application-specific compatibility.
