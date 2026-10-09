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

1. On first launch, explicitly designate a monitor for the private control surface. Once it fits, open **화면 / 영역** and create named regions. Bounds are fractions of that monitor's work area. Two regions on a portrait monitor are separate stable slots, not additional physical monitors.
2. In **창 목록**, select **배치에 추가** for existing applications. Management is opt-in. Choose the target Group in **배치 / 그룹** before adding to a nested Group.
3. Choose horizontal/vertical split, grid, free placement, semantic tabs, or responsive tabs. Save structure explicitly. A tab can contain a whole nested Group. Removing references or unwrapping a Group never closes applications.
4. Select the View and destination slot. **전환 미리보기** reports movement-only, resizing, visibility, focus, unchanged counts, missing bindings, and conflicts; **계획 적용** performs the final changes.
5. Select **전환 시 유지** in the inventory and choose size preservation, keep-here, or temporary bring. These create Visit-local exceptions; ordinary saved geometry remains independent. **저장된 배치로 복원** is an explicit force-restore action.
6. For independent remembered sizes, open **활성 배치의 현재 속성 저장** after application and select **현재 크기만 저장** or **현재 위치만 저장**. A saved property affects only the addressed Placement/slot/display/variant context. Formula rules remain separate from local overrides. Free placement can use saved positions; initial grid/split placement needs no geometry authoring.

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

During settlement the worker continues draining hotkeys and urgent pause/recover/shutdown requests. A committed newer intent supersedes only overlapping scopes; independent slots continue. Superseded effects never commit ownership or become a completed request, and any manager-hidden windows in those scopes remain eligible for reveal. A multi-slot transition records its successful subset as one native undo unit. Explicit rebinding invalidates old ownership and generations; recalling a target after a missing/new native lifetime re-evaluates it without carrying the old instance's preservation exception.

Planning also partitions a request into conservative connected domains: shared resource references, prior scoped claims, overlapping regions, and cross-root retention stay together. A constraint failure in a separate domain leaves that domain unchanged and appears in the preview's `blocked` map; only explicitly previewed successful scopes can apply. Native settlement and supersession respect the same domains, preventing a successful source release from being committed independently of a failed resource transfer. Invalid scope, revisions, schema or tab choices reject the request rather than being dropped during partitioning.

Authored state contains no HWNDs. Startup never auto-applies layouts or matches a reference solely by a title. Existing references missing a live binding are placeholders; select one and explicitly connect an inventory candidate. Geometry observed from native events is runtime evidence and never automatically saved as a manual edit. Paired native user move/size-start and end events save only changed properties in the active Placement context, preserving formulas and unrelated editor drafts. Location changes alone never establish user-edit provenance. Saving promotes only the addressed Visit exception property.

Configuration edits are revisioned and written through flushed temporary-file replacement. The prior valid document is saved to `.json.bak`. A corrupt/unsupported document starts the UI in read-only safe mode and is not overwritten. Restore an existing validated backup after closing the GUI:

```powershell
target\debug\window-manager.exe --restore-backup --config .\my-layouts.json
```

GUI instances and configuration-writing CLI commands share an exclusive same-store lock. Structural configuration edits have a bounded 50-action undo history. **창 배치 되돌리기 미리보기** reverses a settled native transition as one scoped operation, revalidating identities, topology, generations, newer ownership, manual changes, and current protections. Focus is not restored. Failed slots are suspended independently; successful slots keep their presentations. **실패 영역 관리 재개** explicitly clears suspension. Graceful exit waits briefly for worker recovery, with the independent helper as the crash fallback. Manual move/size gesture saving operates only for bound active Placements on a compatible display/DPI; unpaired events, carried leaves, and lifetime changes do not persist geometry.

## Formulas and packages

