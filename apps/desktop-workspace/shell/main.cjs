const {
  app,
  BrowserWindow,
  Menu,
  dialog,
  ipcMain,
  screen,
  shell,
} = require("electron");
const { spawn } = require("node:child_process");
const { join, resolve } = require("node:path");
const { mkdirSync, openSync, closeSync, existsSync } = require("node:fs");

const args = process.argv.slice(1);
function option(name, fallback) {
  const i = args.indexOf(name);
  return i < 0 ? fallback : args[i + 1];
}
const port = Number(option("--port", "4178"));
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw new Error("Invalid port");
const origin = `http://127.0.0.1:${port}`;
const configRoot = resolve(
  option(
    "--config-root",
    join(
      process.env.LOCALAPPDATA ||
        process.env.XDG_STATE_HOME ||
        app.getPath("home"),
      "DesktopEnvironment",
    ),
  ),
);
const stateRoot = resolve(option("--state-root", join(configRoot, "daemon")));
const base = resolve(__dirname, "..");
const installRoot = resolve(base, "../../../..");
const binaryRoot = option("--binaries", join(base, "bin"));
const profileBase = option("--profile", join(configRoot, "webview"));
const daemonOnly = args.includes("--daemon");
const stopOnly = args.includes("--stop-service");
const profile =
  daemonOnly || stopOnly ? join(profileBase, "service-control") : profileBase;
