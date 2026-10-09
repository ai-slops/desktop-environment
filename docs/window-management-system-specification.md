# Multi-View Window Management System
## Product Requirements and Implementation Specification

**Version:** 0.1 — implementation handoff  
**Date:** 2026-10-09  
**Primary platform:** Windows desktop; platform-neutral domain model  
**Document status:** Proposed baseline consolidated from the design discussion  
**Format:** Markdown  
**Product name:** Not yet selected

> Manage existing application windows as reusable, independently arranged parts of a working environment. Switch the surrounding task context without unnecessarily resizing, hiding, moving, or interrupting the windows that should remain continuous.

## Reading guide

This specification defines the product, not an implementation language or GUI framework. Its normative requirements are identified by stable IDs. **MUST**, **SHOULD**, and **MAY** express required behavior, recommended defaults, and optional behavior within this document; they do not imply conformance to an external standard.

The proposed delivery stages are:

| Stage | Meaning |
|---|---|
| **P0 — Feasibility gate** | Test real-window transitions, event handling, visibility, recovery, and application compatibility before building the complete interface. |
| **P1 — Core personal release** | Deliver the coherent Windows-first product: independent layouts, recursive groups, partial transitions, tags, basic formulas, multi-monitor composition, and recovery. |
| **P2 — Advanced workflows** | Add linked templates, richer providers, output-system integration, and optional observation surfaces. |

A feature marked P2 is deferred, not silently replaced with a weaker P1 guarantee. Safety and state-ownership rules apply as soon as the related feature exists. Proposed defaults and performance targets are design decisions, not measured results. Platform observations are separately sourced in Section 23.

## Contents

1. Purpose and product boundaries
2. Goals, non-goals, and operating assumptions
3. Representative workflows
4. Conceptual model and terminology
5. State ownership and persistence
6. Window identity, membership, and discovery
7. Recursive groups, tabs, and responsive layout
8. Multi-monitor composition and independent switching
9. Transition semantics and size preservation
10. Manual changes, automatic changes, and precedence
11. Formula and layout-package requirements
12. Interaction and accessibility requirements
13. Concurrent work, attention, and output protection
14. Window lifecycle, compatibility, and recovery
15. Architecture boundaries and platform execution contract
16. Performance and observability
17. Extension, automation, and security boundaries
18. Logical data and command contracts
19. Acceptance scenarios and requirement traceability
20. Delivery plan and release gates
21. Decision register and unresolved validation items
22. Implementation handoff checklist
23. Technical references and evidence boundaries

---

## 1. Purpose and product boundaries

### 1.1 Problem statement

The intended user works with existing applications across multiple tasks and monitors. Some applications are remote-development frontends, AI interfaces, terminals, browsers, games, or broadcast tools, but the product must not assume that every activity is software development.

Conventional single-desktop membership is insufficient: the same browser, editor, or monitoring window may be useful in several task contexts, with a different position and size in each. Conversely, restoring every saved rectangle on every transition is sometimes undesirable. A user may want to keep a preview at its current size, keep a game exactly where it is, or replace only one monitor region while monitoring continues elsewhere.

The product therefore combines independent saved arrangements with explicit control over what a transition is allowed to change.

### 1.2 Product proposition

A user can create named working arrangements from real application windows, compose several arrangements on physical displays, and switch a chosen part of that composition immediately through stable commands. Windows can participate in multiple arrangements without sharing their layout state. Groups can contain windows and subgroups, switch whole sublayouts through tabs, and respond to their allocated space.

Manual organization is useful without tags or formulas. Automation is added progressively and must remain explainable, bounded, reversible, and subordinate to explicit user constraints.

### 1.3 Boundary relative to the original IDE idea

**CORE-01 — Existing applications first.** The initial product MUST manage existing desktop application windows. It is not a replacement editor, terminal, remote filesystem, AI runtime, or browser.

A remote file explorer, code editor, AI client, and preview browser may be separate applications. Their internal functionality remains their responsibility. Managing internal browser tabs or IDE panes requires an explicit provider and is not part of generic top-level-window support.

**CORE-02 — Separate arrangement from execution.** Opening, hiding, switching, detaching, removing, or deleting an arrangement MUST NOT implicitly launch, restart, terminate, mute, approve, or submit work inside an application. Optional launch and application-control commands are separate actions with separate permissions.

**CORE-03 — Native management boundary.** A later web/PWA control interface MAY edit or invoke the same domain commands, but the Windows window-management backend remains a separate native capability. The initial product does not require the original IDE/PWA shell.

### 1.4 Definition of a useful first release

The first release is useful when a user can reliably return to a named working arrangement, preserve selected windows during the transition, and keep unrelated work undisturbed. A visually impressive tiling animation is not a substitute for that behavior.

---

## 2. Goals, non-goals, and operating assumptions

### 2.1 Goals

| Goal | Required outcome |
|---|---|
| Stable task recall | A fixed shortcut always identifies the same configured target and switching scope. |
| Independent layouts | Reusing a window never implicitly links its geometry across Views or placements. |
| Continuous work | The user can preserve size, position, visibility, and input focus independently. |
| Partial automation | Manual regions coexist with tag-driven, formula-driven, and responsive groups. |
| Concurrent activities | Several Workspaces can be visible at once; there is no global exclusive active Workspace. |
| Multi-monitor flexibility | One monitor can contain several independent regions; one View can use several regions. |
| Predictability | Membership, placement, exceptions, conflicts, and deferred changes have explainable reasons. |
| Recoverability | A bad rule, stalled application, or manager crash does not permanently lose access to windows. |
| Extensibility | Additional layout strategies and application providers do not redefine core state semantics. |

### 2.2 Non-goals for the initial release

The initial release does not provide a new OS compositor, universal embedding of third-party windows, independent interactive copies of one real window, universal recovery of application-internal state, cross-machine window migration, a remote-desktop protocol, an AI execution scheduler, or a broadcast application.

It does not promise zero-latency transitions, frame-atomic redraw across unrelated applications, live rendering of every hidden window, reliable suppression of all third-party focus changes, or privacy protection against arbitrary capture software. Unsupported or unverified capabilities must remain visible as such.

### 2.3 Operating assumptions

The baseline is one interactive Windows user session, with applications managed under ordinary user permissions where possible. Exact supported Windows builds, application classes, and any optional elevated helper are chosen through P0 testing and published in the compatibility matrix.

Multiple monitors may have different orientations, resolutions, refresh rates, and scaling. Large portrait monitors divided into several regions are a primary test configuration, not a special-case emulation of additional physical monitors.

The system stores local configuration and layout metadata by default. Accounts, cloud sync, remote control, and third-party executable plugins are not prerequisites.

---

## 3. Representative workflows

| ID | Workflow | What changes | What remains continuous |
|---|---|---|---|
| UC-01 | Implementation to review | Editor region becomes diff/review; preview may retain its size. | Actual application sessions and unrelated regions. |
| UC-02 | AI work while gaming and broadcasting | AI region switches project, conversation, or review layout. | Game geometry, operating monitors, and automatic non-interference with focus. |
| UC-03 | Multiple AI projects | One task region switches from project A to B. | A's execution, shared operating information, and other visible Workspaces. |
| UC-04 | Shared reference browser | The same browser participates in several Views with independent placements. | One actual browser session; its content is not cloned. |
| UC-05 | Group-level modes | A tab replaces an entire implementation subgroup with a review subgroup. | Sibling groups and application execution. |
| UC-06 | Responsive side panel | Side-by-side subgroups become tabs when the parent region narrows. | Logical membership, preferred sizes, and focused content when feasible. |
| UC-07 | Large portrait displays | Several named regions occupy each physical monitor. | Stable region identities and selective switching. |
| UC-08 | Presentation plus private notes | The user changes private working material while presenting. | Explicit output target; no claim of capture safety without integration. |
| UC-09 | Temporary interruption | A utility or incident window is brought into the current context and then returned. | The saved layout and original placement preference. |
| UC-10 | Monitor removal/reconnection | Runtime layout adapts to remaining displays. | Original topology-specific preferences and protection constraints. |
| UC-11 | Imported responsive layout | Roles, tags, and display targets are mapped before applying. | Current live arrangement until explicit application. |
| UC-12 | Application or manager restart | Existing windows are rediscovered or explicitly rebound. | Saved layouts; no automatic duplication of uncertain application sessions. |

### 3.1 Reference concurrent-work composition

A Screen Composition named `Broadcast: game + AI` contains:

- Center display: `Game / Play`, with geometry preservation and no automatic focus requests.
- Left display: `Project A / AI work`, replaceable independently with `Project A / Review` or `Project B / Status`.
- Right upper region: `Broadcast / Operations`, continuously allocated and not eligible for automatic tab hiding.
- Right lower region: `Broadcast / Chat`, unchanged during AI-region switching.

Starting this composition arranges already available windows. It does not start a stream, start a game, submit an AI task, or alter the capture scene. Those are separate optional commands.

An AI approval request creates attention in the operating interface. It does not activate the AI window automatically. Selecting it opens the required tab path in the configured AI region. The game and operating regions are excluded from the resulting mutation set.

---

## 4. Conceptual model and terminology

### 4.1 User-facing concepts

| Concept | Responsibility | Explicitly not responsible for |
|---|---|---|
| **Workspace** | Organizes a task context, its Views, Collections, and defaults. | Exclusive ownership of windows or physical screens. |
| **Window** | A real application window, represented by a persistent reference when appropriate. | All layouts in which it appears. |
| **Tag** | Shared classification metadata on a Window reference, with provenance. | Geometry or placement-specific roles. |
| **Collection** | Reusable window selection, through a query and manual exceptions. | Visible layout, geometry, or selected tabs. |
| **View** | A named arrangement with one or more root Groups and saved layout preferences. | Exclusive control of all monitors or all work. |
| **Group** | A recursive container of Window Placements and child Groups. | Application lifetime or project ownership. |
| **Placement** | One occurrence of a Window in a particular arrangement, including its local role and layout preferences. | A second application instance. |
| **Display Slot** | A named region of a physical display, with topology mapping and bounds. | The task or View that occupies it. |
| **Screen Composition** | A saved mapping of View roots to Display Slots, with continuity constraints. | Application-internal or broadcast state. |
| **Layout Preset** | Reusable group structure, responsive rules, and parameters. | Implicitly shared live selection or focus state. |

A Group can use an inline selector without requiring a named Collection. A default Workspace and View are created automatically. Users need not understand every concept before grouping their first windows.

### 4.2 Internal concepts

| Concept | Purpose |
|---|---|
| **Live Window Binding** | A validated connection between a persistent Window reference and a current platform handle, including a lifetime/generation identity. |
| **View Presentation** | One live use of a View in a particular set of Display Slots. |
| **Layout Context** | The stable display-profile/root-binding context in which placement preferences are saved. |
| **Visit** | The period during which a presentation occupies a switching scope; bounds temporary transition exceptions. |
| **Transition Request** | The user's or automation's desired target, scope, preservation rules, and focus intent. |
| **Transition Plan** | A validated set of final platform mutations and diagnostics. |
| **Window Claim** | The exclusive assignment of a real interactive window to one active Placement. |
| **Recovery Journal** | Records manager-induced changes needed for bounded recovery or undo. |

These are internal state concepts, not additional mandatory top-level navigation objects.

### 4.3 Structural model

```text
Workspace
  Views
    View
      Root role: main
        Group
          Placement -> Window reference
          Group
            Placement -> Window reference
      Optional root role: auxiliary
        Group ...
  Collections

Screen Composition
  Display Slot: left-work
    View Presentation: Project A / Review
      main root -> left-work
  Display Slot: center-game
    View Presentation: Game / Play
      main root -> center-game
  Display Slot: right-operations
    View Presentation: Broadcast / Operations
      main root -> right-operations
```