Formulas use explicit local logical dimensions at 96 DPI. Inputs are `available_width`, `available_height`, and immediate child `count`. Arithmetic, comparisons, Boolean logic, lazy `condition ? a : b`, and `min`, `max`, `clamp`, `floor`, `ceil`, `abs`, `count()` are supported. Power uses `^`.

Examples:

```text
gap     = available_width < 720 ? 4 : 8
columns = max(1, floor(count ^ 0.5))
width   = min(available_width * 0.30, 600)
```

Source length is bounded to 4096 bytes, tokens to 512, and parse/evaluation nesting to 32. There is no filesystem/network/process/credential/window/input access. Unknown symbols, invalid syntax, non-finite results, divide-by-zero, negative dimensions, and invalid grid column counts fail explicitly. Syntax/function/name validation checks all branches before saving a draft. Value/fit errors are validated using the actual layout snapshot before application.

The GUI edits Group gaps/columns/ratios, width/height breakpoints and hysteresis, Placement client sizes/positions/minima/formulas, multi-root Views, and basic Collection queries with explicit inclusion/exclusion. Selector membership is calculated into a draft and explicitly saved. Removed generated members retain IDs and preferences for later return; export removes local selector sources and caches. Flow wraps observed sizes without shrinking. Responsive folding favors the currently focused descendant unless a tab was explicitly chosen; semantic tabs remain explicit alternatives.

The structural tools select a subtree by its View/ancestor path and move or independently copy it into an explicitly selected Group. Move keeps IDs, preferences and child weights; copy creates fresh IDs while intentionally sharing Window references. Cycle/self-insertion, missing destinations, invalid revisions or an invalid resulting tree reject the entire draft. Size copy addresses a source Placement/context and selected destination Placements/context, sets only local size overrides, preserves formulas, and creates no live link. Structural saving remains separate from native preview/application. The GUI and local session maintain separate bounded authored/native undo histories.

Group-local selector include/exclude overrides leave the source Collection and global tags untouched; local exclusion wins and the inspector exposes how to clear it. Removed generated children retain stable identity, preferences and split weights. Explicitly moving/removing an occurrence excludes it in its old selector Group, so reconciliation cannot silently recreate it there. Export also regenerates node IDs and distinguishes different resources sharing a role name (`member`, `member-2`), while repeated occurrences of one resource reuse one required mapping.

Temporary filters live in the Request/Presentation rather than the authored View. Editing a filter has no native effect until preview/application; clearing it restores the prior tab/variant selection in the same Visit subject to availability and protections. Three-valued query evaluation includes only proven matches, and query width/depth/total-node/text budgets apply to both selectors and temporary filters. Filtered trees keep IDs and child weight association; maintained-visible resources cannot be silently filtered out. The active-property inspector remains available after settlement and saves only explicitly selected size/position on the matching display/DPI/normal state, preserving formulas.

Use `--check` to inspect stable IDs. Export redacts live identities, titles, application hints, tags, and display geometry, replacing resources with role placeholders:

```powershell
target\debug\window-manager.exe --check --config .\my-layouts.json
target\debug\window-manager.exe --export VIEW_ID .\layout-package.json --config .\my-layouts.json
target\debug\window-manager.exe --import .\layout-package.json --workspace WORKSPACE_ID --map preview=WINDOW_ID --map editor=EDITOR_WINDOW_ID --config .\my-layouts.json
```

Close the GUI before importing. Every required role must be mapped explicitly. Duplicate IDs, missing mappings, unknown fields/schema/language versions, resource limits, and invalid structure/formulas reject installation. Import creates independent new node/View IDs and never applies a live layout. Local display mapping remains an explicit separate selection. A copied arrangement still shares the actual application's content.

## Local command sessions

