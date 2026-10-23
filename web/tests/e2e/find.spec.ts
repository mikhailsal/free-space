import { expect, test } from "@playwright/test";
import { MockApi, url } from "./mock";

test("typing sends nothing; Enter sends one search (D9)", async ({ page }) => {
  const api = await new MockApi().install(page);
  await page.goto(url());
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  await page.keyboard.press("/");
  await expect(page.locator("#f-name")).toBeFocused();
  await page.keyboard.type("*.iso");
  await page.locator("#f-min").fill("1G");
  expect(api.count("find")).toBe(0);
  await page.locator("#f-name").press("Enter");
  await expect(page.locator(".summary")).toContainText("1 match · 5.0 G allocated");
  expect(api.count("find")).toBe(1);
  await expect(page.locator("table.grid tbody tr")).toHaveCount(1);
  await expect(page.locator("table.grid tbody tr")).toContainText("Downloads/ubuntu.iso");
  await expect(page.locator("aside .cmdline code")).toHaveText("freespace find /home/tass --name '*.iso' --kind any --min-size 1G --json");
  // single-key shortcuts are off inside fields
  await page.locator("#f-name").press("t");
  await expect(page.getByRole("tab", { name: "Find" })).toHaveAttribute("aria-selected", "true");
});

test("filter chips remove one filter each", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url("/home/tass?mode=find&name=*.mp4&min=1G&run=1"));
  await expect(page.locator("table.grid tbody tr")).toHaveCount(1);
  await page.getByRole("button", { name: "Remove filter name: *.mp4" }).click();
  await expect(page).not.toHaveURL(/name=/);
  await expect(page.locator(".summary")).toContainText("matches");
});

test("bad values are flagged at the field and never sent", async ({ page }) => {
  const api = await new MockApi().install(page);
  await page.goto(url("/home/tass?mode=find"));
  await page.locator("#f-min").fill("lots");
  await page.locator("#f-before").fill("last week");
  await page.getByRole("button", { name: "Find", exact: true }).click();
  const field = page.locator("#f-before");
  await expect(field).toHaveAttribute("aria-invalid", "true");
  await expect(page.locator("#f-before-err")).toHaveText("Enter a date as YYYY-MM-DD or a duration such as 180d.");
  await expect(field).toBeFocused();
  expect(api.count("find")).toBe(0);
});

test("Esc leaves Find for Browse and keeps the filters for next time", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url("/home/tass?mode=find&name=*.iso&run=1"));
  await expect(page.locator("table.grid tbody tr")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tab", { name: "Browse" })).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("/");
  await expect(page.locator("#f-name")).toHaveValue("*.iso");
});
