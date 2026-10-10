# Window manager conformance evidence and support matrix

Status on 2026-10-10: implemented Windows feasibility product; **the P1 release gate is open**. This maps all AT-01–AT-56 and VG-01–VG-10 to available evidence and remaining review. A deterministic model result, an owned STATIC window, or a static GUI render is not an acceptance pass against a real browser/game/broadcaster. No existing user application was manipulated during verification.

## Supported and unverified profiles

| Profile | Evidence | Support boundary |
| --- | --- | --- |
| Ordinary-user Win32 STATIC fixture, normal state, same DPI | Native move-only, batch, explicit show-state, journal reveal, independent crash recovery | Only disposable windows created by the harness; rendering unknown |
| Own manager GUI at 192 DPI | Korean onboarding, parameters/synthetic preview, ancestor/structure editor, Library mapping renders; private-region guard | Static smoke checks, not full keyboard/IME or physical reconnect review |
| Arbitrary browser/editor/terminal | No application-version matrix run | Manual binding and per-resource profiles; hide/show-state opt-in; no advertised compatibility certification |
| Full-screen games / broadcaster / load-sensitive applications | Pure zero-unrelated-operation invariant only | Actual frame timing, CPU/GPU and encoder impact unmeasured |
| Elevated/protected/unsupported native windows | Ordinary-user boundaries and rejection paths | No elevation broker or bypass advertised |
| Mixed-DPI foreign windows / docking | Pure client/DPI settlement and topology fallback fixtures | Native transfer/reconnect distributions unmeasured; strict incompatible preservation rejects |
| Output/AI providers | Granted-resource/session/sequence/TTL/disconnect test contracts | Actual integrations deferred; output and rendering remain unverified |
| macOS/Linux adapters, linked templates, executable extensions, remote API, mirrors/clipping | No implementation | Outside Windows P1 baseline / P2 deferred |

## AT evidence

The linked function identifies relevant evidence, not the whole acceptance scenario. Model tests use synthetic observations. Worker/native/recovery fixtures use their own windows/processes. Every remaining column is still open where the scenario requires real applications, user interaction, hardware or provider integration.

