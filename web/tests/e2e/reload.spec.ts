import { expect, test } from "@playwright/test";
import { MockApi, NOW, url } from "./mock";

const focusPoll = (page: import("@playwright/test").Page) => page.evaluate(() => window.dispatchEvent(new Event("focus")));

test("Load latest keeps the place, or walks up when the folder is gone (D10)", async ({ page }) => {
  const api = await new MockApi(false).install(page);
  await page.goto(url("/home/tass/old"));
  await expect(page.locator(".crumbs h1")).toHaveText("old");
  await expect(page.locator(".notice")).toHaveCount(0);
  // a nightly scan publishes a snapshot without /home/tass/old
  api.snapshotId = "snap-2";
  const tass = api.tree.children![0]!.children![0]!;
  tass.children = tass.children!.filter((c) => c.name !== "old");
  await focusPoll(page);
  const notice = page.locator(".notice").filter({ hasText: "A newer snapshot is ready" });
  await expect(notice).toBeVisible();
  // the old view stays usable until the user asks
  await expect(page.locator(".crumbs h1")).toHaveText("old");
  await page.keyboard.press("r");
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  await expect(page.locator(".toast")).toContainText("“/home/tass/old” is gone in the new snapshot; showing /home/tass");
  await expect(notice).toHaveCount(0);
});

test("Load latest keeps the selection when the row survives", async ({ page }) => {
  const api = await new MockApi(false).install(page);
  await page.goto(url());
  await expect(page.locator('tr[aria-selected="true"]')).toContainText(".cache");
  await page.keyboard.press("ArrowDown");
  await expect(page.locator('tr[aria-selected="true"]')).toContainText("Downloads");
  api.snapshotId = "snap-2";
  await focusPoll(page);
  await page.getByRole("button", { name: "Load latest · r" }).click();
  await expect(page.locator('tr[aria-selected="true"]')).toContainText("Downloads");
  expect(api.requests.some((r) => r.includes("snapshot=snap-2"))).toBe(true);
});

test("a request that meets a rotated snapshot raises the same notice", async ({ page }) => {
  const api = await new MockApi(false).install(page);
  await page.goto(url());
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  api.snapshotId = "snap-2";
  await page.keyboard.press("Enter");
  await expect(page.locator(".notice").filter({ hasText: "newer snapshot" })).toBeVisible();
  await page.keyboard.press("r");
  await expect(page.locator(".crumbs h1")).toHaveText(".cache");
});

test("Dismiss hides the notice until the next snapshot", async ({ page }) => {
  const api = await new MockApi(false).install(page);
  await page.goto(url());
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  api.snapshotId = "snap-2";
  await focusPoll(page);
  const notice = page.locator(".notice").filter({ hasText: "newer snapshot" });
  await notice.getByRole("button", { name: "Dismiss" }).click();
  await expect(notice).toHaveCount(0);
  await focusPoll(page);
  await page.waitForTimeout(300);
  await expect(notice).toHaveCount(0);
  api.snapshotId = "snap-3";
  await focusPoll(page);
  await expect(notice).toBeVisible();
});

test("a refresh that failed after the last good one raises the red notice (D5)", async ({ page }) => {
  const api = new MockApi(false);
  api.lastError = { ts: NOW, op: "refresh", message: "disk full" };
  await api.install(page);
  await page.goto(url());
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("Last refresh failed");
  await expect(alert).toContainText("disk full. Showing the snapshot from");
  await expect(alert.getByRole("button", { name: "Copy: sudo freespace scan" })).toBeVisible();
});

test("a lower-priority notice waits behind the red one (D5)", async ({ page }) => {
  const api = await new MockApi().install(page);
  await page.goto(url());
  await expect(page.getByRole("alert")).toContainText("Scan incomplete");
  api.snapshotId = "snap-2";
  await focusPoll(page);
  const more = page.getByRole("button", { name: "+1 notice" });
  await expect(more).toBeVisible();
  await more.click();
  await expect(page.locator(".notice").filter({ hasText: "newer snapshot" })).toBeVisible();
});
