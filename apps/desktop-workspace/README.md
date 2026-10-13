# Desktop Workspace

An experimental web workspace for Desktop Control, Audio Cable, and Window Manager.
It is a **fixture-only prototype**, not a replacement for the native tools yet.
Buttons simulate operations against virtual devices and windows; they do not run
audio, move Windows windows, launch processes, or read/write production presets.

## Run

Node 22.12 or newer (Node 24 recommended) and pnpm are required.

```powershell
cd apps/desktop-workspace
pnpm install --frozen-lockfile
pnpm dev
```

Open <http://127.0.0.1:4177> in Chrome. Allow popups on that address for separate
panel windows. The development server binds to loopback only.

Drag a tab onto another group's tabs to combine them, or onto a content edge to
split. Resize by dragging the separator. **새 창으로 이동** detaches one panel;
**주 창으로 합치기** returns its group. The activity bar reopens closed panels.
Closing a panel does not stop its fixture job. **기본 배치** changes panel layout
only, preserving the current fixture settings and jobs.

The last complete grid is saved to browser local storage under
`desktop-workspace.layout.v1`. While popouts are open, the previously saved grid is
kept; reloading restores that grid instead of automatically spawning new windows.
Mock device/job state intentionally resets on a full page reload. No production
settings are stored in browser local storage.

## Verify

```powershell
pnpm build
pnpm format:check
pnpm test
pnpm test:e2e
```

On Windows, browser tests use the installed Chrome with an isolated test profile.
On Linux/macOS, install the Playwright Chromium browser using
`pnpm exec playwright install chromium`. Set `PW_CHANNEL` to use another installed
browser channel. Tests never use the native audio/window adapters or a VM.

Unit tests cover startup cancellation, structured errors, feedback prevention
(including default endpoint aliases), stale preview rejection, and volume bounds.
Browser tests exercise radio/checkbox semantics, a reproduced exclusive-device
failure, preview/apply sequencing, tab drag/drop, popout/redocking with shared
state, corrupted layout recovery, and a screenshot in `test-results/workspace.png`.

## Code boundaries

- `src/workspace.ts`: typed backend contract and deterministic fixture controller.
- `src/main.tsx`: docking shell and the three control panels plus diagnostics.
- `src/style.css`: panel-size-responsive styles and explicit selection states.
- `src/workspace.test.ts`: device-free controller tests.
- `tests/workspace.spec.ts`: browser interaction regressions.

See [the integration design](../../docs/desktop-workspace.md) for the real backend
work still needed and the acceptance criteria before replacing a native GUI.
