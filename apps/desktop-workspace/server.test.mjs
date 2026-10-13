import test from "node:test";
import assert from "node:assert/strict";
import { createWorkspaceServer } from "./server.mjs";

test("local transport requires token, matching origin, and typed JSON envelope", async () => {
  const calls = [];
  const bridge = {
    async request(command, id) {
      calls.push(command);
      return { id, ok: true, data: { state: { revision: calls.length } } };
    },
    async close() {},
  };
  const server = createWorkspaceServer({ bridge });
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
