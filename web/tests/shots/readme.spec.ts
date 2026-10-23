// README screenshot (T46): the page on the mocked /home/tass tree.
// Regenerate with `bun run shots`; writes docs/img/gui.png.
import { expect, test } from "@playwright/test";
import { MockApi, url } from "../e2e/mock";

test("gui screenshot for the README", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await new MockApi(false).install(page);
  await page.goto(url());
  await expect(page.locator("table.grid tbody tr").first()).toBeVisible();
  await expect(page.locator("aside .minirow").first()).toBeVisible();
  await page.screenshot({ path: "../docs/img/gui.png" });
});