| Acceptance item | Relevant checked fixture | Remaining scenario gate |
| --- | --- | --- |
| AT-01 | [preserved_size_is_visit_local_and_idempotent_recall_keeps_it](../libs/window-manager-core/src/tests.rs#L578) | Real browser contexts |
| AT-02 | [semantic_tabs_keep_occurrences_independent_even_when_widened](../libs/window-manager-core/src/tests.rs#L1098) | Real app visibility profile |
| AT-03 | [named_shortcuts_use_ids_and_fixed_roles_across_rename_and_workspace_selection](../libs/window-manager-core/src/tests.rs#L2088) | Keyboard/reordering usability |
| AT-04 | [local_membership_exclusions_preserve_ids_weights_and_source_queries](../libs/window-manager-core/src/tests.rs#L1054) | Manual add usability |
| AT-05 | [unknown_negative_query_never_proves_public_and_exclusions_win](../libs/window-manager-core/src/tests.rs#L686) | Exclusion/re-include usability |
| AT-06 | [unknown_negative_query_never_proves_public_and_exclusions_win](../libs/window-manager-core/src/tests.rs#L686) | Actual capture state unverified |
| AT-07 | [selector_membership_is_staged_and_restores_stable_preferences](../libs/window-manager-core/src/tests.rs#L1426) | Real lifecycle churn |
| AT-08 | [continuous_rules_defer_interaction_and_reflow_only_opted_in_groups](../libs/window-manager-core/src/tests.rs#L2237) | Korean IME and application-internal typing |
| AT-09 | [temporary_filter_clear_restores_layout_without_authored_state_drift](../libs/window-manager-core/src/tests.rs#L1699) | Interactive filter selection |
| AT-10 | [preserved_size_is_visit_local_and_idempotent_recall_keeps_it](../libs/window-manager-core/src/tests.rs#L578) | Real same-DPI browser |
| AT-11 | [leaving_a_visit_drops_its_override_and_normal_return_uses_saved_size](../libs/window-manager-core/src/tests.rs#L1147) | Real application context return |
| AT-12 | [manual_edit_saves_only_changed_properties_and_promotion_keeps_other_exception](../libs/window-manager-core/src/tests.rs#L1516) | Actual user resize hook provenance |
| AT-13 | [keep_here_reserves_space_for_surrounding_children](../libs/window-manager-core/src/tests.rs#L1123) | Real window reservations |
| AT-14 | [preserved_size_is_visit_local_and_idempotent_recall_keeps_it](../libs/window-manager-core/src/tests.rs#L578) | Carry-placement usability |
| AT-15 | [semantic_tabs_keep_occurrences_independent_even_when_widened](../libs/window-manager-core/src/tests.rs#L1098) | Explicit branch choice usability |
| AT-16 | [approved_resize_enforces_minima_capabilities_output_and_geometry_locks](../libs/window-manager-core/src/tests.rs#L2469) | Real fit/minimum constraints |
| AT-17 | [subtree_moves_copies_and_size_copies_are_atomic_local_and_independent](../libs/window-manager-core/src/tests.rs#L945) | Multi-selection keyboard review |
| AT-18 | [manual_onboarding_captures_current_arrangement_without_rules_or_native_mutations](../libs/window-manager-core/src/tests.rs#L2590) | Switcher pointer/keyboard no-mutation review |
| AT-19 | [preserved_size_is_visit_local_and_idempotent_recall_keeps_it](../libs/window-manager-core/src/tests.rs#L578) | Real app recall |
| AT-20 | [supersession_interrupts_a_stalled_scope_without_cancelling_disjoint_generations](../apps/window-manager/src/service.rs#L206) | Foreign-app late A→B→C effects |
| AT-21 | [unrelated_scope_and_new_claims_are_not_overwritten_by_old_plans](../libs/window-manager-core/src/tests.rs#L1164) | Independent real-app concurrency |
| AT-22 | [dpi_transfer_verifies_requested_client_size_instead_of_only_frame](../libs/window-manager-core/src/tests.rs#L163) | Chrome/Edge 96↔192 DPI unsupported; default compatibility gate added |
| AT-23 | [subtree_moves_copies_and_size_copies_are_atomic_local_and_independent](../libs/window-manager-core/src/tests.rs#L945) | Nested drag/drop interaction review |
| AT-24 | [responsive_folding_restores_wide_ratios_and_variant_preferences](../libs/window-manager-core/src/tests.rs#L1182) | Interactive responsive resize |
| AT-25 | [semantic_tabs_keep_occurrences_independent_even_when_widened](../libs/window-manager-core/src/tests.rs#L1098) | Real semantic visibility profile |
| AT-26 | [generated_layouts_have_one_final_mutation_per_window_and_no_saved_state_drift](../libs/window-manager-core/src/tests.rs#L1233) | Foreign native resize counts |
| AT-27 | [fixed_size_children_leave_flexible_remainder_and_allowed_fallback_folds](../libs/window-manager-core/src/tests.rs#L1794) | Single Chrome/Edge app-window move-only checked; larger arrangements remain |
| AT-28 | [manual_edit_saves_only_changed_properties_and_promotion_keeps_other_exception](../libs/window-manager-core/src/tests.rs#L1516) | Real gesture/IME provenance |
| AT-29 | [formulas_are_bounded_pure_typed_and_lazy](../libs/window-manager-core/src/tests.rs#L666) | Rule diagnostics usability |
| AT-30 | [packages_require_explicit_mapping_and_have_no_live_identity](../libs/window-manager-core/src/tests.rs#L734) | Mapping/import interaction review |
| AT-31 | [wrapping_and_unwrapping_keep_child_identity_and_removal_never_removes_resources](../libs/window-manager-core/src/tests.rs#L193) | Keyboard unwrap/remove review |
| AT-32 | [all_three_group_preservation_modes_have_distinct_geometry](../libs/window-manager-core/src/tests.rs#L1770) | Content-fit interaction review |
| AT-33 | [game_in_unrelated_slot_receives_zero_operations_and_stale_protection_is_detected](../libs/window-manager-core/src/tests.rs#L1321) | Real full-screen game and neighbor apps |
| AT-34 | [group_and_visit_protections_have_independent_lifetimes_and_attention_is_scoped](../libs/window-manager-core/src/tests.rs#L1600) | Production AI provider deferred |
| AT-35 | [navigation_resolves_exact_shared_occurrence_and_keeps_other_roots_out_of_scope](../libs/window-manager-core/src/tests.rs#L2554) | Production attention provider deferred |
| AT-36 | [group_and_visit_protections_have_independent_lifetimes_and_attention_is_scoped](../libs/window-manager-core/src/tests.rs#L1600) | Real maintained-visible monitoring app |
| AT-37 | [claim_elsewhere_and_stale_binding_are_rejected](../libs/window-manager-core/src/tests.rs#L642) | Real shared-resource transfer review |
| AT-38 | [output_provider_expiry_disconnect_and_capability_scope_are_enforced](../libs/window-manager-core/src/tests.rs#L1549) | Actual output integration deferred |
| AT-39 | [control_fallback_never_uses_public_intersections_or_changes_saved_topology](../libs/window-manager-core/src/tests.rs#L1653) | Physical monitor removal |
| AT-40 | [explicit_topology_fallback_has_independent_preferences_and_reconnect_restores_original](../libs/window-manager-core/src/tests.rs#L1915) | Physical docking/reconnect |
| AT-41 | [provider_disconnect_received_during_application_prevents_scope_commit](../apps/window-manager/src/service.rs#L150) | Actual capture provider deferred |
| AT-42 | [copied_private_view_retains_shared_public_content_warning](../libs/window-manager-core/src/tests.rs#L2206) | Shared-content warning usability |
| AT-43 | [borrowed_expansion_requires_exact_authority_and_restores_empty_slots](../libs/window-manager-core/src/tests.rs#L396) | Real protected-neighbor expansion |
| AT-44 | [stalled_native_submission_has_bounded_failure_and_preserves_other_scopes](../apps/window-manager/src/service.rs#L233) | Foreign hung-app/hotkey responsiveness |
| AT-45 | [stalled_native_submission_has_bounded_failure_and_preserves_other_scopes](../apps/window-manager/src/service.rs#L233) | Real app repeated size rejection |
| AT-46 | [move_only_keeps_client_size_and_destroyed_lifetime_is_rejected](../platforms/windows-window-manager/src/native.rs#L955) | Forced same-value HWND reuse and inventory-to-bind race |
| AT-47 | [separate_helper_recovers_hidden_window_after_parent_process_death](../apps/window-manager/tests/recovery.rs#L35) | Chrome/Edge direct journal profile checked; independent-helper browser review remains |
| AT-48 | [manual_minimization_is_not_undone_by_force_restore](../libs/window-manager-core/src/tests.rs#L1628) | Real user minimization |
| AT-49 | [settled_scope_rechecks_output_lifetime_modal_and_style_without_rejecting_own_effects](../libs/window-manager-core/src/tests.rs#L1994) | Actual owned-modal creation/IME |
| AT-50 | [claim_elsewhere_and_stale_binding_are_rejected](../libs/window-manager-core/src/tests.rs#L642) | Ambiguous real-window restart UI |
| AT-51 | [native_undo_is_scoped_and_revalidates_new_owners_and_manual_changes](../libs/window-manager-core/src/tests.rs#L522) | Real late undo/claim interactions |
| AT-52 | [duplicate_configuration_maps_are_not_normalized_or_saved_over_last_good_state](../libs/window-manager-core/src/tests.rs#L2532) | Safe-mode recovery usability |
| AT-53 | [named_shortcuts_use_ids_and_fixed_roles_across_rename_and_workspace_selection](../libs/window-manager-core/src/tests.rs#L2088) | Native collision/release fixture below; Korean IME/layouts remain |
| AT-54 | [settled_scope_rechecks_output_lifetime_modal_and_style_without_rejecting_own_effects](../libs/window-manager-core/src/tests.rs#L1994) | Actual denied activation result |
| AT-55 | [tab_identity_and_scope_are_validated_before_layout](../libs/window-manager-core/src/tests.rs#L1302) | Local command lifecycle review |
| AT-56 | [game_in_unrelated_slot_receives_zero_operations_and_stale_protection_is_detected](../libs/window-manager-core/src/tests.rs#L1321) | Real game CPU/GPU/frame and encoder impact |

## VG evidence

| Gate | Evidence available | Remaining review |
| --- | --- | --- |
| VG-01 domain boundaries | Separate core/native/app crates; multi-root/Workspace/claim/structure fixtures | Scenario-level product review |
| VG-02 state scope | Independent preference contexts, Visit overrides, save/promotion, local membership and explicit capture tests | Native manual edit provenance under IME/modals |
| VG-03 strategies | Split/grid/free/flow/semantic/responsive, preservation/alignment, pins, failed-child isolation, explicit resize exceptions | Per-app native constraints |
| VG-04 interactions | Discoverable commands, keyboard counterparts for structure edits, exact previews, inventory/recovery, ancestor paths; static Korean GUI inspection | Full first-run, keyboard-only, drag/drop and accessibility walk-through |
| VG-05 formulas/packages | Typed bounded formulas, acyclic parameters, sort/variants, read-only simulation, redaction/mapping/independent import | Interactive authoring/package review; P2 links excluded |
| VG-06 authority | Shared planner/revalidation, strict local pipe capabilities, opaque single-use tokens, bounded replay, no executable layout dependency | Unsupported origin/privilege scenario matrix; remote transport excluded |
| VG-07 native execution | One final mutation per resource, same-thread batch, async foreign-thread submissions, separate activation, bounded settlement and domain commit | Larger foreign-app mixes; owned native Defer failure verified below |
| VG-08 lifecycle | Manual binding; hide/show-state compatibility opt-ins; readiness unknown; own-window graceful reveal and independent crash recovery | Real minimized/modal/elevated/app-version profiles and exact HWND reuse |
| VG-09 measurements | [Reference fixtures and timing distributions](window-manager-performance.md); redacted bounded last-result log; no thumbnail/capture cache | Idle CPU/memory distributions, game/broadcast load, foreign-app/mixed-DPI timings |
| VG-10 API lifecycle | Strict fields/keys/scope/revision, replay collision and missed-event resync, malformed configuration/session tests | Exhaustive scenario exercise of every origin/error/capability; provider integrations excluded |

## Reproduction

```powershell
cargo test --workspace --all-targets --all-features
cargo clippy -p window-manager-core -p windows-window-manager -p window-manager --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo build -p window-manager
```

Latest full workspace run: 128 passing tests, 0 failures, 4 intentionally ignored interactive/child fixtures. The recovery child fixture is exercised indirectly by its parent test. New window-manager crates pass strict Clippy; full-workspace strict Clippy still has documented pre-existing warnings in unrelated native adapters/applications. Stable rustfmt reports unsupported nightly-only configuration options but completes formatting.

To close application gates, record application/version, privilege, monitor identity/DPI, show-state and visibility profile, each AT/VG result, counts/timing distributions, and whether rendering/output evidence is known. Do not mark a provider or application supported solely from its executable name, an API success return or observed geometry. Keep personal titles/content out of published logs.

## Native shortcut lifecycle follow-up

The owned `Win+Ctrl+F14..F22` collision fixture passes without injecting input or replacing existing registrations. It verifies one error for duplicate requested IDs, rejects out-of-range IDs, preserves the original registration after a collision, and releases successfully registered keys on stream drop across three restarts. Observation hooks and their message-loop thread now share the same explicit shutdown lifetime; channel saturation cannot block shutdown. This exercises the production registration path with test virtual keys. Actual digit shortcuts under Korean IME/layouts remain an interactive gate.

## Batch failure follow-up

`native_defer_failure_leaves_prior_window_unchanged_without_end` creates two owned STATIC windows, queues the first move, destroys the second after lifetime validation, and receives a real Win32 Defer failure. End is not called; the first frame remains unchanged and the destroyed lifetime is rejected. Separate injected Begin/Defer/End return-path tests verify immediate stop, retention of each returned handle, one End at most, and no individual replay. An End failure remains an uncertain native outcome; these tests do not promise atomic rollback after End.

## Actual application follow-up

[Published browser measurements](window-manager-performance.md#isolated-actual-browser-applications) cover one isolated normal app window each for Chrome 154.0.8037.98 and Edge 155.0.4283.45 at 96 DPI. Move-only, resize and hide/show passed 30 measured requests per mode; explicit minimize/restore and direct journal recovery also passed. Process termination invalidated bindings and unique temporary profiles were removed. Rendering/output remains unknown. Actual 96↔192 DPI requests failed on both applications; a compatibility opt-in now defaults off and prevents such a plan before native effects. Older configurations inherit the off value. Pure planning explicitly declares simulated compatibility when exercising mixed-DPI fixtures. Physical topology loss, elevated/modals/IME, browser rendering/capture, simultaneous application mixes and exact HWND reuse remain open.
