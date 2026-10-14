import { test, expect } from "@playwright/test";
import type { NativeSnapshot, NativeCommand } from "../src/native-protocol.ts";

function snapshot(): NativeSnapshot {
  const stopped = { phase: "stopped", error: null, logs: [] };
  return {
    revision: 0,
    configRoot: "isolated-test-settings",
    errors: [],
    storageError: null,
    cableStorageError: null,
    settings: {
      last_used: {
        audio: {
          enabled: true,
          source: "default",
          source_label: "",
          target: "speakers",
          target_label: "Speakers",
        },
        display: {
          enabled: false,
          display: "display",
          display_label: "Display",
          fullscreen: false,
          fps: 30,
          timeout_ms: 100,
        },
        additional_displays: [],
      },
      presets: [],
    },
    cableSettings: {
      version: 1,
      cable: null,
      output_id: "speakers",
      gain_db: 0,
      muted: false,
      presets: [],
    },
    endpoints: [
      {
        id: "headphones",
        name: "Headphones",
        flow: "playback",
        isDefault: true,
      },
      { id: "speakers", name: "Speakers", flow: "playback", isDefault: false },
    ],
    cables: [],
    displays: [{ id: "display", name: "Display", width: 1920, height: 1080 }],
    jobs: {
      audio: { ...stopped },
      cable: { ...stopped },
      mirrors: [{ ...stopped }],
    },
    traces: [],
    wm: {
      connected: true,
      error: null,
      inventory: [],
      lastResult: null,
      configuration: {
        version: 1,
        revision: 0,
        workspaces: {
          w: { id: "w", name: "Workspace", remembered_view: null },
        },
        windows: {},
        views: {
          v: {
            id: "v",
            workspace: "w",
            name: "Saved view",
            roots: {
              main: {
                type: "group",
                id: "group",
                name: "Main",
                strategy: "horizontal",
                gap: "8",
                children: [],
              },
            },
          },
        },
        slots: {
          s: {
            id: "s",
            name: "Screen",
            display: "monitor",
            region: [0, 0, 1, 1],
            fallback_displays: [],
          },
        },
        compositions: {},
        collections: {},
        shortcuts: [],
      },
      snapshot: {
        revision: 0,
        paused: false,
        windows: {},
        displays: {},
        presentations: {},
      },
      monitors: { epoch: "epoch", monitors: [] },
    },
  };
}

test("native choices, worker failures and detached panels use the same service state", async ({
  context,
  page,
}) => {
  const state = snapshot();
  const commands: NativeCommand[] = [];
  await context.route("**/api/**", async (route) => {
    if (route.request().url().endsWith("bootstrap"))
      return route.fulfill({ json: { token: "test-token", version: 1 } });
    expect(route.request().headers()["x-workspace-token"]).toBe("test-token");
    const { id, command } = route.request().postDataJSON();
    commands.push(command);
    if (command.kind === "audio.select")
      state.settings.last_used.audio[command.role as "source" | "target"] =
        command.id;
    if (command.kind === "audio.start")
      state.jobs.audio = {
        phase: "failed",
        error: "Initialize failed: AUDCLNT_E_DEVICE_IN_USE (0x8889000A)",
        logs: ["Full cause from native child"],
      };
    state.revision++;
    await route.fulfill({
      json: { id, ok: true, data: { state, result: null } },
    });
  });
  await page.goto("/?mode=native");
  const panel = page.getByRole("region", { name: "Desktop Control 패널" });
  await expect(
    panel.getByRole("button", { name: "오디오 시작", exact: true }),
  ).toBeEnabled();
  expect(commands.every((c) => c.kind === "snapshot")).toBeTruthy();
  await panel
    .getByRole("group", { name: /^대상 출력/ })
    .getByRole("radio", { name: /^Headphones/ })
    .click();
  await expect(
    panel
      .getByRole("group", { name: /^대상 출력/ })
      .getByRole("radio", { name: /^Headphones/ }),
  ).toBeChecked();
  await expect(
    panel.getByRole("button", { name: "오디오 시작", exact: true }),
  ).toBeDisabled();
  await panel
    .getByRole("group", { name: /^대상 출력/ })
    .getByRole("radio", { name: /^Speakers/ })
    .click();
  await expect(
    panel
      .getByRole("group", { name: /^대상 출력/ })
      .getByRole("radio", { name: /^Speakers/ }),
  ).toBeChecked();
  await panel.getByRole("button", { name: "오디오 시작", exact: true }).click();
  await expect(panel.getByText(/Initialize failed/)).toBeVisible();
  const popupEvent = page.waitForEvent("popup");
  await panel.getByRole("button", { name: "새 창으로 이동" }).click();
  const popup = await popupEvent;
  const detached = popup.getByRole("region", { name: "Desktop Control 패널" });
  await expect(detached.getByText(/Initialize failed/)).toBeVisible();
  await detached.getByRole("button", { name: "주 창으로 합치기" }).click();
  await expect(panel.getByText(/Initialize failed/)).toBeVisible();
  expect(commands.filter((c) => c.kind === "audio.start")).toHaveLength(1);
});