**MODEL-01 — Structural identity.** Groups and Placements MUST have stable IDs independent of names, array indexes, and tree paths. Reordering, renaming, wrapping, and unwrapping must not accidentally reset their state.

**MODEL-02 — Tree structure, shared resources.** An instantiated group structure MUST be acyclic. A child node has one parent in a View. Reuse is achieved by copying structure or referencing a versioned preset, not by implicitly sharing one mutable live subtree between parents. Multiple Placements may reference the same Window.

**MODEL-03 — Composition boundary.** A View declares root roles, not hard-coded physical monitor ownership. A Screen Composition binds those roots to Display Slots. A one-root View can occupy one region; a multi-root View can use several regions without becoming the global screen composition.

**MODEL-04 — Simultaneous use.** Multiple Workspaces and View Presentations MAY be active simultaneously. A single real interactive Window MUST have no more than one active owning Placement. Other eligible Placements show a conflict/reference state rather than issuing competing movement requests.

---

## 5. State ownership and persistence

### 5.1 Required separation

| State | Owner / key | Persistence and change rules |
|---|---|---|
| App content, document, internal tabs, process | External application | Never inferred to be copied or restored by layout operations. |
| Window metadata and binding hints | Window reference ID | Persistent where selected; live handle is not durable identity. |
| Global/manual/automatic tags | Window reference ID plus provenance | May affect several Collections; edits are explicit. |
| Collection query and exceptions | Collection ID | Shared membership definition, independent of geometry. |
| View structure and local roles | View ID / node ID | Persistent, revisioned, undoable. |
| Preferred geometry and manual overrides | View ID / Placement ID / Layout Context / variant | Persistent within the current context unless explicitly promoted or copied. |
| Group split ratios and free positions | Group ID / Layout Context / variant | Variant-specific; compact state does not overwrite wide state. |
| Last selected semantic tab and focus | Presentation context / Group ID | Remembered as UI state; never treated as shared template data. |
| Current target layout | Transition generation / Window ID | Runtime only. |
| Observed platform state | Live Window Binding | Runtime truth, not automatically authored intent. |
| Preserve-on-transition exception | Visit ID / Placement or Group / property mask | Temporary; never automatically committed to a View. |
| Last-known-good layout | Presentation / topology context | Recovery checkpoint; not a replacement for authored preferences. |
| Output/capture state | Explicit provider, if installed | Includes evidence and freshness; unknown is a valid state. |

**STATE-01 — Independent occurrences.** Geometry belongs to a Placement and layout context, not to the Window reference. Two tabs in one View can use the same window with different saved sizes, just as two Views can.

**STATE-02 — Stable context, not every pixel size.** Layout Contexts MUST use stable display profiles and root-to-slot mappings. The implementation MUST NOT create a new persisted context for every resize pixel or visit. Two presentations of the same View in different slots have distinct local state by default; promoting changes to the shared View definition is explicit.

**STATE-03 — Authored versus observed.** OS observations MUST NOT overwrite saved geometry merely because a manager-issued transition, application correction, DPI change, or topology fallback occurred. User edits, adapter observations, transition effects, and recovery actions need distinguishable provenance.

**STATE-04 — Temporary overrides.** A preservation override survives local tab operations and permitted reflows within the destination Visit. It ends when that switching scope leaves the destination, when explicitly cleared, or when promoted to persistent state. It does not leak into a later ordinary visit. Repeating an idempotent command that already addresses the same presentation does not create a new Visit or silently clear its overrides.

**STATE-05 — Scoped saving.** Ordinary manual geometry edits save only the properties the user changed, in the current Placement/context/variant. A drag MUST NOT serialize the entire observed desktop into a View. While a property is protected by a temporary transition exception, editing it changes that exception; saving it requires `Save here` or equivalent. Other untouched properties remain independent.

**STATE-06 — Display changes.** Emergency or temporary topology adaptation MUST NOT overwrite the original display-profile layout. Restoring a profile recomputes against its saved preferences and current live bindings, rather than blindly replaying stale handles.

### 5.2 Sharing matrix

| Operation | Membership shared? | Structure/rules shared? | Geometry shared? | Live UI state shared? |
|---|---:|---:|---:|---:|
| Use the same Collection | Yes | No | No | No |
| Copy a Group | Only if it retains a Collection reference | Independent copy | Initial values copied | No |
| Copy current size to other Placements | No change | No change | One-time value copy | No |
| Use a shared size profile (P2) | No change | Size definition only | Selected dimensions share a definition | No |
| Use a linked Layout Preset (P2) | Parameterized | Versioned link | Defaults only; local overrides survive | No |
| Use the same real Window | Actual app content is shared | No | No | App-internal state is shared; manager tab state is not |

### 5.3 Non-negotiable invariants

**STATE-07 — Invariants.** The planner, executor, and persistence layer MUST enforce all of the following:

1. One real window receives at most one final geometric target in a plan.
2. No implicit app launch, termination, input injection, approval, or broadcast change occurs during layout switching.
3. An inactive View may recompute metadata but cannot move a window currently used elsewhere.
4. Preserving current geometry does not change the destination's saved geometry.
5. Explicit hard constraints are not silently relaxed to satisfy a formula or fill space.
6. An unaffected Display Slot retains its live group selection and receives no unnecessary window mutations.
7. View or Group removal does not close its applications.
8. Recoverable windows remain discoverable even when they are hidden, unplaced, conflicted, or unsupported.

---

## 6. Window identity, membership, and discovery

### 6.1 Identity and binding

**MEM-01 — Durable references and live generations.** A Window reference MUST be separate from a platform handle. A live binding records enough lifecycle information to reject a destroyed/reused handle. Application identity, process identity, window class, user-confirmed aliases, and provider-specific document/session IDs can be binding hints; none is universally assumed sufficient.

After restart, an exact provider identity may be rebound automatically. Ambiguous matches MUST require selection rather than silently attaching a similarly titled window. Missing references may keep a reserved Placement without launching an application.

**MEM-02 — Available metadata only.** Queries MUST distinguish known, unknown, unavailable, and stale attributes. Project identity, URL, document path, AI status, or capture state is usable only when a provider actually supplies it. Unknown values are not false, zero, or empty strings. In particular, `NOT private` must not turn an unknown classification into proof that a window is public-safe.

### 6.2 Tags, queries, and manual membership

**MEM-03 — Selection model.** Collections and inline selectors MUST support manual inclusion, manual exclusion, and queries. The P1 query editor supports nested AND/OR/NOT over tags, application identity, user aliases, and available basic window metadata. A textual expression is optional for ordinary use. Manual inclusion does not secretly modify tags.

Membership evaluation follows this order:

```text
Candidates = query matches UNION manual includes
Selected   = Candidates MINUS Collection exclusions
                          MINUS View exclusions
                          MINUS Group-local exclusions
```

Exclusion wins over inclusion at the same scope. An action intended to re-include a window must explicitly clear or override the relevant exclusion and show its scope. A manual Placement can bypass an ordinary selector, but not an active explicit exclusion or protection constraint without an explicit change.

**MEM-04 — Contextual roles.** A Placement's local role, such as `primary`, `reference`, or `support`, MUST be independent of global Window tags. Moving a browser to a reference group in one View must not relabel it as a reference everywhere.

**MEM-05 — Stable dynamic membership.** Automatic members MUST retain stable Placement identities while they remain or return to the same Group. Reordering or renaming a window must not recreate all child nodes. New arrivals append in a stable order unless the user selected another deterministic sort. Repeated evaluation of unchanged inputs is idempotent.

**MEM-06 — Membership overlap.** When several active Groups automatically select the same window, explicit Placement assignments take precedence over inferred assignments, then configured rule priority, then a stable tie-break. The winner and reason are inspectable. Semantic tabs may intentionally contain separate inactive Placements for the same window; this is not an error until simultaneous interactive claims are requested.

**MEM-07 — Explainability.** Every included, absent, or unplaced window MUST expose a concise reason: selector match, direct inclusion, exclusion, missing binding, inactive tab, held elsewhere, unsupported operation, or unsatisfied layout constraint. Automatic tags include their originating rule and support a per-window suppression exception.

### 6.3 Discovery and membership actions

**MEM-08 — Add versus move.** Adding a window or Group to another View MUST preserve its source by default. Moving, copying, borrowing, and adding are distinct commands. Moving between explicit Placements updates only the addressed structure, not inferred global tags.

**MEM-09 — Unplaced inventory.** Windows that do not fit, match no Group, or cannot be controlled MUST remain accessible in a searchable inventory. First-run management must be opt-in: the product must not hide every unassigned desktop window automatically.

**MEM-10 — Temporary query.** A temporary filter MUST NOT mutate the saved View or create a new persisted layout for every query. Clearing it restores the previous presentation state, subject to current window availability. `Save as View` is an explicit operation.

**MEM-11 — Deferred removal during interaction.** If an active window stops matching a rule, the membership model may update immediately, but manager-initiated hiding or disruptive movement MUST be deferred while the user is interacting with it. The reason and pending action remain visible. The system must offer keep-here and apply-now actions.

---

## 7. Recursive groups, tabs, and responsive layout

### 7.1 Structure and operations

**LAY-01 — Recursive children.** A Group MUST accept both Window Placements and child Groups. Users can wrap a selection into a Group, unwrap a Group without closing its children, split it, change its layout strategy, and move the whole subtree as a unit.

**LAY-02 — Basic strategies.** P1 MUST provide horizontal/vertical split, bounded grid or flow, free placement, and tabbed presentation. Strategies use the same child model and do not recreate Window references when changed. Scrolling, paged layouts, and additional strategies may be extensions; a native-window strategy must not promise arbitrary clipping it cannot implement.

**LAY-03 — Group tabs.** A tab may contain one Window Placement or an entire nested Group. Selecting a tab changes only that Group's visible subtree and requested focus. Application processes are not restarted. A Group can itself be a tab child of another Group.

**LAY-04 — Two tab semantics.** Explicit semantic tabs and responsive folding MUST be distinguished in stored state and UI:

| Type | Meaning | Behavior when space increases |
|---|---|---|
| Semantic tabs | Alternative local task arrangements, such as Implementation and Review. | Remain mutually exclusive unless explicitly changed. |
| Responsive folding | Previously concurrent children folded due to available space. | May return to split/grid if constraints permit. |

A layout cannot unfold semantic alternatives containing the same real window into competing active Placements.

### 7.2 Parent–child layout contract

**LAY-05 — Local available space.** Each Group computes its layout from its allocated content rectangle, not from the physical monitor size by default. Parent Groups allocate space to immediate children. Child Groups arrange their own children. A parent MUST NOT reach into descendants and overwrite local layout rules merely to fill its own area.

Groups may expose explicit minimum, preferred, and maximum size hints to the parent. Negotiation must be bounded and cycle-checked. Manager chrome, gaps, and insets are deducted consistently before the content rectangle reaches the child.

**LAY-06 — Stable outer geometry during tab selection.** Tab changes MUST retain the Group's allocated rectangle by default. Content-fit expansion is a separate user-enabled behavior and cannot overwrite protected neighboring slots.

**LAY-07 — Responsive variants.** Groups MAY define variants selected by their available width/height, aspect ratio, selected candidate count, or declared capability constraints. P1 MUST support width/height breakpoints, alternate strategies, and minimum-size fallback. Variant transitions use stable thresholds/debouncing so that small boundary changes do not repeatedly flip layouts.

Structural membership and names are shared across variants. Split ratios, free-placement coordinates, and variant-specific manual adjustments remain separate. Returning to a wide split after compact tabs restores the previous wide ratios.