`window-manager --session --config <file>` accepts one JSON object per line on inherited stdin and returns one response per line on stdout. Close the GUI first: the session exclusively owns the same configuration/runtime. There is no listening network endpoint. The default capability permits `snapshot`, `configuration`, `inventory`, and `events` only. `--allow-control` explicitly grants bind/preview/window_action/apply/undo/pause/recover; `--allow-providers` separately grants provider_register/provider_publish/provider_disconnect. Layout data cannot grant either capability. Control sessions start the independent recovery helper and recover on EOF/exit; read-only sessions do not register shortcuts or perform recovery.

```json
{"id":"read-1","command":{"kind":"snapshot"}}
{"id":"windows-1","command":{"kind":"inventory"}}
```

The envelope and command reject unknown fields. External JSON boundaries reject duplicate object keys at every depth (including escaped spellings), before map/tagged-enum decoding can discard a value. Configuration/package/journal reads are bounded during reading, not just by a prior file-size check. Input is limited to 64 KiB per line, IDs to 200 bytes, live previews to 32, inventory candidates to 256, and event history to 256 / 2 MiB. Inventory explicitly returns titles/class/process for human binding choice plus single-use opaque candidate tokens; normal snapshots/events never expose native handles, lifetime properties or window titles. Each new inventory invalidates prior candidate tokens. Bind requires `expected_revision`, `window`, and `candidate`. Preview accepts the strict domain `request`; window_action accepts the strict domain `action`. Both return an opaque `plan` token with scope, impact, desired geometry, revisions and diagnostics. Apply accepts only that token and performs common lifetime/topology/scope/protection revalidation. Undo returns a preview, which must also be explicitly applied. No command implicitly saves observed geometry.

Control/provider commands retain their canonical arguments and original response in a bounded session replay cache (256 commands / 8 MiB). Retries within that window return the original envelope without repeating effects; reusing an ID with different arguments rejects the command. An oversized response retains a receipt suppressing reexecution and requiring a fresh snapshot. Read-only commands remain fresh reads. The cache does not promise durable exactly-once behavior across session restarts or eviction.

Snapshot returns a cursor `{epoch,sequence}`. Events accepts that cursor. Missed intervals, future cursors and prior-process epochs return `STALE_REVISION`; obtain a new snapshot and resume from its cursor. Domain events summarize configuration, bindings, membership, attention, recovery, topology, transition and provider changes rather than forwarding Win32 callbacks. Provider evidence requires separately granted resources, matching provider session, monotonic sequence and fresh timestamp/TTL; disconnect and expiry invalidate evidence. An inherited pipe client granted control is trusted to issue local actions; no remote authentication or production provider integration is advertised.

Control sessions also accept `structure` with an `edit` containing `expected_revision` and a strict `move`/`copy`/`copy_size` action. Node addresses contain `view`/`node`; destinations contain `view`/`group`/`index` (null appends). Copy-size includes `source_context` and a list of `[address,context]` pairs. `configuration_undo` requires `expected_revision`; successful undo advances the revision. Both atomically save validated authored state, invalidate prior preview tokens and produce no native layout operation.

`monitor_inventory` is read-only and returns a current topology `epoch` with physical device names, optional hardware identities, work areas and DPI. `confirm_monitor` requires control capability and that epoch, an exact `device`, and a chosen saved-display `alias`. Duplicate/absent hardware IDs remain unresolved until explicitly confirmed; other uniquely identified monitors remain available. Aliases are runtime-only and expire whenever the observed monitor inventory changes, including disconnect, work-area or DPI changes. Saved display IDs and geometry contexts are never rewritten. Confirmation changes the topology revision and invalidates prior previews. Visibility recovery remains independent of durable display identification.

First launch exposes monitor designation controls before window titles or layout contents. Once configured, every rendered frame checks that the own control window still fits the private-designated region; dragging it outside redacts contents and offers an explicit return. Unresolved overlapping monitor areas conservatively block controls when a public designation exists. The worker polls topology at one-second intervals in addition to native events without issuing unchanged-state domain events. These rules concern designated geometry; they do not establish actual capture/output privacy.

## Verification and support boundaries

