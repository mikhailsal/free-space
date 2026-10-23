import { defineConfig } from "@playwright/test";

// scripts/test-files.sh (the pre-gate runner) sets E2E_PORT to a free port for each run.
const port = Number(process.env.E2E_PORT || 4173);

export default defineConfig({
  testDir: "tests/e2e",
  timeout: 30_000,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    // PW_CHANNEL=chrome uses an installed Chrome instead of the bundled build
    channel: process.env.PW_CHANNEL || undefined,
    trace: "retain-on-failure",
  },
  webServer: {
    command: `bun run build && PORT=${port} bun tests/e2e/serve.ts`,
    url: `http://127.0.0.1:${port}/`,
    // a server already on a port the runner picked belongs to someone else
    reuseExistingServer: !process.env.CI && !process.env.E2E_PORT,
    stdout: "pipe",
  },
});