**LAY-08 — Focus continuity during folding.** When concurrently visible children fold into responsive tabs, the tab containing the current user target SHOULD become selected. When this is impossible, the reason is reported; the system must not silently activate another application.

### 7.3 Resizing policy and fit behavior

**LAY-09 — Sizing policies.** Each Group MUST support the following independent layout preferences:

| Policy | Behavior |
|---|---|
| Fill available space | Resize eligible children to the allocated layout. |
| Preserve child sizes where feasible | Change positions, gaps, wrapping, or folding before resizing. |
| Fixed children plus flexible remainder | Preserve selected children and let others absorb available space. |

A hard child-size constraint is different from a soft preference. Empty space is a valid outcome. Alignment options include start, center, and end; empty space must not force enlargement.

**LAY-10 — Outer group versus descendants.** The UI and command model MUST distinguish preserving the outer Group rectangle, preserving descendants' client sizes, and preserving the complete internal arrangement. Keeping the Group's outer size alone does not assert that its children avoid resize.

**LAY-11 — Fit failure.** Before applying, the planner MUST detect known minimum-size conflicts, hard preservation conflicts, duplicate claims, and protected-region overlap. A tab fallback solves competition between children, not a single child's inability to fit inside the slot.

The Group defines an ordered list of allowed fallbacks: wrap/reorder, fold eligible children, use an explicitly allowed overflow surface, keep the existing placement, or allow an explicitly approved resize. If no valid fallback exists, retain a safe prior arrangement and expose the conflict. Do not silently shrink a hard-preserved window, cover another protected slot, or lose the window.

**LAY-12 — No hidden side effects.** Previewing a layout, resizing an editor mock region, or hovering a View candidate MUST NOT move live application windows. P1 defaults to outline/preview while editing and one final application on commit. Optional live-resize modes are compatibility-dependent.

### 7.4 Group operations and state

**LAY-13 — Spatial operations.** Window drag, Group drag, tab drag, and ancestor selection MUST show the exact manipulated scope. Group deletion removes arrangement references, not applications. Unwrap promotes children while preserving IDs and reasonable geometry; any unavoidable geometry change is previewable and undoable.

**LAY-14 — Bounded focus mode.** The following commands are distinct: focus a child inside its Group, expand to the current Display Slot, expand to a physical monitor, and expand across specified slots. Expansion beyond the current slot requires an explicit target scope and cannot silently occlude protected monitoring regions.

**LAY-15 — Local failures.** A failed formula or unsupported child MUST degrade only the dependent Group/subtree when independent siblings remain valid. Interdependent constraints are planned as one consistency domain rather than applied in an arbitrary half-state.

---

## 8. Multi-monitor composition and independent switching

**DSP-01 — Display Slots.** A Display Slot MUST have a stable ID, a user-readable name, a display-profile mapping, and a bounded region. A slot may cover an entire display or part of it. Its identifier must not depend solely on current monitor enumeration order. Display identity uncertainty requires mapping confirmation rather than silent assignment to an arbitrary screen.

**DSP-02 — Multiple presentations.** A Screen Composition MUST support simultaneous presentations from different Workspaces. A multi-root View binds its root roles to an explicit set of slots. Replacing one root independently is permitted only through a clearly scoped command; the remaining roots are not silently reinterpreted as a different global View.

**DSP-03 — Partial transitions.** A request to replace one slot or slot set MUST preserve unrelated presentations, selected tabs, stored manager UI state, and actual window geometry. Shared-window conflicts extend the plan's dependency set and require explicit resolution; they do not grant permission to mutate an unrelated source slot silently.

**DSP-04 — Stable shortcuts.** Commands such as `Win + Ctrl + 1` MUST bind to target IDs and explicit scopes, not list indexes. Supported targets include a complete Screen Composition, a View in a fixed slot mapping, or a Workspace's remembered View in a fixed scope. A separate opt-in command may target the current slot. Pointer location must not change the meaning of a fixed-target shortcut.

**DSP-05 — Idempotent recall.** Recalling an already active target MUST not perform unnecessary movement, hide/show, focus toggling, or persistence writes. `Restore saved layout` is a distinct force-reapply action; ordinary recall does not unexpectedly clear a preservation exception or manually maintained runtime state.

**DSP-06 — Slot overlap.** Normal active Display Slots MUST not overlap unless an explicit overlay/floating composition permits it. Temporary retained windows reserve occupied rectangles before other layout is computed. Overlapping protected reservations produce a conflict rather than a best-effort violation.

**DSP-07 — Topology adaptation.** Disconnect, reconnect, work-area change, rotation, and scaling changes MUST create a new validated runtime plan. Original layouts remain saved. When all constraints cannot be satisfied, the system reports which constraints conflict and offers explicit alternatives. It must not automatically move private operating content into a known public output region.

**DSP-08 — Return from fallback.** Reconnecting a previously used topology offers or performs the configured restoration policy without restarting applications. User changes made in the fallback context are retained in that context. Windows created while displays were missing remain discoverable and are not discarded during restoration.

---

## 9. Transition semantics and size preservation

This section is the central behavioral contract. Switching task context does not necessarily mean applying every rectangle stored by the destination View.

### 9.1 Required transition modes

**TRN-01 — Explicit mode and scope.** P1 MUST expose these operations through commands and discoverable UI:

| Mode | Selected retained target | Destination behavior | Saved-state effect |
|---|---|---|---|
| **Open saved arrangement** | None | Apply destination preferences, responsive rules, and persistent constraints. | None from the transition itself. |
| **Switch while keeping size** | Window(s) or Group descendants | Preserve specified client dimensions; movement remains permitted within the explicit scope. | Visit-local override only. |
| **Keep here; switch surroundings** | Window(s) or Group | Preserve position and size; reserve occupied space; replace only eligible surrounding content. | Visit-local override only. |
| **Bring here temporarily at current size** | Window or Group | Add a temporary destination occurrence, subject to claim and fit checks. | No membership or tag edits. |
| **Restore this View's saved arrangement** | Current presentation or selected subset | Explicitly clear selected temporary geometry exceptions and reapply saved preferences. | Does not erase saved manual overrides unless separately requested. |

Preservation targets are selected by stable identity. A command may capture the current selection at invocation, but its resolved target must be inspectable. Repeated shortcuts need not require a confirmation dialog.

### 9.2 Exact preservation semantics

**TRN-02 — Source snapshot.** The planner MUST snapshot the currently observed geometry and binding generation of retained windows when the request is accepted, before applying any destination operations. It must not read a partially transitioned intermediate size as the intended source.

**TRN-03 — Preserve-size behavior.** For an unchanged DPI/decoration context, strict size preservation means preserving the application's observed client width and height, with a separately recorded frame rectangle. A supported application receives no intentional client resize. At another DPI, frame style, or window show state, preservation may be impossible or ambiguous; use the explicit geometry-basis policy in Section 15, retain the source monitor, or report a conflict. Do not relabel a best-effort approximation as strict preservation.

Width-only preservation MAY be offered as a reading-continuity option. It is not equivalent to avoiding all resize operations.

**TRN-04 — Destination occurrence resolution.** If a retained window has exactly one eligible destination Placement, use it. If it has several semantic-tab occurrences, select the intended branch explicitly or through the destination's selected tab. Do not apply the override to all occurrences. If it has no destination Placement, create a Visit-local carry Placement in a designated retention/temporary region. This does not add a saved Placement, alter a Collection, or change tags.

**TRN-05 — Keep-here reservations.** A retained window outside the destination's allocated scope is not moved merely to make the target layout feasible. The user may widen the switching scope explicitly or keep the existing source presentation/anchor. A keep-here request must show which source area remains reserved. On Visit end, release its reservation and replan; do not blindly move the window back if another active scope now owns it.

**TRN-06 — Temporary exception lifecycle.** Preserved geometry takes effect throughout the destination Visit and any accepted local reflow. It never schedules an automatic delayed resize to the saved destination dimensions. `Match saved size`, `Clear this exception`, `Save here`, and `Copy size to other placements` are explicit operations.

### 9.3 Example with independent saved sizes

```text
Window: Preview
Implementation placement: saved client size 1200 x 800
Review placement:         saved client size 1800 x 1000

Open Review normally:
  Apply 1800 x 1000.

From Implementation, keep Preview size and open Review:
  Apply a Visit-local 1200 x 800 constraint in Review.
  Recompute only eligible surrounding content.
  Review's saved 1800 x 1000 remains unchanged.

Leave Review, then open Review normally:
  Apply its saved 1800 x 1000.

While the temporary size is active, select Save here:
  Update only Review's addressed size properties and clear that exception.
```

**TRN-07 — Copy versus shared size.** P1 MUST support one-time copying of selected dimensions to selected Placements without linking future edits. P2 MAY introduce a named shared size profile with explicit units, DPI basis, and subscribers. An ordinary local resize becomes a local exception, not an implicit edit to all subscribers.

### 9.4 Plan–apply–observe behavior

**TRN-08 — Preview and impact report.** A transition plan MUST be inspectable before application and report unchanged windows, movement-only windows, resized windows, show/hide changes, focus requests, conflicts, and unsatisfied preservation requests. Do not claim a precise latency prediction without a validated measurement model. Browsing candidates uses previews, not actual sequential transitions.

**TRN-09 — Latest intent.** Rapid requests replace queued superseded work within the affected scope. Unsent operations from an obsolete generation MUST be discarded. Commands already delivered to applications may complete later; their observations must never update the wrong View's saved preferences. Disjoint scopes may progress independently, subject to shared-window arbitration.

**TRN-10 — Minimal final mutations.** Compute final layout before touching application windows. Apply at most one intended final geometry per changed window per committed plan. Intermediate group calculations and responsive variants do not become intermediate OS resizes. An unchanged protected window receives no redundant move, resize, show/hide, Z-order, or activation request.

**TRN-11 — Completion states.** Expose `planned`, `applying`, `settled`, `partially_applied`, `blocked`, `superseded`, `cancelled`, and `failed` with per-window results. Geometry settlement is not proof of application redraw or input readiness. A rendering readiness field is `unknown` unless measured or supplied by a reliable provider.

**TRN-12 — Focus intent.** Every request explicitly chooses preserve-current-focus, focus-resolved-target, or no-focus-request. Geometry preservation does not imply focus transfer. Automatic layout and attention updates MUST not request activation. Explicit user commands may request it, but denial is reported rather than fought with repeated focus-stealing attempts.

**TRN-13 — Failure and dependency boundaries.** Preflight all known hard conflicts before mutation. Apply independent valid components only under the selected partial-apply policy, and identify exactly what remained unchanged. For mutually dependent components, block or compensate as one unit. Native window operations are not promised to be atomic or perfectly reversible.

---

## 10. Manual changes, automatic changes, and precedence

### 10.1 Scope of manual editing

**EDIT-01 — Property-level overrides.** Dragging a window creates a local position override; resizing creates a local size override; reordering creates an order override; assigning another Group changes only the local parent assignment. The implementation MUST not disable all automatic behavior merely because one property was edited.

**EDIT-02 — Formula-preserving manipulation.** Manipulating a formula-controlled property must either update an exposed parameter or create a local override. It MUST NOT silently replace the formula with a constant. `Replace formula with value`, `Reset override`, and `Promote to preset` are explicit operations.

**EDIT-03 — Autosave visibility.** Normal local arrangement edits are saved to the active layout context with undo support. Temporary transition properties remain Visit-local as defined above. The inspector must show whether a value comes from a preset, View rule, shared size profile, manual override, or current-Visit exception.

### 10.2 Automatic scheduling

