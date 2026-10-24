import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// refactor-guard's pixel-diff (.github/workflows/refactor-guard.yml): tests/guard against the build in $GUARD_DIST,
// or web/dist built first when it is unset. The shots live in $GUARD_SHOTS: a run with --update-snapshots=all takes
// them, and the verdict compares two runs' PNG files byte for byte. A plain run compares against them as strictly as
// Playwright allows (it skips pixels it takes for anti-aliasing) and draws the differences.
const port = Number(process.env.E2E_PORT || 4174);
const serve = `PORT=${port} bun tests/e2e/serve.ts`;

export default defineConfig({
  ...base,
  testDir: "tests/guard",
  retries: 0,
  snapshotPathTemplate: `${process.env.GUARD_SHOTS || "guard-shots"}/{arg}{ext}`,
  expect: { toHaveScreenshot: { maxDiffPixels: 0, threshold: 0, animations: "disabled", caret: "hide" } },
  use: {
    ...base.use,
    baseURL: `http://127.0.0.1:${port}`,
    timezoneId: "UTC",
    locale: "en-US",
    launchOptions: { args: ["--disable-partial-raster"] },
  },
  webServer: {
    command: process.env.GUARD_DIST ? serve : `bun run build && ${serve}`,
    url: `http://127.0.0.1:${port}/`,
    reuseExistingServer: false,
    stdout: "pipe",
  },
});
