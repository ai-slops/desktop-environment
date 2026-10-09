# Window manager reference measurements

Measured 2026-10-10 on Windows 10.0.26200.0, ordinary desktop user token, release build, AMD64 Family 25 Model 117 Stepping 2, AuthenticAMD. Four connected monitors include 96 and 192 DPI; native fixtures deliberately remain on one same-DPI monitor. No elevated target or existing user application is controlled.

These are reference fixtures, not an application compatibility or rendering-liveness certification. Native geometry settlement includes submission and repeated observation/message pumping. Render readiness remains unknown. Quantiles use nearest rank; 20 samples make the native P99 equal to the worst sample. Results depend on desktop load and are not fixed product guarantees.

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

## Reproduction and product logs

```powershell
cargo run --release -p window-manager-core --example benchmark -- target/window-manager-planning-benchmark.json
cargo run --release -p windows-window-manager --example benchmark -- target/window-manager-native-benchmark.json
```

The worker retains one redacted `.last-transition.json` beside its configuration/journal. It records opaque request/window IDs, exact scope, revisions, mode, impact, fallback diagnostics, batch/individual counts, per-window result and microsecond planning/submission/native API/geometry-settlement/focus times. Planning is measured against an immutable snapshot. Submission includes journal preparation; native API time isolates submit_many. Settlement elapsed starts before journal preparation and includes polling; geometry_settlement is null when any submitted operation failed, timed out or was superseded. Authority is rechecked before commit; observed geometry alone is not a committed authority result. No native handles, titles or screenshots are logged.

Unmeasured release gates: real browser/editor/terminal/game/broadcaster mixes, foreign/elevated targets, mixed-DPI native transfer distributions, physical disconnect/reconnect, CPU/GPU load and broadcast encoder impact, idle resource distributions, and actual render readiness. Existing bounded fixture tests cover stalls, rapid supersession, output evidence expiry/disconnect and crash reveal, without claiming those application matrix results.