**EDIT-04 — Reflow policies.** Groups MUST support existing-position-first, reflow-on-entry/request, and continuous-rule modes. Existing-position-first is the default. Membership changes and actual window movement are separate stages. Automatic changes during active typing, drag, modal interaction, or protected presentation are deferred where detectable; undetectable application state must not be claimed as protected with certainty.

**EDIT-05 — Inactive evaluation.** Background rule evaluation may update candidates and planned layout metadata, but cannot manipulate actual windows belonging to a currently active presentation elsewhere. Template updates are staged, not live global mutations.

### 10.3 Constraint resolution

**EDIT-06 — Hard constraints before preferences.** Resolution order is:

1. Validate authority, scope, live identities, and platform capability.
2. Combine all applicable hard constraints: physical availability, explicit geometry/visibility protection, active output restrictions, and strict preservation requests.
3. If hard constraints conflict, block or use an explicitly permitted fallback. No arbitrary priority winner is allowed to violate another hard constraint.
4. Apply Visit-local preferences and property overrides where not constrained.
5. Apply persisted local overrides and shared-size defaults.
6. Evaluate Group rules and Layout Preset defaults.
7. Use deterministic fallback layout.

Persistent protection cannot be overridden by a formula or a soft best-fit preference. An explicit user action can amend a protection rule within its declared scope; ordinary drag or switching does not silently amend it.

**EDIT-07 — Distinct pin meanings.** Always-include, geometry-lock, maintain-visible, show-in-all-contexts, keep-monitor, and always-on-top MUST be separate properties. A convenient profile may configure several, but their meanings and scope remain inspectable.

**EDIT-08 — Scoped undo.** A single user layout action, including an automatic multi-window reflow committed as one action, MUST create one undo unit. Undo reverses manager-authored structure and geometry changes where still feasible; it does not rewind documents, game state, chat, or process execution. Conflicts with newer work require reconciliation rather than destructive rollback.

---

## 11. Formula and layout-package requirements

### 11.1 Progressive authoring

**FORM-01 — Simple controls first.** The editor MUST permit fixed values, proportions, automatic sizing, and formulas at individual properties. P1 formulas cover dimensions, gaps, column counts, conditional strategy/variant selection, and sort priorities. Ordinary grouping, tagging, and switching do not require writing formulas.

A full custom layout algorithm can be introduced through a P2 provider. It must return a declarative layout result; it cannot bypass the planner and move OS windows directly.

**FORM-02 — Pure bounded evaluation.** Formula evaluation MUST be deterministic for an immutable input snapshot, side-effect-free, and bounded by operation, time, recursion, and output-size limits. The expression environment has no direct filesystem, network, process, credential, OS-window, or arbitrary host-language access. Importing a formula is not permission to run an application or submit input.

The exact grammar is an implementation selection. The P1 semantic minimum is arithmetic, comparisons, conditional expressions, Boolean logic, named parameters, and bounded functions such as min/max/clamp/floor/ceil/count. Unknown data is typed and propagates explicitly; divide-by-zero, invalid units, NaN, infinity, negative sizes, and unbounded output are validation errors.

### 11.2 Evaluation inputs and outputs

**FORM-03 — Explicit context.** The editor MUST show the current Group path, available content rectangle, coordinate/unit basis, selected candidate set, and parameter values. Default inputs refer to the Group and its immediate children, not an implicit global current Workspace or monitor.

| Input family | Examples |
|---|---|
| Available space | Content width/height, aspect ratio, usable area after chrome. |
| Child information | Stable identity, declared role, minimum/preferred size, manual constraints. |
| Selection | Candidates before geometric visibility, selected semantic branch. |
| State | Current variant, prior stable ordering, explicit user parameters. |
| Capabilities | Whether the adapter can move, resize, observe, or preserve the requested property. |

Outputs are strategy/variant selection, child rectangles, ordering, and proposed folding/overflow. Hard visibility, geometry, scope, and output restrictions are validated after evaluation and cannot be weakened by its result.

**FORM-04 — Acyclic dependencies.** Layout-output visibility MUST NOT feed back into its own candidate-selection predicate. Parent/child size dependencies require a directed, validated relationship or bounded negotiation. Cycles or repeated variant oscillation freeze the affected Group at its last valid arrangement and produce a diagnostic.

Illustrative expressions, not a mandated language:

```text
support_width = min(available_width * 0.30, 600 logical units)
columns       = max(1, floor(available_width / preferred_child_width))
strategy      = available_width < compact_threshold ? tabs : horizontal_split
```

### 11.3 Preview, diagnostics, and application

**FORM-05 — Simulation.** The editor MUST preview synthetic window counts, missing metadata, large minimum sizes, fixed children, portrait/landscape containers, nested Groups, and absent displays without manipulating live windows. Show which windows would move, resize, fold, remain unchanged, or fail placement.

**FORM-06 — Actionable errors.** Diagnostics MUST identify the Group/property and the conflicting values, for example: `Required minimum width 640; available content width 420`. Offer relevant actions such as resize the Group, choose folding, keep the prior placement, or permit an explicit size exception. Invalid draft expressions never replace the last committed valid rule.

**FORM-07 — Commit boundary.** Applying a draft is explicit. A layout-package update or changed expression does not modify public/retained regions automatically. The resulting plan passes the same validation and undo pipeline as a manual layout edit.

### 11.4 Import, export, and reuse

**FORM-08 — Package contract.** A layout package MUST include a schema version, stable package ID, human-readable name, required parameters/roles, supported strategies, expression-language version, and declared dependencies. Import validates resource limits, references, IDs, and cycles before installation. File import does not execute commands or automatically apply the layout.

**FORM-09 — Environment mapping.** Import MUST provide an explicit mapping step for window roles, Collections/tags, Display Slots, and unavailable capabilities. Unmapped inputs remain visible placeholders. A foreign monitor number or tag name must not silently bind to an unrelated local target.

**FORM-10 — Copies and links.** P1 imports independent copies. P2 linked presets use explicit versions, staged updates, diffs, conflict handling, and local override preservation. Applying one instance must not change selected tabs or focus in another instance.

**FORM-11 — Privacy of packages.** Exports omit live handles, credentials, screenshots, sensitive titles, URLs, and document paths by default. Include only explicitly selected binding hints. Shared packages use role placeholders rather than personal identifiers where possible.

---

## 12. Interaction and accessibility requirements

### 12.1 Primary surfaces

| Surface | Required purpose |
|---|---|
| Quick switcher | Search and recall fixed targets; show target scope and preservation mode. |
| Window inventory | Find visible, hidden, unplaced, missing, and claimed-elsewhere windows. |
| Composition overview | Map View roots to physical-display regions and inspect retained slots. |
| View/Group editor | Edit the tree, responsive variants, formulas, and local overrides. |
| Placement inspector | Explain membership, ownership, geometry source, and compatibility limitations. |
| Layout library | Import, copy, compare, parameterize, and later update presets. |
| Recovery surface | Pause automation, reveal managed windows, and resolve unsupported states. |

**UX-01 — Manual onboarding.** Users MUST be able to select currently open windows, create a Group/View from the current arrangement, name it, and bind a shortcut without configuring tags or formulas. First-run behavior must preserve unrelated applications.

**UX-02 — Progressive complexity.** Ordinary usage shows minimal group chrome. Ancestor breadcrumbs and tree editing appear when selected or in layout-edit mode. Deep nesting must not permanently stack large toolbars above every application.

**UX-03 — Exact actions.** Menus and drag previews distinguish add, move, copy, temporary borrow, remove from this View, collapse here, unwrap Group, and close the real window. A Group's close control must not resemble a command that terminates every contained application.

**UX-04 — Preservation discoverability.** A Window/Group menu MUST offer `Switch View while keeping size` and `Keep here; switch surroundings`. The switcher shows resolved retained targets. Frequently used combinations can be stored as commands; users are not required to memorize an initial matrix of modifier keys.

**UX-05 — State explanation in place.** Temporary preservation, local override, shared-size reference, automatic placement, blocked application, and claimed-elsewhere states need compact labels and inspectable details. `Reset local override` and `Use saved View size` remain distinct actions.

**UX-06 — Search navigation.** Search results deduplicate real windows and list their Placements. Actions include reveal at current location, navigate to a specific View/tab path, add here, and borrow temporarily. Revealing opens only required ancestor tabs and does not replace an unrelated Screen Composition.

**UX-07 — Non-disruptive preview.** Switcher hover, keyboard selection, layout previews, and imported-package inspection MUST not trigger live window mutation. Actual commitment is explicit.

**UX-08 — Accessibility and input.** All structural operations must have keyboard-accessible equivalents, visible focus, descriptive labels, and non-color-only state indicators. Global shortcuts must avoid intercepting ordinary text composition. Korean IME composition, modal dialogs, keyboard layouts, remapping conflicts, and high-DPI scaling are explicit test cases. Shortcut registration failure is reported instead of silently changing a binding.

**UX-09 — No required performance mode.** Preserving a specific window and changing only its surroundings must be ordinary workflow commands, not a global low-performance mode that removes View functionality. Motion is optional; reduced-motion behavior is supported.

**UX-10 — Always accessible recovery.** Pause automatic management, reveal manager-hidden windows, and bring recoverable off-screen windows into reach must be accessible without opening the full layout editor. A separately operable recovery mechanism is required for the manager's own crash or UI stall.

---

## 13. Concurrent work, attention, and output protection

### 13.1 Protection properties

**PRO-01 — Independent protection dimensions.** Define independent constraints for geometry, monitor placement, continued display allocation, manager-initiated focus changes, and output-linked changes. Each constraint has a scope and lifetime: Placement, Group, presentation, or Screen Composition. `Always visible in this composition` does not mean globally on top in every Workspace.

Maintaining visibility means the manager does not hide, fold, or intentionally cover the target through its own operations. It cannot guarantee that arbitrary external applications never cover it. Report detected violations; do not claim exclusive control of the desktop.

**PRO-02 — Ancestor enforcement.** A maintain-visible descendant constrains all ancestor layout choices. An ancestor cannot satisfy its own fit rule by hiding that descendant behind an unselected tab. An explicitly approved summary representation may satisfy a separately defined summary requirement; a badge is not automatically equivalent to showing the full monitoring window.

### 13.2 Attention and concurrent activity

**PRO-03 — Source-based attention.** Attention events require a known source, timestamp, Workspace/task context, and kind. Supported kinds include informational, completed-unreviewed, waiting-for-input, waiting-for-approval, and operational fault. Without an adapter, expose only genuinely observed OS/app signals; do not infer reliable AI state from arbitrary title changes.

**PRO-04 — No automatic activation.** Attention may aggregate through Group tabs, Views, and a central inventory. Receiving attention MUST NOT switch a semantic tab, replace a View, request foreground activation, or resize a protected game. User selection routes to the configured work slot and required tab path.

**PRO-05 — Separate acknowledgment.** Opening an event, reviewing a result, and approving an application action are separate operations. A layout command does not approve AI or application work. Attention previews obey the privacy policy of the surface where they appear.

### 13.3 Broadcast and presentation boundary

**PRO-06 — Designated versus verified public state.** The system MUST distinguish a user-designated public region from a provider-verified current output/capture state. P1 can enforce conservative local constraints on designated regions but must label actual output state as unverified without integration. Hiding or covering a local window is not evidence that capture has stopped; OBS documents window capture that remains visible despite foreground windows. [S12]

**PRO-07 — Shared content warning.** A real window reused in private and public placements still shares application content. When known or designated as public, editing it elsewhere must retain that warning. A copied View is not a private content clone.

**PRO-08 — Restricted automatic output changes.** Automatic membership additions, template updates, and replacement bindings in output-linked regions are staged as candidates rather than silently published. Explicit output changes use a separate command and, when available, an output provider. The window manager does not become a broadcaster merely by naming a slot `public`.

