import assert from "node:assert/strict";
import { _electron as electron, expect } from "@playwright/test";
import { readFile, mkdir, cp, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { randomUUID, randomInt } from "node:crypto";

const guard = spawnSync(
  "powershell.exe",
  [
    "-NoProfile",
    "-Command",
    "$c=Get-CimInstance Win32_ComputerSystem;if($c.Model -notmatch 'Virtual Machine|VirtualBox|VMware|KVM|QEMU'){exit 1}",
  ],
  { windowsHide: true },
);
if (guard.status !== 0)
  throw new Error("Electron execution tests are restricted to the test VM");
// The Node test runner is Electron: package copies must treat .asar as raw files.
process.noAsar = true;
const install = process.env.DESKTOP_WORKSPACE_VM_INSTALL;
if (!install) throw new Error("Set DESKTOP_WORKSPACE_VM_INSTALL");
const metadata = JSON.parse(
  await readFile(join(install, "current.json"), "utf8"),
);
const root = join(
  process.env.LOCALAPPDATA,
  "DesktopEnvironment",
  "workspace-shell-test-state",
  randomUUID(),
);
await mkdir(root, { recursive: true });
const port = randomInt(4200, 4900);
const origin = `http://127.0.0.1:${port}`;
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
let app;
let token;
const budget = setTimeout(() => {
  console.error("VM Electron test exceeded five minutes");
  if (app) app.process().kill();
  void command({ kind: "shutdown" })
    .catch(() => {})
    .finally(() => process.exit(1));
}, 300000);
async function command(command) {
  token ??= (
    await (
      await fetch(`${origin}/api/bootstrap`, {
        signal: AbortSignal.timeout(5000),
      })
    ).json()
  ).token;
  const result = await (
    await fetch(`${origin}/api/command`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Workspace-Token": token,
      },
      body: JSON.stringify({ id: randomUUID(), command }),
      signal: AbortSignal.timeout(10000),
    })
  ).json();
  assert.equal(result.ok, true, JSON.stringify(result.error));
  return result.data.state;
}
async function closeUi() {
  if (!app) return;
  const child = app.process();
  await Promise.race([
    app
      .evaluate(({ BrowserWindow }) => {
        for (const win of BrowserWindow.getAllWindows())
          if (!win.isDestroyed()) win.close();
      })
      .catch(() => {}),
    new Promise((resolve) => setTimeout(resolve, 3000)),
  ]);
  const deadline = Date.now() + 8000;
  while (child.exitCode === null && Date.now() < deadline)
    await new Promise((resolve) => setTimeout(resolve, 100));
  if (child.exitCode === null) {
    child.kill();
    throw new Error("UI did not quit after closing native windows");
  }
  app = undefined;
}
async function launch() {
  const current = JSON.parse(
    await readFile(join(install, "current.json"), "utf8"),
  );
  return electron.launch({
    executablePath: join(
      install,
      "versions",
      current.version,
      "DesktopWorkspaceUI.exe",
    ),
    args: [
      "--port",
      String(port),
      "--config-root",
      root,
      "--profile",
      join(root, "profile"),
      "--panel",
      "cable",
      "--disable-gpu",
    ],
    env,
    timeout: 30000,
  });
}
async function runProcess(file, args, timeout = 30000) {
  const child = spawn(file, args, { env, windowsHide: true });
  let output = "";
  child.stdout.on("data", (data) => (output += data));
  child.stderr.on("data", (data) => (output += data));
  const timer = setTimeout(() => child.kill(), timeout);
  try {
    const code = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", resolve);
    });
    assert.equal(code, 0, output);
  } finally {
    clearTimeout(timer);
  }
}
try {
  console.log("VM step: installed stable launcher starts daemon");
  await runProcess(join(install, "DesktopWorkspace.exe"), [
    "--daemon",
    "--port",
    String(port),
    "--config-root",
    root,
    "--profile",
    join(root, "profile"),
  ]);
  await expect
    .poll(
      async () => {
        try {
          return (await command({ kind: "snapshot" })).configRoot;
        } catch {
          return null;
        }
      },
      { timeout: 20000 },
    )
    .toBe(root);
  console.log("VM step: launch native UI");
  app = await launch();
  const page = await app.firstWindow();
  const faults = [];
  page.on("pageerror", (error) => faults.push(error.message));
  const cable = page.getByRole("region", { name: "Audio Cable 패널" });
  await expect(cable.getByLabel("볼륨 백분율 직접 입력")).toBeVisible();
  for (const value of ["0.5", "0.05"]) {
    await cable.getByLabel("볼륨 백분율 직접 입력").fill(value);
    await cable.getByLabel("볼륨 백분율 직접 입력").press("Enter");
    await expect(cable.locator("output")).toHaveText(`${value}%`);
    const state = await command({ kind: "snapshot" });
    assert.ok(
      Math.abs(100 * 10 ** (state.cableSettings.gain_db / 20) - Number(value)) <
        0.00001,
    );
  }
  console.log("VM step: fractional volume passed");
  await command({ kind: "cable.save" });
  const state = await command({ kind: "refresh" });
  const pair = state.cables.find(
    (cable) => cable.name === "CABLE Input (VB-Audio Virtual Cable)",
  );
  assert.ok(pair, "VM cable fixture required");
  const target = state.endpoints.find(
    (endpoint) =>
      endpoint.name === "Voicemeeter AUX Input (VB-Audio Voicemeeter VAIO)" &&
      endpoint.flow === "playback",
  );
  assert.ok(target, "VM Voicemeeter target required");
  await command({ kind: "cable.select", id: pair.playback_id });
  await command({ kind: "cable.output", id: target.id });
  await command({ kind: "cable.start" });
  await expect
    .poll(async () => (await command({ kind: "snapshot" })).jobs.cable.phase, {
      timeout: 15000,
    })
    .toBe("running");
  console.log("VM step: real audio worker running");
  // Install a distinct immutable version while UI and audio images are open.
  const update = join(root, "update-package");
  const nextVersion = "v" + randomUUID().replaceAll("-", "").slice(0, 24);
  await cp(
    join(install, "versions", metadata.version),
    join(update, "payload", nextVersion),
    { recursive: true },
  );
  await cp(
    join(install, "DesktopWorkspace.exe"),
    join(update, "DesktopWorkspace.exe"),
  );
  const updatedHtml = join(
    update,
    "payload",
    nextVersion,
    "resources/app/dist/index.html",
  );
  await writeFile(
    updatedHtml,
    (await readFile(updatedHtml, "utf8")) + "<!-- vm-ui-update -->",
  );
  await writeFile(
    join(update, "package.json"),
    JSON.stringify({ version: nextVersion }),
  );
  await runProcess(
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      resolve("../../tools/install-desktop-workspace.ps1"),
      "-InstallRoot",
      install,
      "-PackageRoot",
      update,
      "-SkipBuild",
    ],
    60000,
  );
  assert.equal(
    JSON.parse(await readFile(join(install, "current.json"), "utf8")).version,
    nextVersion,
  );
  assert.equal(
    (await command({ kind: "snapshot" })).jobs.cable.phase,
    "running",
  );
  console.log("VM step: update while UI and audio are open passed");
  await page.evaluate(() => {
    window.__vmDragEvents = [];
    for (const type of ["dragstart", "dragend", "mousedown", "mouseup"])
      document.addEventListener(
        type,
        (event) =>
          window.__vmDragEvents.push({
            type,
            target: event.target?.className,
            x: event.screenX,
            y: event.screenY,
            effect: event.dataTransfer?.dropEffect,
          }),
        true,
      );
  });
  assert.ok(
    (
      await (await fetch(origin, { signal: AbortSignal.timeout(5000) })).text()
    ).includes("vm-ui-update"),
    "Daemon must serve the new UI without interrupting jobs",
  );
  const native = await app.evaluate(({ BrowserWindow, screen }) => {
    const win = BrowserWindow.getAllWindows()[0];
    const area = screen.getDisplayMatching(win.getBounds()).workArea;
    win.setBounds({
      x: area.x + 20,
      y: area.y + 20,
      width: Math.min(700, area.width - 160),
      height: Math.min(550, area.height - 100),
    });
    win.focus();
    return {
      frame: win.isMenuBarVisible(),
      bounds: win.getContentBounds(),
      outer: win.getBounds(),
    };
  });
  assert.equal(native.frame, false, "Application menu must be absent");
  assert.ok(
    native.outer.height > native.bounds.height,
    "Windows title bar must remain",
  );
  await page.waitForTimeout(350); // Allow Windows to settle native bounds/focus before physical input.
  await page.screenshot({ path: join(root, "main.png") });
  const tab = page
    .locator(".dv-tab")
    .filter({ hasText: "Audio Cable" })
    .first();
  const box = await tab.boundingBox();
  assert.ok(box);
  const points = await app.evaluate(
    ({ screen }, { source, target }) => ({
      source: screen.dipToScreenPoint(source),
      target: screen.dipToScreenPoint(target),
    }),
    {
      source: {
        x: Math.round(native.bounds.x + box.x + box.width / 2),
        y: Math.round(native.bounds.y + box.y + box.height / 2),
      },
      target: {
        x: native.outer.x + native.outer.width + 70,
        y: native.bounds.y + 150,
      },
    },
  );
  const mouse = spawn(
    "powershell.exe",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      resolve("../../tools/workspace-shell-mouse-vm.ps1"),
      "-SourceX",
      String(points.source.x),
      "-SourceY",
      String(points.source.y),
      "-TargetX",
      String(points.target.x),
      "-TargetY",
      String(points.target.y),
    ],
    { windowsHide: true, encoding: "utf8" },
  );
  let mouseErrors = "";
  mouse.stderr.on("data", (data) => (mouseErrors += data.toString()));
  const mouseTimer = setTimeout(() => mouse.kill(), 10000);
  const mouseExit = await new Promise((resolve, reject) => {
    mouse.once("error", reject);
    mouse.once("exit", (code) => resolve(code));
  });
  clearTimeout(mouseTimer);
  assert.equal(mouseExit, 0, mouseErrors);
  console.log("VM step: physical drag released outside");
  console.log(
    "VM drag evidence:",
    JSON.stringify({
      native,
      box,
      points,
      events: await page.evaluate(() => window.__vmDragEvents),
      faults,
    }),
  );
  await expect.poll(() => app.windows().length, { timeout: 15000 }).toBe(2);
  const popup = app.windows().find((window) => window !== page);
  await expect(
    popup.getByRole("region", { name: "Audio Cable 패널" }),
  ).toBeVisible();
  await expect(popup.locator("output")).toHaveText("0.05%");
  await popup.screenshot({ path: join(root, "popout.png") });
  assert.equal(
    (await command({ kind: "snapshot" })).jobs.cable.phase,
    "running",
  );
  console.log("VM step: popout state passed; close native windows");
  await closeUi();
  assert.equal(
    (await command({ kind: "snapshot" })).jobs.cable.phase,
    "running",
    "Daemon job must survive all UI windows closing",
  );
  app = await launch();
  console.log("VM step: reopened UI");
  const reopened = await app.firstWindow();
  await expect(
    reopened
      .getByRole("region", { name: "Audio Cable 패널" })
      .locator("output"),
  ).toHaveText("0.05%");
  assert.equal(
    (await command({ kind: "snapshot" })).jobs.cable.phase,
    "running",
  );
  assert.deepEqual(faults, []);
  for (const name of ["main", "popout"])
    console.log(
      `VM_WORKSPACE_IMAGE ${name} ${(await readFile(join(root, `${name}.png`))).toString("base64")}`,
    );
  console.log(
    `PASS fractional volume, native window, physical tab tear-off, daemon survival and UI reconnect. Evidence: ${root}`,
  );
} catch (error) {
  console.error(error);
  if (app) {
    console.log(
      "VM renderer faults:",
      await app
        .evaluate(({ BrowserWindow }) =>
          BrowserWindow.getAllWindows().map((win) => ({
            url: win.webContents.getURL(),
            bounds: win.getBounds(),
          })),
        )
        .catch(() => []),
    );
    const window = app.windows()[0];
    if (window)
      console.log(
        `VM_WORKSPACE_IMAGE failure ${(await window.screenshot()).toString("base64")}`,
      );
  }
  throw error;
} finally {
  clearTimeout(budget);
  if (app) {
    try {
      await closeUi();
    } catch (error) {
      console.error(error);
    }
  }
  try {
    await command({ kind: "shutdown" });
  } catch {}
}
