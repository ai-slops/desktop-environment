import { createServer as httpServer } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve, sep, extname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const bodyLimit = 1024 * 1024;
export class NativeBridge {
  pending = new Map();
  error = null;
  constructor(executable, configRoot) {
    const args = configRoot ? ["--config-root", configRoot] : [];
    this.child = spawn(executable, args, {
      windowsHide: true,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stderr = "";
    this.child.stdin.on("error", (error) => this.fail(error));
    this.child.stderr.on("data", (chunk) => {
      stderr = (stderr + chunk.toString()).slice(-6000);
    });
    createInterface({ input: this.child.stdout, crlfDelay: Infinity }).on(
      "line",
      (line) => {
        let response;
        try {
          response = JSON.parse(line);
        } catch {
          this.fail(new Error("Native protocol returned malformed JSON"));
          return;
        }
        const pending = this.pending.get(response.id);
        if (!pending) return;
        clearTimeout(pending.timer);
        this.pending.delete(response.id);
        pending.resolve(response);
      },
    );
    this.child.on("error", (error) => this.fail(error));
    this.child.on("exit", (code) =>
      this.fail(new Error(`Native host exited (${code}). ${stderr}`), false),
    );
  }
  fail(error, kill = true) {
    this.error = error;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    if (kill) this.child.kill();
  }
  request(command, id = randomUUID()) {
    if (this.error) return Promise.reject(this.error);
    if (this.pending.has(id))
      return Promise.reject(new Error("Duplicate in-flight request ID"));
    if (this.pending.size >= 32)
      return Promise.reject(new Error("Too many pending commands"));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () =>
          this.fail(
            new Error(
              "Native command timed out; host stopped to prevent late execution",
            ),
          ),
        40_000,
      );
      this.pending.set(id, { resolve, reject, timer });
      this.child.stdin.write(
        JSON.stringify({ id, command }) + "\n",
        (error) => {
          if (error) this.fail(error);
        },
      );
    });
  }
  async close() {
    this.child.stdin.end();
    if (this.child.exitCode !== null) return;
    await new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.child.kill();
        resolve();
      }, 3000);
      this.child.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
  }
}