**PRO-09 — Management UI placement.** Switchers, thumbnails, titles, notifications, and error details may be sensitive. Provide a private control surface and an explicit fallback when it is unavailable. Do not quietly route sensitive management UI into a designated public slot after a monitor is disconnected.

**PRO-10 — Unknown output is not safe output.** If capture integration disconnects or its state becomes stale, stop making verified-safety claims and restrict dependent automatic changes. Any optional safe-output action requires actual provider support and acknowledged completion. Local blanking alone is not a sufficient fallback for every capture method.

### 13.4 Impossible concurrent constraints

**PRO-11 — Explicit infeasibility.** When a one-monitor fallback cannot simultaneously maintain a full-screen game, a private input area, continuously visible operations, and an unchanged public output, show an unsatisfied-constraints state. Offer explicit alternatives, such as reducing the game area, selecting another control device, or changing output through a supported provider. Do not silently weaken privacy or visibility constraints.

---

## 14. Window lifecycle, compatibility, and recovery

### 14.1 Discovery and ownership

**WIN-01 — Managed versus unmanaged.** The system MUST maintain a clear managed-window set. Unmanaged windows are not automatically hidden or arranged. Associated owned dialogs and popups are discovered as a window family when the platform exposes that relationship.

**WIN-02 — Application families and modals.** Owner/modal relationships influence visibility and focus handling. The manager must not intentionally leave a required modal dialog inaccessible while preserving its blocked owner as though usable. Secure-desktop and unsupported system UI remain outside normal management, with no bypass attempts.

**WIN-03 — External user actions.** Explicit user minimize, restore, maximize, move, and close actions are not indistinguishable from manager-generated changes. A user minimize temporarily suspends manager maintain-visible enforcement for that target and is shown as an exception; the manager must not immediately fight the user by restoring it. Restoring through the taskbar is honored locally and offers reconciliation rather than automatically switching all monitors.

**WIN-04 — Show state versus geometry.** Normal/restored rectangle, maximized state, minimized state, and supported full-screen state are tracked separately. Applying a normal rectangle must not implicitly unmaximize or exit a protected full-screen application. Such a change requires an explicit plan and supported capability.

### 14.2 Visibility and capability profiles

**WIN-05 — Pluggable visibility strategies.** Hide/show, minimize/restore, desktop integration, and any optional cloaking strategy MUST be isolated behind capability-tested adapters. The implementation must not assume one mechanism is correct for all applications. Undocumented APIs, if used experimentally, are version-gated, clearly declared, and not the only recovery path.

**WIN-06 — Process continuity is not render continuity.** The product MUST distinguish a running application from fresh visual content. Chromium's documented occlusion behavior includes stopping rendering and throttling JavaScript for occluded windows. Therefore hidden-window liveness and visual readiness require actual evidence, not a universal guarantee. [S11]

**WIN-07 — Unsupported operations.** Per-window results may be supported, unknown, temporarily blocked, rejected, timed out, or unavailable. A window that refuses resizing remains reachable and may use position-only/manual handling. Apply bounded retries and avoid an infinite correction loop when an application repeatedly restores its preferred geometry.

**WIN-08 — Respect privilege boundaries.** Ordinary user-level operation is the default. Elevated-window support, if offered, uses explicit capability boundaries and disclosure. Failure to access a window does not justify global elevation of formula evaluation or third-party plugins.

### 14.3 Restart, undo, and emergency restoration

**REC-01 — Recovery journal.** Before hiding or otherwise making a managed window difficult to reach, journal its validated live identity, relevant prior state, intended manager change, and ownership generation. Preserve enough information to distinguish manager-induced invisibility from a window the user had already minimized or hidden.

**REC-02 — Independent rescue.** A watchdog/helper or equivalent separately operable mechanism MUST recover manager-hidden supported windows if the main process crashes. Validate current handle lifetime and session before restoring. Never act on a stale handle merely because its integer value matches a journal entry.

**REC-03 — Non-destructive recovery.** Recovery reverses only applicable manager-induced changes. It must not restore every historical rectangle or undo unrelated user changes made after the journal entry. Keep a diagnostic record of skipped or failed recovery actions and provide accessible manual fallback.

**REC-04 — Graceful exit.** Exiting the manager MUST stop enforcement and release or restore manager-hidden windows according to an explicit exit policy. It must not terminate applications. The user may choose to retain visible geometry or restore a recorded pre-management arrangement, with validation.

**REC-05 — Persistence durability.** Authored configuration and committed local edits MUST use revisioned, crash-consistent storage. Imports, migrations, and multi-node structural edits are atomic at the configuration level. Maintain a known-good backup and a safe mode that loads without third-party extensions or automatic layout application.

**REC-06 — Reattachment.** On manager restart, rediscover existing windows before considering optional application launches. Exact binding matches may be resumed; ambiguous or missing matches appear as placeholders. An application restart does not imply that its prior HWND, document, or session still exists.

**REC-07 — Undo reconciliation.** Undo or restore performed after another scope has claimed the same window must revalidate claims and constraints. A restore command cannot silently steal a window from a newer protected presentation.

---

## 15. Architecture boundaries and platform execution contract

### 15.1 Required separation of responsibilities

The implementation may choose its language, toolkit, IPC, and storage engine, but MUST preserve these boundaries.

| Component | Input | Output / responsibility |
|---|---|---|
| Domain/configuration store | Explicit edits and versioned imports | Workspaces, Views, Groups, selectors, profiles, and stable IDs. |
| Window registry | Platform discovery and provider metadata | Validated live bindings, metadata provenance, capabilities. |
| Presentation coordinator | Active composition and scoped commands | Visits, root bindings, temporary overrides, exclusive Window claims. |
| Query/layout evaluator | Immutable state snapshot and available space | Candidate placements, variant selection, final geometry proposals. |
| Transition planner | Proposal, observed state, constraints, request | Validated mutation diff, conflicts, dependencies, revision checks. |
| Platform executor | Validated final operations | Bounded native actions and per-window submission results. |
| Observer/reconciler | Native events, provider results, pending commands | Observed state, settlement, divergence, bounded corrective actions. |
| Recovery controller | Journal and current live identities | Safe reveal/restore, watchdog support, emergency diagnostics. |
| UI / command interface | Human or authorized automation intent | Commands and explainable previews; no direct bypass of the planner. |

**ARCH-01 — Logical groups, native leaves.** The baseline MUST keep third-party application windows as native top-level windows and represent Groups logically. Final rectangles are flattened before execution. Cross-process reparenting is not a prerequisite; Microsoft documents DPI-related hazards for `SetParent`. [S05]

Manager chrome may be separate native windows, but its geometry, focus behavior, and overlap are included in planning. True clipping, composited virtual surfaces, and independent interactive mirrors are outside the P1 baseline.

### 15.2 Planning pipeline

**ARCH-02 — Immutable planning snapshot.** Each plan is built against explicit configuration revisions, binding generations, topology revision, observed geometry, protection state, and scope generations:

```text
Resolve command and target scope
  -> Capture retained-source geometry
  -> Resolve selected semantic branches and candidates
  -> Evaluate group strategies and preferred geometry
  -> Arbitrate real-window claims and retained reservations
  -> Resolve bounded layout/constraint dependencies
  -> Validate capabilities, fit, scope, and protection
  -> Diff against observed native state
  -> Produce final mutation set and diagnostics
  -> Revalidate preconditions immediately before execution
  -> Submit bounded operations
  -> Observe, reconcile, and mark the actual result
```

Layout/claim interactions may require bounded iteration. They must not move actual windows as a way of solving layout. A cycle or exhausted budget returns a conflict or last-good result.

**ARCH-03 — Scope and generation control.** Use per-scope request generations plus central per-window claim arbitration. Conflicting plans are serialized, merged, superseded, or rejected explicitly. A global cancellation counter must not unnecessarily cancel disjoint work. Already-issued native effects are not assumed cancellable.

**ARCH-04 — Event attribution.** Native observations are evidence, not authored edits. Correlate them with pending commands and explicit user interaction where possible. When provenance is ambiguous, reconcile current state without automatically committing it as a user's saved layout. Event callbacks must not perform blocking application calls or recursively trigger full reflow. `SetWinEventHook` supports out-of-context event observation but still requires careful asynchronous and reentrant handling. [S04]

### 15.3 Native operation contract

**ARCH-05 — Bounded execution.** The UI, hotkey dispatcher, and recovery controller MUST remain responsive while an application is stalled. Keep potentially blocking native operations outside their critical paths. Use supported asynchronous paths and compatibility-specific timeouts where appropriate; asynchronous submission is not settlement. [S01, S14]

**ARCH-06 — Minimal batch application.** Use supported batched positioning where suitable, accounting for the same-parent requirement and failure semantics of `DeferWindowPos`. Reuse its returned batch handle correctly; abandon a failed batch as documented. Do not assume all batches or cross-application redraw form an atomic visual transaction. [S02, S03]

**ARCH-07 — No synthetic live-resize animation.** The P1 transition executor MUST NOT implement animation by repeatedly resizing external applications through intermediate rectangles. Compute one final geometry and submit it once per plan. Any later correction must be justified by a new observation or constraint and recorded separately, not hidden inside an animation loop.

**ARCH-08 — Focus and Z-order isolation.** Position, size, visibility, Z-order, topmost state, and foreground intent are independently represented. Preserve unrequested properties. Windows supports distinct positioning flags, while foreground activation is restricted and may be denied. [S01, S09]

The system must not use input injection or unrelated user-gesture simulation to bypass a denied activation request.

### 15.4 Geometry basis and DPI

**ARCH-09 — Coordinate types.** Persist logical geometry with explicit units and retain observed physical client/frame rectangles separately. Every geometry value identifies coordinate space, display profile, DPI basis, and whether it describes client content or outer frame. Negative virtual-screen coordinates are valid.

`GetWindowRect` is DPI-virtualized and can include invisible resize borders; extended frame bounds use different DPI treatment. These values must not be mixed without conversion. `WM_SIZE` concerns the client area, and DPI-aware applications may receive `WM_DPICHANGED` on a cross-DPI move. [S06, S07, S08]

**ARCH-10 — Geometry policies.** Support these distinct policies:

| Policy | Contract |
|---|---|
| Saved logical layout | Recompute the saved size for the current layout context and adapter. |
| Preserve current client dimensions | On a compatible same-DPI context, retain observed client width/height exactly. |
| Preserve monitor and dimensions | Do not transfer the target to another display as part of preservation. |
| Logical-size continuity across displays | Preserve the declared logical extent, accepting that physical pixels and application DPI behavior may change. |

A request must not silently switch between these meanings. A fractional/rounding tolerance can be used for ordinary logical placement and settlement, but it cannot hide a resize in a strict-preservation acceptance test.

**ARCH-11 — Application constraints.** Observe or query size limits only through bounded supported mechanisms. Treat limits as unknown where unavailable. If an app rejects the target rectangle, settle to an explicit divergent state, use an allowed fallback, or stop managing the affected dimension. Do not alternate forever between manager and application preferences.

### 15.5 Optional platform features

**ARCH-12 — Native virtual desktops are an adapter.** The View model MUST NOT be a one-to-one alias of Windows virtual desktops. The documented `IVirtualDesktopManager` methods cover membership lookup and movement, not this product's complete composition and transition model. Cross-desktop operation must have explicit support and cannot silently switch all contexts. [S13]

**ARCH-13 — Observation surfaces.** P2 MAY offer live thumbnails or cached previews. They are labeled observation surfaces, not independently interactive instances. DWM thumbnails provide source-to-destination visual relationships; they do not create another application session. Their memory, privacy, and visibility behavior requires separate budgeting. [S10]

