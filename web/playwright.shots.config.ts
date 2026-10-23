import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// `bun run shots`: README screenshots only (tests/shots), not part of CI.
export default defineConfig({ ...base, testDir: "tests/shots", retries: 0 });