function authorized(token, given) {
  if (typeof given !== "string") return false;
  const a = Buffer.from(token);
  const b = Buffer.from(given);
  return a.length === b.length && timingSafeEqual(a, b);
}
async function body(request) {
  let size = 0;
  const chunks = [];
  for await (const chunk of request) {
    size += chunk.length;
    if (size > bodyLimit)
      throw Object.assign(new Error("Command exceeds 1 MiB"), { status: 413 });
    chunks.push(chunk);
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw Object.assign(new Error("Malformed JSON request"), { status: 400 });
  }
}
export function createWorkspaceServer({
  bridge,
  heartbeatMs = 1000,
  uiInstallRoot,
  distRoot = fileURLToPath(new URL("./dist/", import.meta.url)),
}) {
  const token = randomBytes(32).toString("hex");
  let port = 0;
  const jsonResponse = (response, status, data) => {
    response.writeHead(status, {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
    });
    response.end(JSON.stringify(data));
  };
  const server = httpServer(async (request, response) => {
    const allowedHosts = new Set([`127.0.0.1:${port}`, `localhost:${port}`]);
    const host = request.headers.host;
    const origin = request.headers.origin;
    if (!allowedHosts.has(host) || (origin && origin !== `http://${host}`)) {
      jsonResponse(response, 403, {
        error: {
          code: "ORIGIN_REJECTED",
          message: "This workspace accepts same-origin local requests only",
        },
      });
      return;
    }
    try {
      const url = new URL(request.url, `http://${host}`);
      if (request.method === "GET" && url.pathname === "/api/bootstrap") {
        jsonResponse(response, 200, { token, mode: "native", version: 1 });
        return;
      }
      if (url.pathname.startsWith("/api/")) {
        if (!authorized(token, request.headers["x-workspace-token"])) {
          jsonResponse(response, 401, {
            error: {
              code: "UNAUTHORIZED",
              message: "Missing workspace session token",
            },
          });
          return;
        }
        if (request.method !== "POST" || url.pathname !== "/api/command") {
          jsonResponse(response, 405, {
            error: {
              code: "METHOD_REJECTED",
              message: "Use POST /api/command",
            },
          });
          return;
        }
        if (!request.headers["content-type"]?.startsWith("application/json")) {
          jsonResponse(response, 415, {
            error: {
              code: "JSON_REQUIRED",
              message: "JSON content type required",
            },
          });
          return;
        }
        const envelope = await body(request);
        if (
          !envelope.command ||
          typeof envelope.command.kind !== "string" ||
          typeof envelope.id !== "string" ||
          !envelope.id ||
          envelope.id.length > 200
        ) {
          jsonResponse(response, 400, {
            error: {
              code: "INVALID_REQUEST",
              message: "A request ID and typed command are required",
            },
          });
          return;
        }
        const result =
          bridge.error &&
          envelope.command.kind === "shutdown" &&
          Object.keys(envelope.command).length === 1
            ? { id: envelope.id, ok: true, data: { result: { stopped: true } } }
            : await bridge.request(envelope.command, envelope.id);
        jsonResponse(response, 200, result);
        if (envelope.command.kind === "shutdown" && result.ok)
          setTimeout(() => {
            void close();
          }, 50);
        return;
      }
      if (request.method !== "GET") {
        response.writeHead(405);
        response.end();
        return;
      }
      const pathname = decodeURIComponent(url.pathname);
      // UI updates advance independently of the daemon and its native jobs.
      let staticRoot = distRoot;
      if (uiInstallRoot) {
        const current = JSON.parse(
          await readFile(resolve(uiInstallRoot, "current.json"), "utf8"),
        );
        if (!/^v[a-f0-9]{24}$/.test(current.version))
          throw new Error("Invalid installed UI version");
        staticRoot = resolve(
          uiInstallRoot,
          "versions",
          current.version,
          "resources/app/dist",
        );
      }
      const filename = resolve(
        staticRoot,
        pathname === "/" ? "index.html" : `.${pathname}`,
      );
      if (!filename.startsWith(resolve(staticRoot) + sep)) {
        response.writeHead(403);
        response.end();
        return;
      }
      const mime =
        {
          ".html": "text/html; charset=utf-8",
          ".js": "text/javascript; charset=utf-8",
          ".css": "text/css; charset=utf-8",
          ".svg": "image/svg+xml",
          ".png": "image/png",
        }[extname(filename)] ?? "application/octet-stream";
      const content = await readFile(filename);
      response.writeHead(200, {
        "Content-Type": mime,
        "X-Content-Type-Options": "nosniff",
        "Cache-Control":
          extname(filename) === ".html" ? "no-store" : "public, max-age=3600",
      });
      response.end(content);
    } catch (error) {
      jsonResponse(
        response,
        error.code === "ENOENT" ? 404 : (error.status ?? 503),
        {
          ok: false,
          error: { code: "SERVICE_UNAVAILABLE", message: error.message },
        },
      );
    }
  });
  let closing;
  let heartbeatPending = false;
  const heartbeat =
    heartbeatMs > 0
      ? setInterval(() => {
          if (heartbeatPending || closing) return;
          heartbeatPending = true;
          void bridge
            .request({ kind: "snapshot" })
            .catch(() => {})
            .finally(() => {
              heartbeatPending = false;
            });
        }, heartbeatMs)
      : undefined;
  heartbeat?.unref();
  function close() {
    closing ??= (async () => {
      clearInterval(heartbeat);
      await bridge.close();
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    })();
    return closing;
  }
  return {
    async listen(requestedPort = 4178) {
      await new Promise((resolve, reject) => {
        server.once("error", reject);
        server.listen(requestedPort, "127.0.0.1", resolve);
      });
      port = server.address().port;
      return `http://127.0.0.1:${port}`;
    },
    close,
  };
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  let port = 4178;
  let configRoot;
  let uiInstallRoot;
  let executable = fileURLToPath(
    new URL(
      `../../target/desktop-control/debug/desktop-workspace-host${process.platform === "win32" ? ".exe" : ""}`,
      import.meta.url,
    ),
  );
  const args = process.argv.slice(2);
  while (args.length) {
    const option = args.shift();
    const value = args.shift();
    if (!value) throw new Error(`${option} requires a value`);
    if (option === "--port") {
      port = Number(value);
      if (!Number.isInteger(port) || port < 1 || port > 65535)
        throw new Error("Invalid port");
    } else if (option === "--host-exe") executable = resolve(value);
    else if (option === "--config-root") configRoot = resolve(value);
    else if (option === "--ui-install-root") uiInstallRoot = resolve(value);
    else throw new Error(`Unknown option ${option}`);
  }
  const bridge = new NativeBridge(executable, configRoot);
  const workspace = createWorkspaceServer({ bridge, uiInstallRoot });
  try {
    console.log(`Desktop Workspace ready: ${await workspace.listen(port)}`);
  } catch (error) {
    await workspace.close();
    throw error;
  }
  process.once("SIGINT", () => {
    void workspace.close();
  });
  process.once("SIGTERM", () => {
    void workspace.close();
  });
}