const windows = new Set();
mkdirSync(profile, { recursive: true });
app.setPath("userData", profile);
Menu.setApplicationMenu(null);
function trusted(contents) {
  try {
    return new URL(contents.getURL()).origin === origin;
  } catch {
    return false;
  }
}
async function bootstrap() {
  const response = await fetch(`${origin}/api/bootstrap`, {
    signal: AbortSignal.timeout(1500),
  });
  if (!response.ok) throw new Error("Service unavailable");
  const data = await response.json();
  if (
    data.version !== 1 ||
    data.mode !== "native" ||
    typeof data.token !== "string"
  )
    throw Object.assign(new Error("Port belongs to another service"), {
      listening: true,
    });
  const snapshot = await fetch(`${origin}/api/command`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Workspace-Token": data.token,
    },
    body: JSON.stringify({
      id: require("node:crypto").randomUUID(),
      command: { kind: "snapshot" },
    }),
    signal: AbortSignal.timeout(5000),
  })
    .then((response) => response.json())
    .catch((error) => {
      throw Object.assign(error, { listening: true });
    });
  if (!snapshot.ok)
    throw Object.assign(
      new Error(
        snapshot.error?.message ||
          "Native service failed; stop the service and start it again.",
      ),
      { listening: true },
    );
  if (
    resolve(snapshot.data.state.configRoot).toLowerCase() !==
    configRoot.toLowerCase()
  )
    throw Object.assign(
      new Error(
        "This port serves another settings directory. Choose a different port for this instance.",
      ),
      { listening: true },
    );
  return data;
}
async function ensureDaemon() {
  try {
    return await bootstrap();
  } catch (error) {
    // A listening incompatible service must never be replaced or killed.
    if (error.listening) throw error;
  }
  mkdirSync(stateRoot, { recursive: true });
  const log = openSync(join(stateRoot, "server.log"), "a");
  const errors = openSync(join(stateRoot, "server-error.log"), "a");
  const child = spawn(
    process.execPath,
    [
      join(base, "server.mjs"),
      "--port",
      String(port),
      "--host-exe",
      join(binaryRoot, "desktop-workspace-host.exe"),
      "--config-root",
      configRoot,
      ...(existsSync(join(installRoot, "current.json"))
        ? ["--ui-install-root", installRoot]
        : []),
    ],
    {
      detached: true,
      windowsHide: true,
      stdio: ["ignore", log, errors],
      env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" },
      cwd: base,
    },
  );
  closeSync(log);
  closeSync(errors);
  child.unref();
  let spawnError;
  child.on("error", (error) => {
    spawnError = error;
  });
  for (let attempt = 0; attempt < 60; attempt++) {
    if (spawnError) throw spawnError;
    try {
      return await bootstrap();
    } catch (error) {
      if (error.listening) throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  throw new Error(`Service did not start. Logs: ${stateRoot}`);
}
function secureWindow(win) {
  windows.add(win);
  win.setMenu(null);
  win.setMenuBarVisibility(false);
  win.on("closed", () => windows.delete(win));
  win.webContents.on("will-navigate", (event, url) => {
    if (new URL(url).origin !== origin) event.preventDefault();
  });
  win.webContents.setWindowOpenHandler((details) => {
    const url = new URL(details.url);
    if (url.origin !== origin || url.pathname !== "/popout.html") {
      if (url.protocol === "https:") void shell.openExternal(details.url);
      return { action: "deny" };
    }
    return {
      action: "allow",
      overrideBrowserWindowOptions: {
        frame: true,
        autoHideMenuBar: true,
        minWidth: 360,
        minHeight: 280,
        webPreferences: {
          preload: join(__dirname, "preload.cjs"),
          contextIsolation: true,
          nodeIntegration: false,
          sandbox: true,
        },
      },
      createWindow: (options) => {
        const popup = new BrowserWindow({
          ...options,
          frame: true,
          autoHideMenuBar: true,
          webPreferences: {
            ...options.webPreferences,
            preload: join(__dirname, "preload.cjs"),
            contextIsolation: true,
            nodeIntegration: false,
            sandbox: true,
          },
        });
        secureWindow(popup);
        return popup.webContents;
      },
    };
  });
}
ipcMain.on("workspace:close-ui", (event) => {
  if (trusted(event.sender))
    BrowserWindow.fromWebContents(event.sender)?.close();
});
ipcMain.handle("workspace:drag-finished", (event, point) => {
  if (
    !trusted(event.sender) ||
    !point ||
    !Number.isFinite(point.x) ||
    !Number.isFinite(point.y)
  )
    return null;
  const cursor = screen.getCursorScreenPoint();
  // Consult every native content rectangle: a cross-window drop is not a new popout.
  if (
    [...windows].some((win) => {
      if (win.isDestroyed()) return false;
      const b = win.getContentBounds();
      return (
        cursor.x >= b.x &&
        cursor.x < b.x + b.width &&
        cursor.y >= b.y &&
        cursor.y < b.y + b.height
      );
    })
  )
    return null;
  return { x: cursor.x, y: cursor.y };
});
app.on("window-all-closed", () => app.quit()); // Only the UI exits. The detached daemon owns jobs.
if (!daemonOnly && !stopOnly && !app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on("second-instance", () => {
    const first = [...windows][0];
    if (first) {
      if (first.isMinimized()) first.restore();
      first.show();
      first.focus();
    }
  });
  app
    .whenReady()
    .then(async () => {
      if (stopOnly) {
        const data = await bootstrap();
        await fetch(`${origin}/api/command`, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-Workspace-Token": data.token,
          },
          body: JSON.stringify({
            id: require("node:crypto").randomUUID(),
            command: { kind: "shutdown" },
          }),
        });
        app.quit();
        return;
      }
      await ensureDaemon();
      if (daemonOnly) {
        app.quit();
        return;
      }
      const win = new BrowserWindow({
        title: "Desktop Workspace",
        width: 1100,
        height: 800,
        minWidth: 500,
        minHeight: 360,
        frame: true,
        autoHideMenuBar: true,
        show: false,
        webPreferences: {
          preload: join(__dirname, "preload.cjs"),
          contextIsolation: true,
          nodeIntegration: false,
          sandbox: true,
        },
      });
      secureWindow(win);
      win.once("ready-to-show", () => win.show());
      const query = new URLSearchParams();
      if (args.includes("--fixture")) query.set("mode", "fixture");
      const panel = option("--panel", "");
      if (["audio", "cable", "windows", "diagnostics"].includes(panel))
        query.set("panel", panel);
      await win.loadURL(`${origin}/${query.size ? "?" + query : ""}`);
    })
    .catch(async (error) => {
      await dialog.showMessageBox({
        type: "error",
        title: "Desktop Workspace",
        message: error.message,
      });
      app.quit();
    });
}