**ARCH-14 — Gaming compatibility.** Do not require a permanent full-display overlay over a protected game. Microsoft documents that overlapping content can change the flip-model composition path. Benchmark manager chrome and optional overlays independently rather than assuming they are free or always harmful. [S15]

---

## 16. Performance and observability

### 16.1 Distinct measurements

**PERF-01 — Timing definitions.** Record separately:

| Metric | Start and finish |
|---|---|
| Planning latency | Accepted immutable request to validated plan. |
| Submission latency | Plan commit to native requests issued. |
| Geometry settlement | Commit to observed required geometry/show-state result, or timeout. |
| Application readiness | Commit to provider-confirmed or experimentally observed usable content; otherwise unknown. |
| UI responsiveness | Input command to local visible acknowledgment, independent of stalled apps. |

Measure median, p95, p99, worst case, operation counts, and failures. Aggregate results by application class and transition type, not just window count. Do not equate a return from a positioning API with all applications being redrawn. [S03]

### 16.2 Proposed engineering targets

These are initial targets to validate, not advertised guarantees or existing benchmark results.

| Area | Initial target / gate |
|---|---|
| Planning | p95 at or below 5 ms for the declared reference fixture of 16 live windows and a small nested layout. Report formula limits and hardware. |
| Primary normal-work readiness | Investigate p95 at or below 100 ms on the supported reference application mix; report app-specific outliers instead of claiming universal compliance. |
| Protected unchanged windows | Zero unnecessary move/resize/show/hide/Z-order/activation requests. This is a hard functional gate. |
| Strict same-DPI size preservation | Zero intended client-size changes for retained supported windows. This is a hard functional gate. |
| Queue behavior | No growing backlog of obsolete transitions; latest accepted intent eventually wins when target applications become responsive. |
| Recovery | Main-process failure cannot remove access to the independent recovery action. Supported manager-hidden windows pass the crash-recovery suite. |
| Idle behavior | No full-desktop busy polling or continuous full-resolution screenshot capture by default. |

**PERF-02 — Reference fixtures.** P0 MUST test 4, 8, and 16 windows with: movement only; partial and widespread resizing; hide/show; semantic and responsive tabs; same and mixed DPI; disconnected monitors; hung applications; and rapid A/B/C requests. Use actual editor/browser/terminal applications and the game-plus-broadcast configuration where available.

**PERF-03 — Concurrent-load measurement.** Record CPU/GPU load, memory, game frame-time distribution, and available broadcaster render/encode statistics while switching only the work region. Not touching the game's geometry is necessary but does not prove there is no shared-resource performance impact.

**PERF-04 — Structured diagnostics.** Each committed transition logs request ID, scope, revisions, selected mode, affected bindings, mutation counts, settlement results, supersession, and fallback reasons. Logs redact titles/paths by default and avoid screenshot capture unless explicitly enabled. A human-readable impact report is derived from the same plan.

**PERF-05 — Memory policy.** Saved Views primarily store metadata. Full-resolution images for every View MUST NOT be required. Optional thumbnails use explicit size limits, eviction, refresh policy, and privacy controls. Slow formulas and extensions receive separate accounting so they cannot masquerade as native application latency.

---

## 17. Extension, automation, and security boundaries

**EXT-01 — Stable extension responsibilities.** Define separate interfaces for metadata/binding providers, layout strategies, application capability profiles, visibility strategies, attention sources, output-state providers, command contributions, and optional observation surfaces. A provider reports capability and evidence; it does not acquire unrestricted authority over every window.

**EXT-02 — One command pathway.** GUI actions, shortcuts, a local CLI, and future AI clients use the same validated command layer. Commands declare arguments, resolved scope, required capabilities, side effects, dry-run support, and undo behavior. A future AI agent must not need to simulate UI clicks to use supported operations.

P1 requires command-layer conformance; a public remote API and autonomous-agent client are P2 delivery features.

**EXT-03 — Authority and confirmation.** Pure layout/package data cannot invoke app execution, arbitrary shell commands, input injection, output switching, or credential access. Commands that close real windows or affect verified output require separate authority from ordinary arrangement changes. Confirmation policy may be configured for an explicitly trusted local user action, not bypassed by a formula.

**EXT-04 — Local transport boundaries.** If IPC is exposed, authenticate the same-user client and constrain its permissions. Do not bind an unauthenticated control API to the network. Remote access is a separate feature with its own authentication, authorization, and audit design.

**EXT-05 — Executable plugins.** Third-party executable plugins are P2 and not implicitly sandboxed by being called plugins. Their privileges must be declared, isolation must be real where promised, and untrusted formula evaluation must not run in their unrestricted environment. Built-in adapters may be trusted components but still obey planner and scope contracts.

**EXT-06 — Configuration safety.** Imports enforce size/depth limits, typed schemas, safe file handling, and explicit mappings. Diagnostic/export bundles redact sensitive metadata. A corruption or migration error enters safe mode rather than executing a partially interpreted configuration.

---

## 18. Logical data and command contracts

The following records define required semantics, not a database schema or programming language. Equivalent representations are acceptable if all state and acceptance contracts are preserved.

### 18.1 Minimum logical records

| Record | Required fields / relationships |
|---|---|
| `Workspace` | Stable ID, revision, name, View references, Collection references, remembered default target. |
| `WindowRef` | Stable ID, user alias, tags with provenance, binding hints, optional application/provider identity. |
| `LiveBinding` | WindowRef ID, platform/session identity, handle, generation, process/lifetime evidence, observed capabilities and geometry. |
| `Collection` | ID, revision, typed query, include/exclude references, stable sort, optional parameter definitions. |
| `View` | ID, revision, Workspace ID, named root roles, Group/Placement tree, default parameters. |
| `Group` | ID, ordered children, optional selector/Collection, strategy, semantic-tab or folding policy, responsive variants, constraints, fallback list. |
| `Placement` | ID, WindowRef or explicit unresolved role binding, local role, local exceptions, optional shared-size reference. |
| `LayoutPreference` | View/node/context/variant key, explicit property overrides, unit basis, provenance, revision. |
| `DisplayProfile` | Stable monitor mappings, work areas, DPI/orientation information, unresolved mapping state. |
| `DisplaySlot` | ID, profile-specific bounds and monitor mapping, constraints, optional explicit overlay permissions. |
| `ScreenComposition` | ID, revision, presentation bindings, preserved/operating slots, optional output designation. |
| `Presentation` | Stable binding identity, View/root-to-slot mapping, current context, Visit ID, selected tabs, local runtime overlays. |
| `TransitionRequest` | Request ID, explicit scope, target IDs, mode, retained targets/property mask, focus intent, partial-apply policy, expected revisions. |
| `TransitionPlan` | Generation, snapshot revisions, claims, final mutations, diagnostics, impact summary, dependency components. |
| `TransitionResult` | Status, per-window submission and observation outcomes, unknown readiness, fallback actions, undo reference. |
| `RecoveryEntry` | Validated live identity, prior state, manager-induced mutation, generation, observed result, recovery eligibility. |
| `LayoutPackage` | Schema/language version, ID/version, role/parameter schema, declarative content, dependencies, privacy-safe metadata. |

A missing binding and an unresolved template role are different states. A role must be resolved before a native operation can be emitted. Dynamic selectors create stable Placements; they do not replace persistent identity with an ephemeral list index.

### 18.2 Required command families

| Command family | Example operations | Mutation boundary |
|---|---|---|
| Query | List Workspaces, View tree, live bindings, claims, capabilities, attention. | Read-only. |
| Structure | Create View; wrap/unwrap Group; add/copy/move Placement; create semantic tabs. | Revisioned authored state. |
| Selection | Edit query; include/exclude; tag; set local role. | Explicit membership scope only. |
| Composition | Bind roots to slots; open target in scope; restore display profile. | Saved composition or runtime presentation, explicitly selected. |
| Transition | Plan; apply; preserve size; retain here; temporarily bring; cancel pending request. | Runtime mutations and Visit-local exceptions. |
| Persist geometry | Save selected properties here; copy size; clear local override. | Specified LayoutPreference records only. |
| Protection | Set geometry/visibility/focus/output restrictions with lifetime. | Explicit scoped constraint. |
| Recovery | Pause; reveal manager-hidden; recover off-screen; undo action. | Validated manager-owned changes. |
| Application control | Close window; launch target; provider-specific action. | Separate permission and command family. |

### 18.3 Request and result semantics

**API-01 — Validation before mutation.** Commands MUST validate target identity, scope, capability, schema, and expected revision. Unknown fields in strict control commands are rejected rather than interpreted as defaults that widen scope.

**API-02 — Idempotency.** Mutating command requests accept a request/idempotency identifier. Retrying an accepted command must not create duplicate Groups, duplicate application launches, or additional Visits unintentionally. Plan IDs are valid only for their declared snapshot; stale application requires revalidation/replanning.

**API-03 — Explicit error model.** Use machine-readable error classes at minimum: `STALE_REVISION`, `STALE_BINDING`, `TARGET_MISSING`, `AMBIGUOUS_BINDING`, `CLAIM_CONFLICT`, `OUT_OF_SCOPE`, `UNSATISFIABLE_CONSTRAINTS`, `UNSUPPORTED_OPERATION`, `FORMULA_INVALID`, `FORMULA_BUDGET_EXCEEDED`, `APPLICATION_TIMEOUT`, `OUTPUT_STATE_UNKNOWN`, and `PERMISSION_DENIED`. Include a human-readable explanation and affected object IDs.

**API-04 — Events.** Publish domain events for committed configuration edits, binding lifecycle, membership changes, attention, plan/result status, topology changes, and recovery actions. Native platform event names are adapter details. Event subscription must support obtaining a current snapshot after a missed-event interval; clients must not assume their local cache is complete forever.

### 18.4 Illustrative transition request

This is an example semantic payload, not a final wire schema:

```json
{
  "requestId": "example-request-001",
  "action": "presentation.open",
  "scope": { "slotIds": ["left-work"] },
  "target": {
    "viewId": "project-a-review",
    "rootBindings": { "main": "left-work" }
  },
  "mode": "preserve-selected-size",
  "preserve": [{
    "windowRefId": "preview-browser",
    "properties": ["clientWidth", "clientHeight"],
    "strength": "required",
    "basis": "current-monitor-dpi",
    "duration": "destination-visit"
  }],
  "focus": { "policy": "preserve-current" },
  "partialApply": "independent-components-only",
  "expectedViewRevision": 12
}
```

The executor must not silently add `center-game` or `right-operations` to this request. If retaining `preview-browser` conflicts with a protected claim outside `left-work`, the result identifies that conflict and proposes an explicit resolution.

---

## 19. Acceptance scenarios and requirement traceability

A passing result must be demonstrated with both deterministic planner tests and real-window integration tests where applicable. A unit test using synthetic rectangles is not evidence of application compatibility or visual readiness. Test observations include saved-state diffs, native operation logs, observed geometry, focus, and recovery accessibility.

### 19.1 Core state and membership

| Test | Given / action | Required result | Requirements |
|---|---|---|---|
| AT-01 | The same browser has different saved sizes in Implementation and Review; open each normally. | Each occurrence restores its own preference; neither overwrites the other. | STATE-01, STATE-03 |
| AT-02 | Two semantic tabs in one View reference the same preview. | Each tab has independent geometry; only the selected occurrence owns the window. | MODEL-04, LAY-03, LAY-04 |
| AT-03 | Rename/reorder a View, Group, and Placement. | IDs, geometry preferences, and fixed shortcuts remain attached to the same targets. | MODEL-01, DSP-04 |
| AT-04 | Add a nonmatching window to a View. | It is included locally; global tags and source View membership are unchanged. | MEM-03, MEM-08 |
| AT-05 | A window matches a query but has an explicit exclusion. | It stays excluded; inspector names the responsible exclusion and re-include action. | MEM-03, MEM-07 |
| AT-06 | A negative filter references unavailable privacy metadata. | Unknown does not become proof of public safety. | MEM-02, PRO-06 |
| AT-07 | Dynamic query results reorder or briefly disappear and return. | Persistent dynamic children retain identity and eligible saved preferences. | MEM-05 |
| AT-08 | An active typing window no longer matches its selector. | Pending removal is shown; no manager-induced disruptive hide occurs during detected interaction. | MEM-11, EDIT-04 |
| AT-09 | A temporary filter is applied and cleared. | Saved View revision and geometry remain unchanged; prior presentation is restored subject to availability. | MEM-10 |

