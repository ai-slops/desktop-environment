# Window manager reference measurements

Measured 2026-10-10 on Windows 10.0.26200.0, ordinary desktop user token, release build, AMD64 Family 25 Model 117 Stepping 2, AuthenticAMD. Four connected monitors include 96 and 192 DPI; native fixtures deliberately remain on one same-DPI monitor. No elevated target or existing user application is controlled.

The planning and STATIC measurements are reference fixtures. Limited actual-browser evidence is recorded separately below; rendering liveness remains uncertified. Native geometry settlement includes submission and repeated observation/message pumping. Render readiness remains unknown. Quantiles use nearest rank; 20 samples make the native P99 equal to the worst sample. Results depend on desktop load and are not fixed product guarantees.

## Pure planning

Nested groups with 4/8/16 synthetic window observations; 50 warm-up iterations and 500 measured samples per case. Planning excludes discovery, serialization, journal persistence and native execution. Every case asserts unique final mutations; move-only asserts zero resizes; authored state remains unchanged. Formula budgets: 4096 bytes, 512 tokens, depth 32; membership arbitration at most four passes. Disconnected cases expect TargetMissing on all 500 samples.

| Windows | Mode | Median ms | P95 ms | P99 ms | Worst ms | Failures |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 4 | move_only | 0.0519 | 0.0751 | 0.0900 | 0.1353 | 0 |
| 4 | partial_resize | 0.0440 | 0.0867 | 0.1305 | 0.1940 | 0 |
| 4 | widespread_resize | 0.0411 | 0.0600 | 0.0842 | 0.1169 | 0 |
| 4 | semantic_switch | 0.0246 | 0.0477 | 0.0782 | 0.1363 | 0 |
| 4 | responsive_fold | 0.0248 | 0.0401 | 0.0556 | 0.0581 | 0 |
| 4 | mixed_dpi_logical | 0.0431 | 0.0616 | 0.1015 | 0.1816 | 0 |
| 4 | disconnected | 0.0187 | 0.0263 | 0.0343 | 0.0467 | expected TargetMissing ×500 |
| 8 | move_only | 0.0683 | 0.1163 | 0.2035 | 0.2642 | 0 |
| 8 | partial_resize | 0.0618 | 0.1083 | 0.1517 | 0.2686 | 0 |
| 8 | widespread_resize | 0.0558 | 0.0977 | 0.1211 | 0.2704 | 0 |
| 8 | semantic_switch | 0.0308 | 0.0539 | 0.0940 | 0.1472 | 0 |
| 8 | responsive_fold | 0.0353 | 0.0449 | 0.0806 | 0.1890 | 0 |
| 8 | mixed_dpi_logical | 0.0551 | 0.0884 | 0.1315 | 0.2168 | 0 |
| 8 | disconnected | 0.0209 | 0.0395 | 0.0688 | 0.1312 | expected TargetMissing ×500 |
| 16 | move_only | 0.1077 | 0.1667 | 0.2010 | 0.2365 | 0 |
| 16 | partial_resize | 0.0952 | 0.1721 | 0.1961 | 0.2497 | 0 |
| 16 | widespread_resize | 0.0816 | 0.1238 | 0.1657 | 0.1923 | 0 |
| 16 | semantic_switch | 0.0472 | 0.0583 | 0.0935 | 0.1107 | 0 |
| 16 | responsive_fold | 0.0603 | 0.0980 | 0.1271 | 0.2460 | 0 |
| 16 | mixed_dpi_logical | 0.0854 | 0.1381 | 0.1655 | 0.1812 | 0 |
| 16 | disconnected | 0.0284 | 0.0328 | 0.0489 | 0.0553 | expected TargetMissing ×500 |

## Native adapter

Disposable, same-thread/same-parent Win32 STATIC windows created and destroyed by the example. Five warm-ups and 20 measured samples per case. Submission measures submit_many; geometry settlement measures time from submission start through final observed frame/visibility. Move-only additionally asserts unchanged client size/DPI. Each request submits one final rectangle per changed window; no resize animation. The fixture bypasses the product journal and worker arbitration and cannot establish foreign-thread application performance. All cases had zero submission/settlement failures and zero observed foreground changes.

