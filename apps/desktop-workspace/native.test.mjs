import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { resolve } from "node:path";
import { NativeBridge } from "./server.mjs";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { once } from "node:events";

test("real host starts idle, serves devices, and configures an isolated window session", async () => {
  const root = await mkdtemp(resolve("../../target/workspace-native-test-"));
  const bridge = new NativeBridge(
    resolve("../../target/desktop-control/debug/desktop-workspace-host.exe"),
    root,
  );
  let sequence = 0;
  const request = async (command) =>
    bridge.request(command, `integration-${++sequence}`);
  try {
    const first = await request({ kind: "snapshot" });
    assert.equal(first.ok, true, JSON.stringify(first.error));
    const state = first.data.state;
    assert.equal(state.jobs.audio.phase, "stopped");
    assert.equal(state.jobs.cable.phase, "stopped");
    assert.equal(state.wm.connected, false);
    assert.ok(Array.isArray(state.endpoints));
    assert.ok(Array.isArray(state.displays));
    assert.deepEqual(await readdir(root), []);
    const connected = await request({ kind: "windows.connect" });
    assert.equal(connected.ok, true, JSON.stringify(connected.error));
    const config = connected.data.state.wm.configuration;
    assert.ok(connected.data.state.wm.inventory.length >= 0);
    const created = await request({
      kind: "windows.view.add",
      name: "Integration test view",
    });
    assert.equal(created.ok, true, JSON.stringify(created.error));
    assert.equal(
      created.data.state.wm.configuration.revision,
      config.revision + 1,
    );
    assert.ok(
      created.data.state.wm.configuration.views[created.data.result.view],
    );
    const stale = await request({
      kind: "windows.session",
      command: {
        kind: "configure",
        expected_revision: config.revision,
        configuration: config,
      },
    });
    assert.equal(stale.ok, false);
    assert.match(stale.error.message, /revision/i);
    const saved = JSON.parse(
      await readFile(resolve(root, "window-manager.json"), "utf8"),
    );
    assert.ok(saved.views[created.data.result.view]);
    const disconnected = await request({ kind: "windows.disconnect" });
    assert.equal(disconnected.ok, true);
    const invalid = await request({ kind: "shutdown", shell: "unrecognized" });
    assert.equal(invalid.ok, false);
    const shutdown = await request({ kind: "shutdown" });
    assert.equal(shutdown.ok, true);
    console.log(
      `Native inventory: ${state.endpoints.length} audio endpoints, ${state.displays.length} displays. No audio or window transitions started.`,
    );
  } finally {
    await bridge.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("real preview, apply and undo affect only a disposable fixture window", async () => {
  const root = await mkdtemp(resolve("../../target/workspace-window-test-"));
  const title = `Desktop Workspace integration ${crypto.randomUUID()}`;
  const script = `Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class FixtureWindow { [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int n); }'; $form=New-Object System.Windows.Forms.Form; $form.Text='${title}'; $form.StartPosition='Manual'; $area=[System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea; $form.Location=New-Object System.Drawing.Point ($area.X+40),($area.Y+40); $form.Size=New-Object System.Drawing.Size 420,280; $form.Add_Shown({[FixtureWindow]::ShowWindow($form.Handle,5)|Out-Null;[Console]::Out.WriteLine('ready')}); [System.Windows.Forms.Application]::Run($form)`;
  const fixture = spawn(
    resolve(
      process.env.SystemRoot,
      "System32/WindowsPowerShell/v1.0/powershell.exe",
    ),
    ["-NoProfile", "-STA", "-Command", script],
    { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] },
  );
  const output = createInterface({ input: fixture.stdout });
  let stderr = "";
  fixture.stderr.on("data", (chunk) => (stderr += chunk));
  const bridge = new NativeBridge(
    resolve("../../target/desktop-control/debug/desktop-workspace-host.exe"),
    root,
  );
  let sequence = 0;
  const command = async (value) => {
    const result = await bridge.request(value, `window-${++sequence}`);
    assert.equal(result.ok, true, JSON.stringify(result.error));
    return result.data;
  };
  const session = (value) =>
    command({ kind: "windows.session", command: value });
  try {
    await Promise.race([
      once(output, "line"),
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error(`Fixture readiness timeout: ${stderr}`)),
          10000,
        ).unref(),
      ),
    ]);
    const connected = await command({ kind: "windows.connect" });
    const candidate = connected.state.wm.inventory.find(
      (item) => item.title === title,
    );
    assert.ok(
      candidate,
      `Owned fixture must appear in native inventory: ${JSON.stringify(connected.state.wm.inventory.map((item) => ({ title: item.title, process: item.process })))}; fixture PID ${fixture.pid}; ${stderr}`,
    );
    const baseline = candidate.frame;
    const monitor = connected.state.wm.monitors.monitors.find(
      (item) =>
        baseline.x >= item.work_area.x &&
        baseline.x < item.work_area.x + item.work_area.width &&
        baseline.y >= item.work_area.y &&
        baseline.y < item.work_area.y + item.work_area.height,
    );
    assert.ok(monitor);
    const createdSlot = await command({
      kind: "windows.slot.add",
      name: "Fixture area",
      device: monitor.device,
    });
    const slot = createdSlot.result.slot;
    const config = createdSlot.state.wm.configuration;
    config.slots[slot].region = [0.1, 0.1, 0.4, 0.4];
    await session({
      kind: "configure",
      expected_revision: config.revision,
      configuration: config,
    });
    const view = Object.keys(config.views)[0];
    const rootName = Object.keys(config.views[view].roots)[0];
    const registered = await command({
      kind: "windows.add",
      candidates: [candidate.id],
      view,
      root: rootName,
    });
    const window = Object.keys(registered.state.wm.configuration.windows)[0];
    const plan = (
      await session({
        kind: "preview",
        request: {
          id: "fixture-transition",
          expected_revision: registered.state.wm.configuration.revision,
          scope: [slot],
          targets: [{ view, roots: { [rootName]: slot } }],
          mode: "open",
          retain: [],
          selected_tabs: {},
          focus: null,
        },
      })
    ).result;
    assert.deepEqual(plan.blocked, {});
    assert.equal(Object.keys(plan.desired).length, 1);
    for (let index = 0; index < 40; index++) {
      const renewed = await session({
        kind: "preview",
        request: {
          id: `repeat-${index}`,
          expected_revision: registered.state.wm.configuration.revision,
          scope: [slot],
          targets: [{ view, roots: { [rootName]: slot } }],
          mode: "open",
          retain: [],
          selected_tabs: {},
          focus: null,
        },
      });
      Object.assign(plan, renewed.result);
    }
    const applied = (await session({ kind: "apply", plan: plan.plan })).result;
    assert.equal(applied.status, "settled", JSON.stringify(applied));
    assert.equal(applied.windows.length, 1);
    assert.equal(applied.windows[0].window, window);
    const changed = (await session({ kind: "snapshot" })).result.windows[window]
      .frame;
    assert.notDeepEqual(changed, baseline);
    const undo = (await session({ kind: "undo", id: plan.request })).result;
    assert.deepEqual(undo.blocked, {});
    assert.deepEqual(undo.restored[window].frame, baseline);
    const undone = (await session({ kind: "apply", plan: undo.plan })).result;
    assert.equal(undone.status, "settled", JSON.stringify(undone));
    const restored = (await session({ kind: "snapshot" })).result.windows[
      window
    ].frame;
    assert.deepEqual(restored, baseline);
    console.log(
      "Owned fixture window moved, resized, and restored through the real native service.",
    );
    await command({ kind: "shutdown" });
  } finally {
    await bridge.close();
    output.close();
    if (fixture.exitCode === null) {
      fixture.kill();
      await once(fixture, "exit");
    }
    await rm(root, { recursive: true, force: true });
  }
});