Automated coverage includes independent copied preferences/property saving; transient preservation/restore/leave-return; idempotent recall; hard fit/scope failures; claim conflicts including post-preview claims; semantic alternatives referencing one window; responsive fold/unfold and retained ratios; keep-here reservations; disjoint scope generations; pure bounded formulas; three-valued unknown query results and explicit exclusion precedence; strict control fields; package mappings; durable configuration/backup; and generated 0/1/4/8/16-child layouts over multiple viewport widths with unique final mutations and no saved-state drift.

Real Win32 tests use disposable native STATIC windows to verify exact client-size preservation under move-only submission, destroyed-lifetime rejection, journaled reveal without geometry changes, and independently recovering a still-live hidden window after a separate manager-parent fixture is killed. The ignored `helper_parent_fixture` is launched by the integration test; it is not an untested product behavior.

Test host: Windows build `10.0.26200.0`, ordinary user privileges. GUI smoke verification renders Korean text and enumerates windows without applying anything. These checks establish fixture behavior, **not** browser/editor/game/broadcaster compatibility or redraw readiness. Per-window render readiness and actual capture/output state remain `unknown`/unverified. Logs contain opaque IDs, scope/revisions/counts/results, not titles or screenshots. The latest redacted result is retained beside the journal; there is no unbounded history or capture cache.

Native settlement verifies the target DPI, requested client dimensions and minimum client dimensions in addition to the final physical frame. DPI transfer estimates non-client padding only for submission; a mismatched observed client/DPI fails within the deadline. Before committing each connected domain, the worker rechecks live lifetime, modal/style capabilities, topology/generations, protected overlap and current output evidence, including TTL expiry during application. Explicit show-state actions commit their actually observed runtime geometry without saving it to the authored layout.

Expansion is Visit-local: an explicitly selected leaf/subtree can fill its parent Group, source Slot, one monitor, or a contiguous rectangle of explicitly named Slots. Parent expansion preserves owned siblings outside the parent Group with zero mutations. Monitor expansion requires every intersected configured Slot to be listed; private content cannot enter public-designated regions. Borrowed Slots form one atomic domain, and protection/hide capability failures block that domain. Expanded preferences use an independent context. Collapse previews the original source layout and remembered borrowed targets; previously empty borrowed Slots are explicitly released. It never rewrites the authored tree. The local session exposes `collapse { slot }` as a control-capability preview; normal `preview` requests accept optional `expansion` and `release` fields.

Groups declare up to 32 numeric parameters in a validated acyclic dependency graph (16 KiB aggregate source, 128 inherited names). Reserved context names cannot be shadowed. Parameters use the Group's local available rectangle and immediate candidate count; children inherit resolved values. Variant conditions are typed Boolean formulas, and bounded numeric sort priorities reorder the evaluated copy while preserving stable IDs and associated ratios. Width/height formulas, gap/column formulas, conditions and declarations are validated before replacing a saved configuration. The editor displays ancestor path, candidate IDs, logical-unit basis, allocated physical rectangle, DPI, and resolved Group inputs.

The editor's synthetic preview exercises portrait/landscape dimensions, configurable candidate counts, missing bindings, large minima, fixed children, nested groups and absent displays. The same pure planner evaluates a cloned configuration with synthetic observations. Its report contains no native mutations/bindings and cannot be applied. Read-only sessions also accept `simulate { input }`.

The Layout Library prepares privacy-safe exports, validates bounded package files, shows schema/language/dependency/strategy declarations, edits exposed parameters, requires explicit Window-role mapping (or explicit unbound placeholders), and installs independent copies without moving windows. Package writes are atomic. Uniform contextual property formulas become portable Placement default rules; personal position/client-size overrides and monitor contexts are omitted. Package parameters and strategies must match content; executable/unavailable dependencies are rejected.