test("native apply requires a saved configuration and a fresh preview", async ({
  context,
  page,
}) => {
  const state = snapshot();
  const commands: NativeCommand[] = [];
  await context.route("**/api/**", async (route) => {
    if (route.request().url().endsWith("bootstrap"))
      return route.fulfill({ json: { token: "test-token", version: 1 } });
    const { id, command } = route.request().postDataJSON();
    commands.push(command);
    let result: unknown = null;
    if (command.kind === "windows.session") {
      if (command.command.kind === "configure") {
        state.wm.configuration = command.command.configuration;
        state.wm.configuration!.revision++;
      }
      if (command.command.kind === "preview")
        result = {
          plan: "opaque-native-plan",
          request: "transition",
          revision: state.wm.configuration!.revision,
          topology_revision: 0,
          desired: {},
          blocked: {},
          diagnostics: [],
          impact: { moved: 0, resized: 0, hidden: 0, shown: 0 },
        };
      if (command.command.kind === "apply") result = { status: "complete" };
    }
    state.revision++;
    await route.fulfill({ json: { id, ok: true, data: { state, result } } });
  });
  await page.goto("/?mode=native");
  const panel = page.getByRole("region", { name: "Window Manager 패널" });
  const apply = panel.getByRole("button", {
    name: "실제 창에 적용",
    exact: true,
  });
  await expect(apply).toBeDisabled();
  await panel
    .getByRole("button", { name: "배치 미리보기", exact: true })
    .click();
  await expect(apply).toBeEnabled();
  await panel.getByText("배치·그룹·화면 영역 편집", { exact: true }).click();
  await panel.getByLabel("배치 이름", { exact: true }).fill("Edited view");
  await expect(apply).toBeDisabled();
  await panel.getByRole("button", { name: "배치 편집 저장" }).click();
  await expect(
    panel.getByRole("button", { name: "배치 편집 저장" }),
  ).toBeDisabled();
  await expect(apply).toBeDisabled();
  await panel
    .getByRole("button", { name: "배치 미리보기", exact: true })
    .click();
  await apply.click();
  await expect(
    panel.getByText("적용 결과: complete", { exact: true }),
  ).toBeVisible();
  expect(
    commands.filter(
      (c) =>
        c.kind === "windows.session" &&
        (c.command as NativeCommand).kind === "apply",
    ),
  ).toHaveLength(1);
});

test("missing native service disables controls without falling back to mock devices", async ({
  context,
  page,
}) => {
  const state = snapshot();
  let disconnected = false;
  await context.route("**/api/**", async (route) => {
    if (disconnected)
      return route.fulfill({
        status: 503,
        json: { error: { message: "Native host exited" } },
      });
    if (route.request().url().endsWith("bootstrap"))
      return route.fulfill({ json: { token: "test-token", version: 1 } });
    const { id } = route.request().postDataJSON();
    state.revision++;
    return route.fulfill({ json: { id, ok: true, data: { state } } });
  });
  await page.goto("/?mode=native");
  const start = page
    .getByRole("region", { name: "Desktop Control 패널" })
    .getByRole("button", { name: "오디오 시작", exact: true });
  await expect(start).toBeEnabled();
  disconnected = true;
  await expect(start).toBeDisabled();
  await expect(page.getByText("테스트 모드", { exact: true })).toHaveCount(0);
  await expect(
    page.getByText("Native host exited", { exact: false }).first(),
  ).toBeVisible();
});

