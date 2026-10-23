// README screenshots (T46): Browse and Changes on the made-up /home/alex.
// Regenerate with `bun run shots`; writes docs/img/gui.png and gui-changes.png.
import { expect, test } from "@playwright/test";
import { NOW, url } from "../e2e/mock";
import { ShotApi } from "./home";

test.use({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 2 });

test.beforeEach(async ({ page }) => {
  // the ages on the page ("3 h", "snapshot 7 h ago") read the same on any day
  await page.clock.setFixedTime(new Date(NOW * 1000));
  await new ShotApi().install(page);
});

test("browse", async ({ page }) => {
  await page.goto(url("/home/alex"));
  await expect(page.locator("table.grid tbody tr").first()).toBeVisible();
  await expect(page.locator("aside .minirow").first()).toBeVisible();
  await page.screenshot({ path: "../docs/img/gui.png" });
});

test("changes", async ({ page }) => {
  await page.goto(url("/home/alex"));
  await expect(page.locator("table.grid tbody tr").first()).toBeVisible();
  await page.keyboard.press("d");
  await expect(page.locator("table.grid tbody tr").filter({ hasText: ".ollama" })).toBeVisible();
  await page.screenshot({ path: "../docs/img/gui-changes.png" });
});
