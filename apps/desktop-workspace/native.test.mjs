import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { resolve } from "node:path";
import { NativeBridge } from "./server.mjs";

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
