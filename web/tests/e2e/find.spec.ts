import { expect, test } from "@playwright/test";
import { DAY, file, loseAborts, M, MockApi, NOW, url } from "./mock";

const rows = (page: import("@playwright/test").Page) => page.locator("table.grid tbody tr");

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

test("/ puts the cursor in the name field when hashchange comes after a 0 ms timer", async ({ page }) => {
  // Chromium does not order the hashchange that draws the form against timers; make it late
  await page.addInitScript(() => {
    const add = window.addEventListener.bind(window);
    window.addEventListener = ((type: string, fn: EventListener, opts?: boolean | AddEventListenerOptions) =>
      add(type, type === "hashchange" ? (e: Event) => setTimeout(() => fn(e), 50) : fn, opts)) as typeof window.addEventListener;
  });
  await new MockApi().install(page);
  await page.goto(url());
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  await page.keyboard.press("/");
  await expect(page.locator("#f-name")).toBeFocused();
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

test("Show more asks for twice the rows and keeps the selection", async ({ page }) => {
  const api = new MockApi();
  const tass = api.node("/home/tass");
  tass.children ??= [];
  for (let i = 1; i <= 35; i++) tass.children.push(file(`part-${String(i).padStart(2, "0")}.bin`, (400 - 5 * i) * M, NOW - i * DAY));
  await api.install(page);
  await page.goto(url("/home/tass?mode=find&name=part-*&run=1"));
  await expect(rows(page)).toHaveCount(30);
  await expect(page.locator(".more")).toHaveText("Showing 30 of 35 · Show more");
  await rows(page).nth(2).click();
  await page.getByRole("button", { name: "Show more" }).click();
  await expect(rows(page)).toHaveCount(35);
  await expect(page.locator(".more")).toBeHidden();
  await expect(page).toHaveURL(/[?&]limit=60(&|$)/);
  await expect(page.locator('table.grid tbody tr[aria-selected="true"]')).toContainText("part-03.bin");
  const limits = api.requests.filter((r) => r.startsWith("/api/find")).map((r) => new URL(r, "http://mock").searchParams.get("limit"));
  expect(limits).toEqual(["30", "60"]);
});

// As in browse.spec.ts: with the abort lost, the late answer arrives and only the count of loads drops it.
for (const lost of [false, true]) {
  test(`a search cancelled while it runs keeps the previous results, and its late answer changes nothing${lost ? ", even with the abort lost" : ""} (D19)`, async ({
    page,
  }) => {
    if (lost) await loseAborts(page);
    const api = await new MockApi().install(page);
    await page.goto(url("/home/tass?mode=find&name=*.iso&run=1"));
    await expect(rows(page)).toHaveCount(1);
    const slow = api.hold("find");
    await page.locator("#f-name").fill("*.mp4");
    await page.locator("#f-name").press("Enter");
    const late = await slow.arrived;
    await expect(page.locator(".summary")).toContainText("Searching… name filters can take a few seconds");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator(".toast")).toHaveText("Search cancelled");
    await expect(page.locator(".summary")).toContainText("1 match · 5.0 G allocated");
    await expect(page.locator("main")).toHaveAttribute("aria-busy", "false");
    await slow.release();
    // the page has the late answer in full before the checks below
    if (lost) await (await late.response())?.finished();
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toContainText("Downloads/ubuntu.iso");
    await expect(page.locator(".summary")).toContainText("1 match · 5.0 G allocated");
    await expect(page.locator(".progress")).not.toHaveClass(/\bon\b/);
  });
}

// A load that is no longer the latest leaves the busy state to the one that is: ending the search it gave up on must not
// end the newer search's progress and its Cancel.
for (const lost of [false, true]) {
  test(`a search given up on for a newer one leaves the newer one busy and cancellable${lost ? ", even with the abort lost" : ""} (D19)`, async ({
    page,
  }) => {
    if (lost) await loseAborts(page);
    const api = await new MockApi().install(page);
    await page.goto(url("/home/tass?mode=find&name=*.iso&run=1"));
    await expect(rows(page)).toHaveCount(1);
    const first = api.hold("find", (q) => q.get("name") === "*.mp4");
    const second = api.hold("find", (q) => q.get("name") === "*.log");
    await page.locator("#f-name").fill("*.mp4");
    await page.locator("#f-name").press("Enter");
    const late = await first.arrived;
    await page.locator("#f-name").fill("*.log");
    await page.locator("#f-name").press("Enter");
    await second.arrived;
    await first.release();
    // the page has the first answer in full before the checks below; with the abort working, it gave up on it at once
    if (lost) await (await late.response())?.finished();
    await expect(page.locator("main")).toHaveAttribute("aria-busy", "true");
    await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
    await second.release();
    await expect(page.locator("main")).toHaveAttribute("aria-busy", "false");
    await expect(rows(page)).toHaveCount(1);
    await expect(rows(page)).toContainText("big.log");
  });
}
