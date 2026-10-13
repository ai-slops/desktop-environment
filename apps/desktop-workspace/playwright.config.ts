import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests",
  workers: 1,
  fullyParallel: false,
  use: {
    baseURL: "http://127.0.0.1:4177",
    viewport: { width: 1440, height: 1000 },
    channel:
      process.env.PW_CHANNEL ??
      (process.platform === "win32" ? "chrome" : undefined),
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  webServer: {
    command:
      "node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 4177 --strictPort",
    url: "http://127.0.0.1:4177",
    reuseExistingServer: !process.env.CI,
  },
});
