import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createWorkspaceServer } from "./server.mjs";

test("installed UI advances without restarting the daemon; version paths remain constrained", async () => {
  const root = await mkdtemp(join(tmpdir(), "workspace-ui-"));
  const server = createWorkspaceServer({
    bridge: {
      async request() {
        return { ok: true };
      },
      async close() {},
    },
    heartbeatMs: 0,
    uiInstallRoot: root,
  });
  const origin = await server.listen(0);
  try {
    for (const digit of ["1", "2"]) {
      const version = "v" + digit.repeat(24);
      const dist = join(root, "versions", version, "resources/app/dist");
      await mkdir(dist, { recursive: true });
      await writeFile(join(dist, "index.html"), digit);
      await writeFile(join(root, "current.json"), JSON.stringify({ version }));
      assert.equal(await (await fetch(origin)).text(), digit);
    }
    await writeFile(
      join(root, "current.json"),
      JSON.stringify({ version: "../../outside" }),
    );
    assert.equal((await fetch(origin)).status, 503);
  } finally {
    await server.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("daemon drains worker state even when no UI is connected", async () => {
  let snapshots = 0;
  const bridge = {
    async request(command) {
      if (command.kind === "snapshot") snapshots++;
      return { ok: true };
    },
    async close() {},
  };
  const server = createWorkspaceServer({ bridge, heartbeatMs: 20 });
  await server.listen(0);
  try {
    await new Promise((resolve) => setTimeout(resolve, 85));
    assert.ok(snapshots >= 2);
  } finally {
    await server.close();
  }
  const settled = snapshots;
  await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(snapshots, settled);
});

test("authenticated shutdown closes a daemon whose native host has already failed", async () => {
  let closed = false;
  const bridge = {
    error: new Error("host failed"),
    async request() {
      throw this.error;
    },
    async close() {
      closed = true;
    },
  };
  const server = createWorkspaceServer({ bridge, heartbeatMs: 0 });
  const origin = await server.listen(0);
  try {
    const { token } = await (await fetch(`${origin}/api/bootstrap`)).json();
    const response = await fetch(`${origin}/api/command`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Workspace-Token": token,
      },
      body: JSON.stringify({
        id: "shutdown-failed-host",
        command: { kind: "shutdown" },
      }),
    });
    assert.equal((await response.json()).ok, true);
    await new Promise((resolve) => setTimeout(resolve, 90));
    assert.equal(closed, true);
  } finally {
    await server.close();
  }
});

test("local transport requires token, matching origin, and typed JSON envelope", async () => {
  const calls = [];
  const bridge = {
    async request(command, id) {
      calls.push(command);
      return { id, ok: true, data: { state: { revision: calls.length } } };
    },
    async close() {},
  };
  const server = createWorkspaceServer({ bridge, heartbeatMs: 0 });
  const url = await server.listen(0);
  try {
    const { token } = await (await fetch(`${url}/api/bootstrap`)).json();
    const post = (headers) =>
      fetch(`${url}/api/command`, {
        method: "POST",
        headers: { "Content-Type": "application/json", ...headers },
        body: JSON.stringify({
          id: "request-1",
          command: { kind: "snapshot" },
        }),
      });
    assert.equal((await post({})).status, 401);
    assert.equal(
      (
        await post({
          "X-Workspace-Token": token,
          Origin: "https://example.invalid",
        })
      ).status,
      403,
    );
    assert.equal(
      (await post({ "X-Workspace-Token": token, Origin: url })).status,
      200,
    );
    assert.deepEqual(calls, [{ kind: "snapshot" }]);
    const invalid = await fetch(`${url}/api/command`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Workspace-Token": token,
      },
      body: JSON.stringify({ command: { kind: "shutdown" } }),
    });
    assert.equal(invalid.status, 400);
    assert.equal(calls.length, 1);
  } finally {
    await server.close();
  }
});