| Windows | Mode | Submission median / P95 / P99 / worst ms | Settlement median / P95 / P99 / worst ms | Batched / individual operations |
| --- | --- | --- | --- | --- |
| 4 | move_only | 1.631 / 3.002 / 3.038 / 3.038 | 5.638 / 7.833 / 7.964 / 7.964 | 80 / 0 |
| 4 | partial_resize | 2.364 / 3.601 / 3.687 / 3.687 | 4.840 / 6.147 / 6.608 / 6.608 | 40 / 0 |
| 4 | widespread_resize | 3.486 / 5.900 / 9.556 / 9.556 | 7.546 / 12.101 / 14.100 / 14.100 | 80 / 0 |
| 4 | hide_show | 4.221 / 11.124 / 12.345 / 12.345 | 11.174 / 19.847 / 21.602 / 21.602 | 80 / 0 |
| 8 | move_only | 4.279 / 6.245 / 6.935 / 6.935 | 12.894 / 16.475 / 17.687 / 17.687 | 160 / 0 |
| 8 | partial_resize | 5.055 / 6.779 / 8.251 / 8.251 | 10.278 / 12.598 / 13.215 / 13.215 | 80 / 0 |
| 8 | widespread_resize | 6.730 / 8.151 / 9.322 / 9.322 | 13.923 / 18.086 / 18.105 / 18.105 | 160 / 0 |
| 8 | hide_show | 6.507 / 18.749 / 20.269 / 20.269 | 15.702 / 32.064 / 33.078 / 33.078 | 160 / 0 |
| 16 | move_only | 4.716 / 8.420 / 9.999 / 9.999 | 22.845 / 27.581 / 27.829 / 27.829 | 320 / 0 |
| 16 | partial_resize | 9.866 / 13.976 / 14.064 / 14.064 | 20.972 / 25.243 / 28.749 / 28.749 | 160 / 0 |
| 16 | widespread_resize | 13.262 / 20.341 / 22.787 / 22.787 | 30.287 / 40.778 / 42.027 / 42.027 | 320 / 0 |
| 16 | hide_show | 14.424 / 41.049 / 52.650 / 52.650 | 38.838 / 71.541 / 84.813 / 84.813 | 320 / 0 |

## Isolated actual browser applications

Measured 2026-10-10, release build, ordinary user, Chrome 154.0.8037.98 and Edge 155.0.4283.45 (installed executable FileVersion). Each application used one newly launched app window with a unique temporary profile and constant data page. Processes were created suspended, assigned to an owned kill-on-close Job, then resumed. Existing profiles/windows were not bound. Profiles were removed and terminated bindings were rejected. Reports contain no native handle, PID, profile path or personal title/content: [Chrome raw report](measurements/window-manager-chrome-20261010.json), [Edge raw report](measurements/window-manager-edge-20261010.json).

Same-monitor 96 DPI cases used five warm-ups and 30 measured requests each; one asynchronous individual geometry/visibility request per sample. Client size and DPI were checked in addition to frame/visibility. All six cases had zero failures, zero client/DPI mismatches and zero observed foreground changes. Rendering readiness is unknown; product planner/journal overhead is excluded from these timing cases.

| Application | Mode | Submission median / P95 / P99 / worst ms | Settlement median / P95 / P99 / worst ms |
| --- | --- | --- | --- |
| Chrome 154.0.8037.98 | move_only | 0.028 / 0.477 / 0.559 / 0.559 | 4.950 / 6.272 / 6.275 / 6.275 |
| Chrome 154.0.8037.98 | resize | 0.120 / 0.456 / 1.313 / 1.313 | 4.649 / 8.534 / 11.136 / 11.136 |
| Chrome 154.0.8037.98 | hide_show | 0.261 / 0.656 / 0.824 / 0.824 | 9.213 / 13.838 / 16.909 / 16.909 |
| Edge 155.0.4283.45 | move_only | 0.146 / 0.548 / 1.911 / 1.911 | 5.640 / 6.846 / 183.608 / 183.608 |
| Edge 155.0.4283.45 | resize | 0.111 / 0.762 / 1.327 / 1.327 | 4.675 / 9.093 / 11.896 / 11.896 |
| Edge 155.0.4283.45 | hide_show | 0.216 / 0.606 / 1.521 / 1.521 | 8.635 / 15.921 / 16.072 / 16.072 |

