import {
  cp,
  mkdir,
  readFile,
  rename,
  writeFile,
  readdir,
} from "node:fs/promises";
import { resolve, join } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const output = resolve(
  process.argv[2] ?? join(root, "../../target/desktop-workspace-package"),
);
const binaries = resolve(
  process.argv[3] ?? join(root, "../../target/desktop-workspace-build/debug"),
);
const electron = join(root, "node_modules/electron/dist");
const hash = createHash("sha256");
async function digest(path) {
  const entries = (await readdir(path, { withFileTypes: true })).sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  for (const entry of entries) {
    hash.update(entry.name);
    if (entry.isDirectory()) await digest(join(path, entry.name));
    else hash.update(await readFile(join(path, entry.name)));
  }
}
await digest(join(root, "dist"));
await digest(join(root, "shell"));
hash.update(await readFile(join(root, "server.mjs")));
hash.update(await readFile(join(root, "node_modules/electron/package.json")));
for (const name of [
  "desktop-workspace-host.exe",
  "window-manager.exe",
  "display-relay.exe",
])
  hash.update(await readFile(join(binaries, name)));
const version = "v" + hash.digest("hex").slice(0, 24);
const payload = join(output, "payload", version);
await mkdir(payload, { recursive: true });
await cp(electron, payload, { recursive: true });
await rename(
  join(payload, "electron.exe"),
  join(payload, "DesktopWorkspaceUI.exe"),
);
const appRoot = join(payload, "resources/app");
await mkdir(join(appRoot, "bin"), { recursive: true });
await cp(join(root, "dist"), join(appRoot, "dist"), { recursive: true });
await cp(join(root, "shell"), join(appRoot, "shell"), { recursive: true });
await cp(join(root, "server.mjs"), join(appRoot, "server.mjs"));
for (const name of [
  "desktop-workspace-host.exe",
  "window-manager.exe",
  "display-relay.exe",
])
  await cp(join(binaries, name), join(appRoot, "bin", name));
await writeFile(
  join(appRoot, "package.json"),
  JSON.stringify({
    name: "desktop-workspace",
    version: "0.1.0",
    main: "shell/main.cjs",
  }),
);
await cp(
  join(binaries, "desktop-workspace.exe"),
  join(output, "DesktopWorkspace.exe"),
);
await writeFile(
  join(output, "package.json"),
  JSON.stringify({ version, payload }, null, 2),
);
console.log(`Prepared ${version}: ${output}`);