test("native edits preserve window resources, reorder ratios and commit cable volume", async ({
  context,
  page,
}) => {
  const state = snapshot();
  const commands: NativeCommand[] = [];
  const config = state.wm.configuration!;
  for (const id of ["one", "two"])
    config.windows[id] = {
      id,
      alias: id,
      tags: [],
      allow_hide: false,
      capabilities: {
        allow_move: true,
        allow_resize: true,
        allow_show_state: false,
        allow_dpi_transfer: false,
      },
      protection: {
        geometry_lock: false,
        maintain_visible: false,
        keep_monitor: false,
        prohibit_focus: false,
      },
    };
  config.views.v.roots.main.children = ["one", "two"].map((id) => ({
    type: "placement",
    id: `placement-${id}`,
    window: id,
    role: id,
  }));
  config.views.v.roots.main.ratios = [2, 1];
  await context.route("**/api/**", async (route) => {
    if (route.request().url().endsWith("bootstrap"))
      return route.fulfill({ json: { token: "test-token", version: 1 } });
    const { id, command } = route.request().postDataJSON();
    commands.push(command);
    if (
      command.kind === "windows.session" &&
      command.command.kind === "configure"
    ) {
      state.wm.configuration = command.command.configuration;
      state.wm.configuration!.revision++;
    }
    if (command.kind === "cable.volume") {
      state.cableSettings.gain_db = 20 * Math.log10(command.value / 100);
      state.cableSettings.muted = false;
    }
    if (command.kind === "mirror.settings")
      state.settings.last_used.display = command.settings;
    state.revision++;
    await route.fulfill({
      json: { id, ok: true, data: { state, result: null } },
    });
  });
  await page.goto("/?mode=native");
  const panel = page.getByRole("region", { name: "Window Manager 패널" });
  await panel.getByText("배치·그룹·화면 영역 편집", { exact: true }).click();
  await panel.getByRole("button", { name: "창 순서 위로" }).nth(1).click();
  await panel.getByRole("button", { name: "배치 편집 저장" }).click();
  await expect(
    panel.getByRole("button", { name: "배치 편집 저장" }),
  ).toBeDisabled();
  expect(
    state.wm.configuration!.views.v.roots.main.children!.map(
      (node) => node.window,
    ),
  ).toEqual(["two", "one"]);
  expect(state.wm.configuration!.views.v.roots.main.ratios).toEqual([1, 2]);
  await panel.getByRole("button", { name: "배치에서 빼기" }).first().click();
  await panel.getByRole("button", { name: "배치 편집 저장" }).click();
  await expect(
    panel.getByRole("button", { name: "배치 편집 저장" }),
  ).toBeDisabled();
  expect(Object.keys(state.wm.configuration!.windows)).toHaveLength(2);
  expect(state.wm.configuration!.views.v.roots.main.children).toHaveLength(1);
  await page
    .getByRole("button", { name: "Audio Cable 열기", exact: true })
    .click();
  const cable = page.getByRole("region", { name: "Audio Cable 패널" });
  await cable.getByRole("slider", { name: /모니터링 볼륨/ }).press("ArrowLeft");
  await expect(cable.locator("output")).toHaveText("99.99%");
  await expect(
    cable.getByRole("button", { name: "설정 저장", exact: true }),
  ).toBeEnabled();
  expect(commands.filter((command) => command.kind === "cable.volume")).toEqual(
    [{ kind: "cable.volume", value: 99.99 }],
  );
  await page
    .getByRole("button", { name: "Desktop Control 열기", exact: true })
    .click();
  const audio = page.getByRole("region", { name: "Desktop Control 패널" });
  await audio.getByLabel("미러링 1 FPS", { exact: true }).selectOption("60");
  await expect(audio.getByLabel("미러링 1 FPS", { exact: true })).toHaveValue(
    "60",
  );
  expect(
    commands.some((command) => command.kind === "mirror.settings"),
  ).toBeTruthy();
});