The spatial editor provides Window/Group/tab-subtree drag sources with full ancestor paths and explicit Group drop targets. Drop prepares a validated structure preview; accepting it changes only the draft, and saving/applying are separate operations. A stale preview cannot replace newer typed edits. Independent copy is a separate drag mode. Selected immediate children can be wrapped into a new Group, unwrapped with stable child IDs, or removed as arrangement references without removing Window resources or closing applications. Nested AND/OR/NOT and alias queries are available through ordinary controls.

Fixed shortcuts support View mappings, full Compositions, and a Workspace's remembered View in the original fixed role/Slot scope. They resolve stable IDs at invocation; missing remembered roles reject rather than using pointer position or widening scope. Version-one fixed-View shortcut documents remain readable. Local `recall { target }` returns the same control-capability preview.

`Save as View` explicitly saves the proven temporary-filter matches as an independent arrangement with fresh node IDs and shared Window resources. Selector caches are detached in that copy, while the source query/View stays unchanged. Session `save_filtered_view` requires a current revision, source View, name and query. Clearing/applying temporary filters alone never saves a View.

The property tools distinguish saving local observed overrides, explicitly replacing both size formulas with observed numeric values, and copying the authored size rule into a portable default preset. Position and other display contexts remain independent. New contexts inherit the portable rule instead of silently replacing it with an empty preference. Shared public-content warnings remain on reused/copied Window references through public Composition/shortcut designations, current/runtime public usage, explicit per-resource designation and live output-linked evidence. They do not claim content isolation or capture verification.

Groups expose existing-position-first (default), entry/request reflow, and explicit continuous-rule policies. Adding authored membership to an active default Group preserves its owned frames as visible Visit exceptions; explicit Restore previews full reflow. Continuous rules operate only on active private scopes with unchanged bound membership, preserve other Groups' arrangements, have a minimum 250 ms stable-input debounce, and submit at most one bounded transaction per scheduler pass. They defer when a managed window is foreground, a gesture/modal/show-state interaction is detected, or the presentation is public, expanded, retained or protected. Fresh output evidence is required where configured. Pending reasons are inspectable. Read-only/provider-only sessions never run native continuous rules. Foreground deferral is conservative; ordinary APIs do not establish full application-internal typing/IME detection.

Four alternating variant choices within two seconds freeze the affected Group's last valid owned arrangement and report a diagnostic; incompatible bounds still reject safely. Explicit Restore or a new configuration revision resets that bounded history. Provider updates/disconnects arriving during application are processed before settlement authority is committed.

Overlapping inferred memberships resolve explicit interactive assignment first, then higher Group rule priority, then stable Group/Placement ID. Candidate suppression occurs only on evaluated copies with associated ratios retained; source selectors and saved trees remain unchanged. The preview identifies the winner and reason. Unconditionally interactive candidates are arbitrated before fitting; potentially folded/semantic branches participate only when evaluated. Re-evaluation is limited to four passes and unstable branch/arbitration interactions reject while keeping the previous owned arrangement. Two explicit interactive assignments, repeated presentation of the same occurrence, and an existing valid owner outside the requested scope remain conflicts.

Reference planning/native measurement definitions, distributions and reproduction commands are published in [window-manager-performance.md](window-manager-performance.md). GUI smoke verification also covers scoped formula inputs, synthetic preview, subtree editing and Library role mapping at 192 DPI with synthetic references. Miniature labels are clipped/elided within their own tiles and full aliases remain available on hover.

A fit conflict offers an explicit size-exception preview for the selected Group’s Placement IDs within the original request scope. Applying it waives child/Visit size preservation and bounds oversized preferred dimensions to their allocated rectangles for that Visit. It does not rewrite saved formulas/preferences or weaken geometry locks, minimum size, capabilities, monitor/output constraints, or Group outer/arrangement preservation. Exceptions appear in presentations/snapshots and diagnostics, defer continuous rules, expire on leaving/changing bindings, and clear on explicit Restore. The strict request field is `approved_resize` (Placement IDs); the preview still requires explicit application.

