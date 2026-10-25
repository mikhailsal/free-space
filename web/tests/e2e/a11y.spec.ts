import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { MockApi, url } from "./mock";

for (const [w, h] of [
  [1280, 800],
  [700, 900],
  [360, 740],
] as const) {
  test(`axe finds nothing and the page never scrolls sideways at ${w} px (D16)`, async ({ page }) => {
    await page.setViewportSize({ width: w, height: h });
    await new MockApi().install(page);
    await page.goto(url());
    await expect(page.locator("table.grid tbody tr").first()).toBeVisible();
    await expect(page.locator("aside .minirow").first()).toBeVisible({ visible: w > 700 });
    const r = await new AxeBuilder({ page }).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`)).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(w);
    for (const m of ["t", "/"]) {
      await page.keyboard.press("Escape");
      await page.locator("table.grid tbody tr").first().focus();
      await page.keyboard.press(m);
      await page.waitForTimeout(200);
      expect(await page.evaluate(() => document.documentElement.scrollWidth), m).toBeLessThanOrEqual(w);
    }
  });
}

test("narrow screens: the inspector is a sheet that traps and returns focus", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 });
  await new MockApi().install(page);
  await page.goto(url());
  const row = page.locator("table.grid tbody tr").nth(1);
  await row.click();
  const sheet = page.getByRole("dialog");
  await expect(sheet).toBeVisible();
  await expect(sheet.getByRole("button", { name: "Close" })).toBeFocused();
  const box = await sheet.getByRole("button", { name: "Close" }).boundingBox();
  // layout heights are fractional (43.99997 px for 44 px rows at some widths)
  expect(Math.round(box?.height ?? 0)).toBeGreaterThanOrEqual(44);
  for (let i = 0; i < 12; i++) await page.keyboard.press("Tab");
  expect(await page.evaluate(() => !!document.activeElement?.closest("aside"))).toBe(true);
  const r = await new AxeBuilder({ page }).include("aside").analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`)).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(sheet).toHaveCount(0);
  await expect(row).toBeFocused();
});

test("help overlay: keys, recent messages, focus back where it was", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url());
  await expect(page.locator('table.grid tbody tr[aria-selected="true"]')).toContainText(".cache");
  await page.keyboard.press("h");
  await expect(page.locator(".toast")).toBeVisible();
  const row = page.locator('table.grid tbody tr[aria-selected="true"]');
  await row.focus();
  await page.keyboard.press("?");
  const dlg = page.getByRole("dialog");
  await expect(dlg).toContainText("hide until next scan");
  await expect(dlg.locator(".recent")).toContainText("Hidden until the next scan");
  const r = await new AxeBuilder({ page }).include(".dialog").analyze();
  expect(r.violations.map((v) => v.id)).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(dlg).toHaveCount(0);
  await expect(row).toBeFocused();
});

test("reduced motion: no fill animation, static progress line", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await new MockApi().install(page);
  await page.goto(url());
  const t = await page.locator(".fillline i").evaluate((el) => getComputedStyle(el).transitionDuration);
  expect(t).toBe("0s");
});
