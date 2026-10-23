import { expect, test } from "@playwright/test";
import { MockApi, url } from "./mock";

const rows = (page: import("@playwright/test").Page) => page.locator("table.grid tbody tr");
const selected = (page: import("@playwright/test").Page) => page.locator('table.grid tbody tr[aria-selected="true"]');

test("first row is selected and the inspector is filled on load (D6)", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url());
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  await expect(rows(page).first()).toHaveAttribute("aria-selected", "true");
  await expect(selected(page)).toContainText(".cache");
  await expect(page.locator("aside .title")).toHaveText(".cache");
  await expect(page.locator("aside .cmdline code")).toHaveText("freespace ls /home/tass/.cache --json");
  await expect(page.locator("aside .minirow").first()).toContainText("huggingface");
  await expect(page.locator(".identity")).toContainText("tass-vm");
  await expect(page.locator(".identity .fs")).toContainText("97% used");
  // folder total is a lower bound: projects/secret could not be read
  await expect(page.locator(".crumbs .size")).toHaveText(/^≥ /);
});

test("keyboard moves, opens and goes up, keeping the place (Pass 6)", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url());
  await expect(selected(page)).toContainText(".cache");
  await page.keyboard.press("ArrowDown");
  await expect(selected(page)).toContainText("Downloads");
  await expect(selected(page)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator(".crumbs h1")).toHaveText("Downloads");
  await expect(page).toHaveURL(/#\/home\/tass\/Downloads\?/);
  await expect(selected(page)).toContainText("ubuntu.iso");
  await page.keyboard.press("End");
  await expect(selected(page)).toContainText("notes.txt");
  await page.keyboard.press("ArrowLeft");
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  await expect(selected(page)).toContainText("Downloads");
  await page.goBack();
  await expect(page.locator(".crumbs h1")).toHaveText("Downloads");
});

test("sorting by Modified asks the API and marks the header", async ({ page }) => {
  const api = await new MockApi().install(page);
  await page.goto(url());
  await page.getByRole("button", { name: /Modified/ }).click();
  await expect(page.locator('th[aria-sort="ascending"]')).toContainText("Modified");
  expect(api.requests.some((r) => r.includes("sort=mtime"))).toBe(true);
  await expect(page).toHaveURL(/sort=mtime/);
});

test("hide until next scan, undo, and it lasts for this snapshot only (D11)", async ({ page }) => {
  const api = await new MockApi().install(page);
  await page.goto(url());
  await expect(selected(page)).toContainText(".cache");
  await page.keyboard.press("h");
  await expect(rows(page).filter({ hasText: ".cache" })).toHaveCount(0);
  await expect(page.locator(".crumbs .hid")).toHaveText(/^1 hidden · 83 G$/);
  await expect(selected(page)).toContainText("Downloads");
  await page.locator(".toast").getByRole("button", { name: "Undo" }).click();
  await expect(rows(page).filter({ hasText: ".cache" })).toHaveCount(1);
  await page.keyboard.press("h");
  await page.reload();
  await expect(page.locator(".crumbs .hid")).toBeVisible();
  await page.locator(".crumbs .hid").click();
  await expect(rows(page).filter({ hasText: ".cache" })).toContainText("hidden");
  // the next snapshot starts with nothing hidden
  api.snapshotId = "snap-2";
  await page.reload();
  await expect(rows(page).filter({ hasText: ".cache" })).toHaveCount(1);
  await expect(page.locator(".crumbs .hid")).toHaveCount(0);
});

test("unreadable folders raise the red notice before any number (D13)", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url());
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("Scan incomplete: 1 directory could not be read as root");
  await alert.getByRole("button", { name: "Show unreadable paths" }).click();
  await expect(alert).toContainText("/home/tass/projects/secret");
  await page.goto(url("/home/tass/projects/secret"));
  await expect(page.locator(".bodystate")).toContainText("Contents unavailable");
});

test("Top files and Top folders follow the controls (D17, D18)", async ({ page }) => {
  const api = await new MockApi().install(page);
  await page.goto(url());
  await page.keyboard.press("t");
  await expect(page.getByRole("tab", { name: "Top files" })).toHaveAttribute("aria-selected", "true");
  await expect(selected(page)).toContainText("model.safetensors");
  await expect(selected(page)).toContainText(".cache/huggingface/");
  await expect(page.locator(".summary")).toContainText("files larger than 100 M");
  await page.getByRole("radio", { name: "Folders" }).click();
  await expect(page.locator(".summary")).toContainText("untouched for 6 months");
  await expect(page.locator("aside .cmdline code")).toHaveText(
    "freespace top /home/tass --kind dir --min-size 100M --older-than 180d --json",
  );
  expect(api.requests.some((r) => r.includes("kind=dir") && r.includes("older_than=180d"))).toBe(true);
  await page.getByRole("button", { name: "Show in folder" }).click();
  await expect(page.getByRole("tab", { name: "Browse" })).toHaveAttribute("aria-selected", "true");
});

test("Changes is disabled without a previous snapshot", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url());
  const tab = page.getByRole("tab", { name: "Changes" });
  await expect(tab).toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("d");
  await expect(page.locator(".toast")).toHaveText("No previous snapshot to compare yet.");
  await expect(page.getByRole("tab", { name: "Browse" })).toHaveAttribute("aria-selected", "true");
});

test("Changes lists growth and removals with glyphs, not color alone (D8)", async ({ page }) => {
  const api = new MockApi();
  api.prevTree = structuredClone(api.tree);
  const tass = api.tree.children![0]!.children![0]!;
  tass.children = tass.children!.filter((c) => c.name !== "old");
  tass.children.push({ name: "new.bin", kind: "file", allocated: 2 ** 30 });
  await api.install(page);
  await page.goto(url());
  await page.keyboard.press("d");
  await expect(page.locator(".summary .context")).toContainText("depth 1 · files included");
  await expect(rows(page)).toHaveCount(2);
  await expect(rows(page).filter({ hasText: "old" })).toContainText("▼");
  await expect(rows(page).filter({ hasText: "old" })).toContainText("gone");
  await expect(rows(page).filter({ hasText: "new.bin" })).toContainText("▲");
  await expect(rows(page).filter({ hasText: "new.bin" })).toContainText("new");
});
