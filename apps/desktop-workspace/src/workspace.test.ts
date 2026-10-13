import test from "node:test";
import assert from "node:assert/strict";
import { createFixtureBackend } from "./workspace.ts";

test("exclusive startup returns a structured error with a request trace", async () => {
  const backend = createFixtureBackend(0);
  await backend.dispatch({ kind: "audio.scenario", scenario: "exclusive" });
  const result = await backend.dispatch({ kind: "audio.start" });
  assert.equal(result.ok, false);
  assert.equal(result.issue?.code, "AUDCLNT_E_DEVICE_IN_USE");
  assert.equal(backend.getSnapshot().audio.phase, "failed");
  assert.ok(
    backend
      .getSnapshot()
      .traces.some(
        (trace) => trace.id === result.requestId && trace.status === "error",
      ),
  );
});
test("the default alias cannot bypass feedback prevention", async () => {
  const backend = createFixtureBackend(0);
  await backend.dispatch({ kind: "audio.select", role: "target", id: "cable" });
  const result = await backend.dispatch({ kind: "audio.start" });
  assert.equal(result.issue?.code, "FEEDBACK_PREVENTED");
});
test("stop cancels an in-flight startup; late completion cannot restart it", async () => {
  const backend = createFixtureBackend(15);
  const pending = backend.dispatch({ kind: "audio.start" });
  await backend.dispatch({ kind: "audio.stop" });
  const result = await pending;
  assert.equal(result.ok, false);
  assert.equal(backend.getSnapshot().audio.phase, "stopped");
  assert.ok(
    backend
      .getSnapshot()
      .traces.some(
        (trace) =>
          trace.id === result.requestId && trace.status === "cancelled",
      ),
  );
});
test("selection changes invalidate a window preview before apply", async () => {
  const backend = createFixtureBackend();
  await backend.dispatch({ kind: "windows.preview" });
  assert.equal(backend.getSnapshot().preview?.changes.length, 2);
  await backend.dispatch({
    kind: "windows.select",
    id: "terminal",
    selected: true,
  });
  const result = await backend.dispatch({ kind: "windows.apply" });
  assert.equal(result.issue?.code, "STALE_PREVIEW");
  assert.ok(
    backend
      .getSnapshot()
      .windows.every((window) => window.region === "현재 위치"),
  );
});
test("applying a plan affects only the selected mock windows", async () => {
  const backend = createFixtureBackend();
  await backend.dispatch({ kind: "windows.preview" });
  assert.equal((await backend.dispatch({ kind: "windows.apply" })).ok, true);
  assert.equal(
    backend.getSnapshot().windows.find((window) => window.id === "terminal")
      ?.region,
    "현재 위치",
  );
  assert.equal(backend.getSnapshot().preview, undefined);
  assert.equal((await backend.dispatch({ kind: "windows.apply" })).ok, false);
});
test("volume rejects nonfinite/out-of-range input and keeps its previous value", async () => {
  const backend = createFixtureBackend();
  for (const value of [NaN, Infinity, -1, 101])
    assert.equal(
      (await backend.dispatch({ kind: "cable.volume", value })).ok,
      false,
    );
  assert.equal(backend.getSnapshot().cable.volume, 35);
  await backend.dispatch({ kind: "cable.output", id: "default" });
  assert.equal(
    (await backend.dispatch({ kind: "cable.start" })).issue?.code,
    "FEEDBACK_PREVENTED",
  );
});