Remaining release gates / P1 work:

- Real editor/browser/terminal/game/broadcaster matrix; elevated windows, Korean IME/modal interaction, mixed DPI, hotkey collision, hung-app and rapid supersession measurements, and application-load/performance distributions.
- Physical reconnect/docking validation for the monitor identity confirmation workflow. Explicit current-topology confirmations handle absent/duplicate identities without automatic guessing. Ordered display fallbacks preserve the original slot/display identity, use independent fallback preference contexts, reject private-to-public intersections, and offer an explicit original-context restore after reconnect. Unrelated absent/unresolved monitors do not block available scopes. Explicit show-state and off-screen rescue share preview/revalidation/settlement/undo. Show-state changes require a per-reference compatibility opt-in; maximize can activate and is blocked by focus protection. Rescue preserves physical size on compatible DPI and blocks hard fit/monitor/protection conflicts. Unavailable/ambiguous mappings never overwrite saved preferences.
- Optional overflow surfaces remain unsupported; ordinary native windows are never advertised as clipped. Group preservation distinguishes outer geometry, descendants' client sizes and the internal arrangement; sizing supports fixed children plus flexible remainder and empty-space alignment. Ordered Flow/responsive-tab fallbacks are opt-in. A failed owned subtree fitting its allocation keeps prior frames while independent siblings progress. Window/Placement/Group protections persist; Presentation protections last for the Visit; active Composition protection is derived from targets. Same-thread/same-parent native windows use `DeferWindowPos` with the returned handle retained and no End/replay after failure. Foreign application threads use asynchronous individual final submission to avoid a blocking cross-thread batch.
- Remaining application compatibility and interaction review. Pointer drag/drop structure previews, exact-scope expansion/collapse, addressed subtree move/copy, multi-Placement size copy, local membership overrides, Workspace remembered View selection, multi-root editing, Collection selectors, staged membership, inline property/variant editing, explicit reveal and bounded local CLI transition/session/event interfaces are available.
- Actual broadcast/AI provider adapters and compatibility verification. Provider contracts enforce granted resources, sessions, sequence numbers, timestamps/TTL and disconnect invalidation. Output-dependent changes reject unknown/stale evidence, including evidence expiring after preview. Attention routing opens only the explicitly designated slot/tab path and never automatically activates or approves. No production provider adapter is advertised. Private-designated control placement requires a fitting region with no public-designated intersection; contents stay redacted until the own control window fits. Monitor loss offers explicit designation changes without changing original display mappings. Designation cannot prove capture safety.

P2 linked templates, executable extensions, app-internal providers, remote APIs, and observation mirrors are not implemented. These limitations are explicit; placeholders or fixture tests do not satisfy their release gates.

## Developer verification

The new crates pass strict Clippy. The earlier geometry/audio warnings were corrected with bounded coordinate conversion coverage. Full-workspace strict Clippy exposes further pre-existing warnings in input/desktop-duplication native adapters and other applications (raw pointer syntax, ABI conversions, const suggestions and duplicated transitive dependencies). New window-manager crates pass strict Clippy. Full-workspace type checking and automated tests pass; the unchanged ignored tests are interactive fixtures and the intentionally child-launched recovery fixture.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-targets --all-features
```

Native and atomic-replace tests require a normal desktop user token; an AppContainer sandbox may deny file replacement even inside its own temporary directory. Tests manipulate only disposable fixture windows/processes, not the user's existing applications.

Native API contracts were checked against Microsoft's [SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos), [SetPropW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setpropw), and [SetWinEventHook](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook) documentation. API availability is not evidence of application-specific compatibility.

Native batch and explicit-state contracts additionally follow Microsoft's [DeferWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-deferwindowpos) and [ShowWindowAsync](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindowasync) documentation. Tests separately pump the disposable window's message queue and observe state settlement.