Both applications also passed explicit minimize/normal restore, recovery preserving a minimized window, recovery revealing a hidden window without changing frame/client, and emptied recovery journals. These are direct adapter/journal tests, not evidence of crash-helper behavior against every browser mode.

**Restricted mixed-DPI profile:** both applications failed all 20 measured 96↔192 DPI transitions within the 900 ms settlement bound. A requested 1600×1200 frame settled at 3200×2400 on 192 DPI; a requested 800×600 frame settled at 400×300 on return to 96 DPI. Client dimensions also differed, and each run observed two foreground changes. The probe sent one geometry request and did not replay a correction. Timeout quantiles in raw reports are failure bounds, not successful transition latency. This profile is unsupported. `capabilities.allow_dpi_transfer` now defaults to false, including older v1 configurations; normal same-DPI operations remain available. Explicit opt-in is for independently verified application profiles and does not certify rendering or capture safety. Undo also rechecks this policy.

Reproduce only with a disposable, isolated profile through the supplied harness:

```powershell
cargo run --release -p windows-window-manager --example application_probe -- "C:\Program Files\Google\Chrome\Application\chrome.exe" Chrome 154.0.8037.98 target/window-manager-chrome-probe.json
cargo run --release -p windows-window-manager --example application_probe -- "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" Edge 155.0.4283.45 target/window-manager-edge-probe.json
```

The application/version arguments label the report; verify the installed executable FileVersion before each run. Only Chrome/Edge-style app windows are accepted; no window fallback by title or process name is used.

## Empty local-session idle cost

Measured after the final release build on 2026-10-10. The harness creates a new empty configuration and an ordinary-user read-only `--session` process, keeps stdin open without issuing commands, drains output, waits five seconds, then samples CPU/working set/private bytes every second for 30 intervals. The exact executable SHA-256 is in the [raw report](measurements/window-manager-idle-session-20261010.json). EOF shut the session down successfully; its fixture directory was removed. Only the owned process is sampled or terminated.

| Metric | Median | P95 | P99 / worst |
| --- | ---: | ---: | ---: |
| CPU, percent of one logical core | 0.0000% | 1.5469% | 1.5491% |
| Working set | 10.6250 MiB | 10.6289 MiB | 10.6289 MiB |
| Private bytes | 2.0078 MiB | 2.0078 MiB | 2.0078 MiB |

CPU is calculated from process-time deltas divided by actual elapsed wall time, without division by the machine's core count. Process-time quantization at one-second sampling means a zero median does not establish zero work. This session has no GUI, bound windows, providers, registered shortcuts or output capture. It exercises the actor's idle topology/event loop; it does not establish GUI/GPU cost, idle scaling with managed windows or game/broadcast impact. Native observation hooks are active and terminate with the session.

```powershell
cargo build --release -p window-manager
pwsh -File tools/measure-window-manager-idle.ps1
```

Wait for the build to complete before measuring; a running Windows executable prevents its build output from being replaced. The PowerShell 7 harness uses a unique workspace fixture directory and validates its absolute path before cleanup.

## Reproduction and product logs

```powershell
cargo run --release -p window-manager-core --example benchmark -- target/window-manager-planning-benchmark.json
cargo run --release -p windows-window-manager --example benchmark -- target/window-manager-native-benchmark.json
```

The worker retains one redacted `.last-transition.json` beside its configuration/journal. It records opaque request/window IDs, exact scope, revisions, mode, impact, fallback diagnostics, batch/individual counts, per-window result and microsecond planning/submission/native API/geometry-settlement/focus times. Planning is measured against an immutable snapshot. Submission includes journal preparation; native API time isolates submit_many. Settlement elapsed starts before journal preparation and includes polling; geometry_settlement is null when any submitted operation failed, timed out or was superseded. Authority is rechecked before commit; observed geometry alone is not a committed authority result. No native handles, titles or screenshots are logged.

Unmeasured release gates: larger browser/editor/terminal/game/broadcaster mixes, elevated targets, verified mixed-DPI native transfers, physical disconnect/reconnect, CPU/GPU load and broadcast encoder impact, GUI/bound-window idle resource distributions, and actual render readiness. Existing bounded fixture tests cover stalls, rapid supersession, output evidence expiry/disconnect and crash reveal, without claiming those application matrix results.