### 19.2 Preservation and transition behavior

| Test | Given / action | Required result | Requirements |
|---|---|---|---|
| AT-10 | Preview is 1200×800; Review saves 1800×1000; invoke preserve-size Review. | Client size stays 1200×800 on a compatible same-DPI setup; Review still saves 1800×1000. | TRN-02, TRN-03, TRN-06 |
| AT-11 | Leave the preserved Review visit, then open Review normally. | 1800×1000 is restored; no temporary override leaks into the new Visit. | STATE-04, TRN-06 |
| AT-12 | Select Save here while the temporary size is active. | Only addressed size properties in the destination context change; source preferences remain intact. | STATE-05, TRN-07 |
| AT-13 | Keep a preview here while replacing surrounding tools. | Preview client/frame geometry remains unchanged; other children respect its reserved area. | TRN-05, DSP-06 |
| AT-14 | Retain a window absent from the destination View. | A temporary carry Placement appears; no saved membership or tags change. | TRN-04 |
| AT-15 | Retain a window with multiple eligible destination occurrences. | Intended branch is resolved explicitly or from selected semantic state, not applied to all occurrences. | TRN-04, MODEL-04 |
| AT-16 | Preserved child is larger than the destination slot. | Fit conflict or allowed fallback; no silent resize, hidden window, or overlap with a protected slot. | LAY-11, EDIT-06 |
| AT-17 | Copy a window size to three Placements and later resize one. | The later resize changes only that Placement; no implicit link was created. | TRN-07 |
| AT-18 | Switch candidates repeatedly in the quick switcher without committing. | Zero native geometry or visibility mutations occur. | LAY-12, UX-07 |
| AT-19 | Recall the already active View with a preservation exception. | No mutation and no silent exception reset; explicit restore remains available. | DSP-05, STATE-04 |
| AT-20 | Submit A→B→C rapidly while B's app responds late. | Latest C target wins; old observations do not save into C or replay a queued B transition. | TRN-09, ARCH-03, ARCH-04 |
| AT-21 | Submit independent changes in two slots. | Neither is cancelled solely because the other received a newer request. | TRN-09, ARCH-03 |
| AT-22 | A preserved-size target is moved to another DPI display. | Declared basis/fallback is followed; no false strict-preservation success is reported. | TRN-03, ARCH-09, ARCH-10 |

### 19.3 Groups and formulas

| Test | Given / action | Required result | Requirements |
|---|---|---|---|
| AT-23 | Drag a tab containing a nested Group. | Entire subtree moves; all IDs and internal preference state survive. | LAY-01, LAY-03, LAY-13 |
| AT-24 | Resize a Group from wide split to compact responsive tabs and back. | Wide ratios return; selected compact tab prioritizes the previously focused child. | LAY-07, LAY-08 |
| AT-25 | Widen a semantic tab group containing duplicate window references. | It stays mutually exclusive; no simultaneous duplicate claim is created. | LAY-04, MODEL-04 |
| AT-26 | Resize a parent Group. | Child Groups adapt to allocated space, not global monitor width; final native sizes are applied only once. | LAY-05, TRN-10 |
| AT-27 | Enable preserve-child-size and give excess space. | Child dimensions remain fixed; visible gaps/alignment are valid. | LAY-09 |
| AT-28 | Manually resize a formula-controlled child. | A local override or exposed parameter changes; the formula is not silently replaced. | EDIT-01, EDIT-02 |
| AT-29 | Enter an invalid expression, a cyclic rule, and an over-budget rule. | Invalid draft is not committed; last-good layout remains; diagnostic identifies the cause. | FORM-02, FORM-04, FORM-06 |
| AT-30 | Import a layout from different tags and monitors. | Mapping is required; no arbitrary local binding or live movement occurs before approval. | FORM-08, FORM-09 |
| AT-31 | A full nested Group is unwrapped or removed. | Unwrap retains children; remove removes arrangement references; neither closes apps. | LAY-13, CORE-02 |
| AT-32 | A tab requires more preferred space than its sibling. | Tab selection does not resize the outer Group unless explicit content-fit behavior permits it. | LAY-06 |

### 19.4 Concurrent work, monitors, and privacy

| Test | Given / action | Required result | Requirements |
|---|---|---|---|
| AT-33 | Game, AI work, operations, and chat occupy independent slots; change only AI View. | Zero unnecessary mutations to game/operations/chat; their manager UI state remains intact. | DSP-03, TRN-10, PRO-01 |
| AT-34 | An AI provider reports approval needed while the game has focus. | Attention appears in the configured control surface; no automatic activation, approval, or View switch. | PRO-03, PRO-04, PRO-05 |
| AT-35 | User selects that attention event. | The designated AI slot and required tab path open; unrelated slots are not replaced. | UX-06, PRO-04 |
| AT-36 | A continuously displayed monitor is nested under a Group that wants to fold it. | Ancestor folding is blocked or uses an explicitly allowed summary; no silent loss of visibility. | PRO-02 |
| AT-37 | One real window is selected by two simultaneously visible Views. | Existing valid owner stays; other occurrence reports a conflict and offers explicit transfer. | MODEL-04, MEM-06 |
| AT-38 | The source window is protected or known output-linked; request transfer from another scope. | Transfer is blocked pending explicit resolution; unrelated ownership is not stolen. | DSP-03, PRO-01, PRO-08 |
| AT-39 | Remove a monitor during a designated-public composition. | Private controls are not silently relocated into a public region; original layout remains saved. | DSP-07, PRO-09, PRO-11 |
| AT-40 | Restore the original monitor topology. | Saved context returns under the chosen policy; fallback edits and new windows remain accounted for. | STATE-06, DSP-08 |
| AT-41 | Capture provider disconnects while reporting public output. | State becomes unknown/stale; verified-safety claims stop and dependent automatic changes are restricted. | PRO-06, PRO-10 |
| AT-42 | Reuse a publicly designated browser in a private View. | Shared-content warning remains; no false claim of isolated browser state. | PRO-07 |
| AT-43 | Expand a Group toward a protected operating slot. | Only explicitly permitted scope is used; expansion cannot silently cover the protected slot. | LAY-14, DSP-06 |

### 19.5 Platform failures and recovery

| Test | Given / action | Required result | Requirements |
|---|---|---|---|
| AT-44 | One application stalls during a transition. | Hotkeys/recovery remain responsive; independent components can progress with explicit partial status. | TRN-13, ARCH-05 |
| AT-45 | An application repeatedly rejects the requested size. | Bounded correction then explicit divergence/fallback; no infinite resize fight. | WIN-07, ARCH-11 |
| AT-46 | A live handle is destroyed and its value is reused. | Stale transition/recovery work never controls the replacement window through the obsolete binding. | MEM-01, REC-02 |
| AT-47 | Kill the manager after it has hidden supported windows. | Independent recovery validates and restores manager-hidden windows; pre-existing user minimization is respected. | REC-01, REC-02, REC-03 |
| AT-48 | User minimizes a maintained-visible window. | User intent is honored as a scoped exception rather than immediately undone by the manager. | WIN-03 |
| AT-49 | App creates an owned modal dialog during a tab/View operation. | Dialog is accessible and associated with its owner; blocked owner is not falsely reported ready. | WIN-02, TRN-11 |
| AT-50 | Restart with ambiguous same-title windows. | User binding choice is required; no duplicate app launch or arbitrary attachment. | MEM-01, REC-06 |
| AT-51 | Undo after another slot has claimed a changed window. | Conflict is surfaced/reconciled; undo does not steal it from the new owner. | REC-07, EDIT-08 |
| AT-52 | Launch a corrupted configuration or incompatible package. | Safe mode/validation failure without partial interpretation or automatic window manipulation. | REC-05, EXT-06 |
| AT-53 | Register conflicting shortcuts; use Korean IME during editing. | Registration conflict is visible; normal composition is not silently intercepted or corrupted. | UX-08 |
| AT-54 | Change only geometry while the application denies activation. | Geometry and activation results remain distinct; no repeated synthetic-input focus workaround. | TRN-12, ARCH-08 |
| AT-55 | Execute a layout command with an expired plan revision or a wider hidden scope field. | Revalidation/rejection, with no unauthorized widening of the mutation set. | API-01, API-02 |
| AT-56 | Keep a protected full-screen game while changing neighboring work windows. | Show-state and geometry remain untouched; actual performance impact is recorded separately. | WIN-04, ARCH-14, PERF-03 |

### 19.6 Test layers

The implementation should provide a deterministic model/planner test suite, a fake platform adapter with delayed/reordered lifecycle effects, a real-window harness, a persistence/migration and crash-recovery suite, and scenario-level manual usability tests.

Property-based tests should verify the invariants in STATE-07 across generated group trees, duplicate references, viewport sizes, membership edits, and transition sequences. Deterministic tests must cover zero candidates, one child, many children, empty tabs, nested tab paths, invalid metrics, and missing bindings.

Record failures by capability and application version. A supported profile is not merely an application executable name: tested display scaling, show state, visibility strategy, and relevant rendering behavior matter.


### 19.7 Review and measurement gates

The scenario matrix is supplemented by the following checks. Together they provide an explicit verification destination for every requirement ID. A deferred P2 capability does not pass its gate merely because a placeholder interface exists.

| Gate | Verification evidence | Requirements |
|---|---|---|
| VG-01 — Product and domain boundaries | Inspect the data model and a multi-root/two-Workspace fixture. Demonstrate acyclic structure, shared resources, stable Display Slots, and no dependency on an IDE or PWA shell. | CORE-01, CORE-03, MODEL-02, MODEL-03, DSP-01, DSP-02 |
| VG-02 — State scope | Compare serialized state across two layout contexts, local role edits, an inactive rule update, and a manual change. Only the intended context/properties change; inactive evaluation emits no native commands. | STATE-02, EDIT-03, EDIT-05, MEM-04 |
| VG-03 — Layout strategy contract | Exercise each supported strategy, all three group-preservation meanings, distinct pin properties, and one failed child rule. Confirm correct dependent-subtree isolation. | LAY-02, LAY-10, LAY-15, EDIT-07 |
| VG-04 — Interaction review | Complete first-run grouping, exact-scope switching, preservation selection, inventory recovery, deep ancestor navigation, and no-motion use without formulas or required hidden shortcuts. Inspect plan impact and exception provenance. | UX-01, UX-02, UX-03, UX-04, UX-05, UX-09, UX-10, TRN-01, TRN-08, MEM-09 |
| VG-05 — Formula/package review | Verify typed property formulas, declared context, synthetic preview, explicit application, privacy-safe exports, and independent-copy import. For P2 links, verify versioned staging and local override preservation. | FORM-01, FORM-03, FORM-05, FORM-07, FORM-10, FORM-11 |
| VG-06 — Authority review | Attempt unsupported capability use from each command origin. Confirm common planner enforcement, local transport authentication where present, no executable permissions from layout data, and separation of privileged/plugin execution. | EXT-01, EXT-02, EXT-03, EXT-04, EXT-05, WIN-08 |
| VG-07 — Native execution review | Inspect the native-top-level/logical-group boundary, immutable snapshots, batch failure paths, actual resize counts, desktop adapter isolation, and any optional thumbnail behavior. No real-window animation or interactive-clone claim is hidden in an adapter. | ARCH-01, ARCH-02, ARCH-06, ARCH-07, ARCH-12, ARCH-13 |
| VG-08 — Lifecycle support matrix | Verify opt-in management, per-app visibility profiles, render-liveness labels, and graceful exit using pre-existing minimized and manager-hidden windows. | WIN-01, WIN-05, WIN-06, REC-04 |
| VG-09 — Performance evidence | Publish the reference fixtures, timing definitions and distributions, structured/redacted logs, idle measurements, formula accounting, and thumbnail/cache limits where applicable. | PERF-01, PERF-02, PERF-04, PERF-05 |
| VG-10 — API lifecycle | Exercise every declared error class and recover a client from a missed-event interval using a fresh snapshot; native event details do not become the public domain protocol. | API-03, API-04 |

---

## 20. Delivery plan and release gates

### P0 — Feasibility and correctness harness

Build the live binding registry, scoped transition planner, minimal executor, observation/reconciliation loop, and independent reveal/recovery mechanism. Configuration may be hand-authored; polished tagging and layout editors are not required at this stage.

Required demonstrations:

- Two saved layouts for the same windows, plus strict same-DPI keep-size and keep-here transitions.
- Native operation diffs that leave unrelated slots untouched.
- One nested split/semantic-tab example and a compact responsive variant.
- Rapid supersession, a hung application, handle-lifetime changes, and a forced manager crash.
- Same/mixed-DPI display movement and a display-disconnect fallback.
- A recorded visibility-strategy and application-compatibility decision.

**P0 gate:** Pass the applicable state-isolation, minimal-mutation, latest-intent, identity, and recovery tests before expanding into a full manager. Publish measured behavior and unresolved application exceptions. Failure in one application may justify a restricted capability profile rather than invalidating the whole model.

### P1 — Coherent personal-use release

Deliver all requirements not explicitly identified as P2, including stable Workspace/View organization; reusable Window references; tags and basic query editing; independent Placement preferences; recursive Groups and both tab semantics; Display Slots and Screen Compositions; partial switching; size-preserving/keep-here/temporary-bring operations; responsive fallbacks; basic bounded formulas with import/export mapping; local manual overrides; undo; private-designated control UI; and crash recovery.

P1 must include the command-layer semantics even when the CLI is minimal. It may use built-in adapters only. Output state may remain unverified, but the UI must make that limitation explicit.

**P1 gate:** Pass applicable AT-01 through AT-56 and VG-01 through VG-10 against the published support matrix. Provider-dependent tests use test providers for contract validation and are not advertised as real integrations until validated against the actual application. Document unsupported application categories and privacy/output limitations prominently.

### P2 — Advanced providers and reuse

Potential additions are linked size profiles, linked/versioned Layout Presets, richer metadata and attention providers, verified output integration, observation-only mirrors, additional layout algorithms, richer application-internal state adapters, authenticated remote control/PWA control surfaces, and third-party executable plugins.

P2 does not relax P1 invariants. Its providers must participate in scope validation, effect planning, permission checks, privacy handling, and undo/recovery classification.

### Release compatibility statement

Every release must list tested Windows builds, privileges, display configurations, application profiles, known failed operations, visibility strategy, and recovery support. Features depending on private APIs must be individually identified. No release should be described as universally application-compatible or broadcast-safe solely because ordinary placement tests pass.

---

## 21. Decision register and unresolved validation items

### 21.1 Baseline product decisions

| Decision | Resolution |
|---|---|
| Product category | Window-management layer over existing applications, not a new IDE. |
| Workspace meaning | Organizational context, never a single global exclusive desktop. |
| View meaning | Reusable arrangement with root roles; not the complete physical composition. |
| Physical arrangement | Screen Compositions bind View roots to Display Slots. |
| Layout ownership | Placement/context/variant, not global per-window geometry. |
| Same window in multiple places | Multiple saved occurrences; one active interactive owner. |
| Group content | Recursive Groups and Window Placements share the same tree model. |
| Tabs | Semantic alternatives and responsive folding have distinct semantics. |
| Normal recall | Stable target and scope; idempotent if already active. |
| Size-preserving switch | Explicit Visit-local override, never implicit persistent saving. |
| Keep-here switch | Preserve geometry and reserve space; switch only eligible surroundings. |
| Manual edits | Property-level, local by default, undoable. |
| Imported layouts | Preview and environment mapping; independent copy by default. |
| Automatic motion | Existing-position-first by default; no real-window animation requirement. |
| Native integration | Logical group tree with top-level native windows; no mandatory reparenting. |
| Broadcast awareness | Conservative designated-region handling; verified output requires a provider. |
| Application lifetime | Independent of View and Group lifetime. |

### 21.2 Implementation selections to validate, not unresolved UX

| Item | Selection criterion | Default until selected |
|---|---|---|
| Language and UI toolkit | Native event integration, reliable input/IME, fast local UI, maintainability. | No framework is mandated. |
| Exact supported Windows builds | P0 compatibility and recovery evidence. | Do not claim support outside tested builds. |
| Visibility strategy per app class | Hide/restore correctness, taskbar behavior, rendering, modal behavior, crash recovery. | Capability-gated; unknown apps remain reachable rather than aggressively hidden. |
| Elevated helper | Least privilege and ability to isolate privileged operations. | Ordinary user mode only. |
| Formula grammar and budgets | Typed units, bounded evaluation, explainability, P0 performance. | Use the FORM semantic contract; no arbitrary host code. |
| Native foreground behavior | Explicit user-action tests and OS restrictions. | No automatic focus requests; report denied requests. |
| Application minimum size discovery | Bounded supported observation/query. | Unknown constraints with safe fallback, not invented minima. |
| Topology identity heuristics | Reconnect, docking, duplicate monitors, rotation, scaling. | Confirm ambiguous mappings. |
| Optional clipping/scrolling strategy | Real native-window compatibility and no misleading interaction promises. | Tabs, wrapping, or explicit overflow instead of assumed clipping. |
| Measured performance envelope | Actual reference hardware, workloads, and percentile measurements. | Engineering targets in Section 16 are unverified. |
| Verified broadcast integration | Output target identity, status freshness, actual capture behavior, acknowledged actions. | Output state unverified; no privacy guarantee. |

These items are recorded decisions for engineering validation, not reasons to postpone the specification's state model or start implementing contradictory defaults.

---

## 22. Implementation handoff checklist

Implementation work is ready to proceed when the team or coding agent has:

1. Adopted the concept names, state-ownership matrix, and STATE-07 invariants without collapsing View, Workspace, and Screen Composition into one object.
2. Selected the first tested OS/build and application fixture, and defined how unsupported windows remain accessible.
3. Defined stable IDs, configuration revisions, live binding generations, and per-scope transition generations.
4. Written planner tests for preserve-size, keep-here, temporary carry, duplicate active claims, and hard fit failure before adding elaborate UI.
5. Made authored state, observed state, current targets, and temporary Visit overrides separate records.
6. Implemented independent recovery before relying on hiding or minimizing applications.
7. Preserved semantic versus responsive tab intent and variant-specific manual geometry.
8. Required every UI/shortcut/provider action to pass through the same scope and protection validation.
9. Identified which results are measured, observed, inferred, unsupported, or unknown in diagnostics.
10. Assigned delivery stages and acceptance IDs to implementation issues rather than treating this document as an undifferentiated feature list.

### Definition of done for an implementation issue

The issue names affected requirement IDs; states whether it changes persisted, runtime, application, or output state; includes normal and failure-path tests; specifies undo/recovery behavior; and documents unsupported capabilities. A UI that appears correct while corrupting saved preferences or issuing unnecessary native operations is not complete.

### Product-level completion criterion

The user can switch a chosen task region, preserve the windows they are actively using, and return to independent saved layouts without disturbing unrelated work. When a constraint cannot be honored, the system explains the conflict and leaves a recoverable state rather than silently substituting a different behavior.

---

## 23. Technical references and evidence boundaries

Official documentation was checked on **2026-10-09**. These references support the limited platform observations below; they do not validate performance targets, every third-party application's behavior, or the entire proposed product design. Source IDs used elsewhere in this document refer to this section. URLs are supplied as code text for portability in plain Markdown.

### [S01] Microsoft — SetWindowPos

Documents independent flags for retaining position, size, Z-order, and non-activation, plus conditional asynchronous positioning. It supports separating those properties in the execution model; it is not a universal latency guarantee.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos`

### [S02] Microsoft — DeferWindowPos

Documents batch construction, the same-parent constraint, updated handle handling, and abandoning the operation after failure. Used for the batching adapter's contract.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-deferwindowpos`

### [S03] Microsoft — EndDeferWindowPos

Documents updating window positions and sizes together and delivery of position-changing/changed messages. It does not establish that unrelated applications have all completed content redraw when the call returns.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enddeferwindowpos`

### [S04] Microsoft — SetWinEventHook

Documents native event hooks, asynchronous out-of-context delivery, message-loop requirements, and reentrancy considerations. The proposal's generation/provenance model is a design response, not a claim that native events contain our transition IDs.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook`

### [S05] Microsoft — SetParent

Documents style-handling considerations and unexpected behavior/errors when different DPI-awareness modes are combined. Supports avoiding cross-process reparenting as a foundational requirement.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent`

### [S06] Microsoft — WM_SIZE

Documents size-change notifications and client-area dimensions. Supports the distinction between application content size and a Group's outer allocated rectangle.

`https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-size`

### [S07] Microsoft — GetWindowRect

Documents DPI virtualization, possible invisible resize borders, and the different handling of extended frame bounds. Supports explicit coordinate/frame conversions.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect`

### [S08] Microsoft — WM_DPICHANGED

Documents DPI-change notification and the suggested new rectangle. Cross-monitor logical-size continuity is therefore not equivalent to guaranteeing unchanged physical client pixels or app rendering work.

`https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged`

### [S09] Microsoft — SetForegroundWindow

Documents restrictions on setting the foreground window, including possible denial. Supports separating explicit focus requests from geometry updates and reporting denied activation.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow`

### [S10] Microsoft — DWM Thumbnail Overview

Documents live thumbnail relationships between source windows and destination rendering areas. This is a visual facility, not an independent application session or universal interactive window clone.

`https://learn.microsoft.com/en-us/windows/win32/dwm/thumbnail-ovw`

### [S11] Chromium — Windows Native Window Occlusion Detection

Documents rendering suppression and JavaScript throttling for occluded windows, including treatment of other virtual desktops. This is evidence of one major application's behavior, not a rule for every application.

`https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/windows_native_window_occlusion_tracking.md`

### [S12] OBS — Window Capture Sources

Documents capturing a selected window even when other windows are in front of it. Supports the requirement not to equate a local covering overlay with verified output privacy.

`https://obsproject.com/kb/window-capture-sources`

### [S13] Microsoft — IVirtualDesktopManager

Documents window desktop membership lookup and movement. Its published interface is not the multi-region, multi-placement View model defined here.

`https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ivirtualdesktopmanager`

### [S14] Microsoft — ShowWindowAsync

Documents asynchronous window show-state requests. Used as one available adapter mechanism, not a guarantee of application readiness or universal visibility compatibility.

`https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindowasync`

### [S15] Microsoft — For best performance, use DXGI flip model

Documents flip-model presentation paths and interactions with other desktop content. Supports measuring optional management overlays with games rather than assuming an invariant performance effect.

`https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model`

---

**End of specification.**
