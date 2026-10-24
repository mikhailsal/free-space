import { expect, type Page, test } from "@playwright/test";
import { MockApi, NOW, TOKEN, url } from "./mock";

test("first run: the commands to make a snapshot (D7)", async ({ page }) => {
  const api = new MockApi();
  api.noSnapshot = true;
  api.lastError = { ts: NOW, op: "scan", message: "disk full" };
  await api.install(page);
  await page.goto(url());
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("No snapshot yet");
  await expect(page.locator(".cmdrow").first()).toContainText("sudo freespace scan /");
  await expect(page.getByRole("alert")).toContainText("disk full");
});

test("a running first scan shows progress and the page opens itself when it ends (D12)", async ({ page }) => {
  const api = new MockApi();
  api.noSnapshot = true;
  api.scanning = { op: "scan", path: "/", pid: 42, since: NOW, elapsed_seconds: 48 };
  await page.clock.install();
  await api.install(page);
  await page.goto(url());
  await expect(page.locator("#fp-scan")).toContainText("Scanning / · 48 s");
  api.noSnapshot = false;
  api.scanning = null;
  await page.clock.runFor(5_500);
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
});

test("no token: session ended, with the command to start again", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto("/#/home/tass");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Session ended");
  await expect(page.locator(".cmdrow")).toContainText("freespace gui");
});

test("the server went away mid-session", async ({ page }) => {
  const api = await new MockApi().install(page);
  await page.goto(url());
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  api.sessionGone = true;
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Session ended");
  await expect(page.locator("main")).toContainText("It will reopen at /home/tass/Downloads");
});

test("the token moves to sessionStorage and survives a hash without it", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url());
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  expect(await page.evaluate(() => sessionStorage.getItem("freespace.token"))).toBe(TOKEN);
  // T7: gone from the address bar, history and every link on the page
  expect(page.url()).not.toContain(TOKEN);
  const hrefs = await page.locator("a[href]").evaluateAll((as) => as.map((a) => a.getAttribute("href") ?? ""));
  expect(hrefs.filter((h) => h.includes("t="))).toEqual([]);
  await page.goto("/#/home/tass/Downloads");
  await page.reload();
  await expect(page.locator(".crumbs h1")).toHaveText("Downloads");
});

test("snapshot not readable by this user (DX D8)", async ({ page }) => {
  const api = new MockApi();
  api.failWith = {
    status: 403,
    code: "SNAPSHOT_ACCESS",
    message: "cannot read /var/lib/freespace/snapshot.db",
    fix: "sudo freespace install-timer",
  };
  await api.install(page);
  await page.goto(url());
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Cannot read the snapshot");
  await expect(page.locator(".cmdrow")).toContainText("sudo freespace install-timer");
});

test("a folder that is not in the snapshot offers the way up", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url("/home/tass/nope/deeper"));
  await expect(page.locator(".bodystate")).toContainText("This folder is not in the snapshot.");
  await page.getByRole("button", { name: "Go to parent" }).click();
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
});

/** Keeps in the page each request it sends, at the moment it sends it. */
function countAsks() {
  const w = window as unknown as { asks: string[] };
  w.asks = [];
  const send = window.fetch.bind(window);
  window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    w.asks.push(String(input));
    return send(input, init);
  }) as typeof window.fetch;
}

const statusAsks = (page: Page) =>
  page.evaluate(() => (window as unknown as { asks: string[] }).asks.filter((u) => u.startsWith("/api/status")).length);

// D10, D12: how often each page asks for status. The clock moves only in the steps below, and the page sends a poll in
// its timer's callback, so right after a step the count shows whether a poll came due within it.
const POLLS: { page: string; every: number; go: (page: Page) => Promise<unknown> }[] = [
  {
    page: "no snapshot",
    every: 5_000,
    go: async (page) => {
      const api = new MockApi();
      api.noSnapshot = true;
      await api.install(page);
      await page.goto(url());
      await expect(page.getByRole("heading", { level: 1 })).toHaveText("No snapshot yet");
    },
  },
  {
    page: "the session ended",
    every: 10_000,
    go: async (page) => {
      await new MockApi().install(page);
      await page.goto("/#/home/tass");
      await expect(page.getByRole("heading", { level: 1 })).toHaveText("Session ended");
    },
  },
  {
    page: "a snapshot",
    every: 60_000,
    go: async (page) => {
      await new MockApi().install(page);
      await page.goto(url());
      await expect(page.locator(".crumbs h1")).toHaveText("tass");
    },
  },
];

for (const p of POLLS) {
  test(`with ${p.page} the page asks for status every ${p.every / 1000} s, no sooner (D10, D12)`, async ({ page }) => {
    await page.addInitScript(countAsks);
    await page.clock.install({ time: NOW * 1000 });
    await page.clock.pauseAt(NOW * 1000 + 1000);
    // by the time the page shows its mark, it has set its next poll
    await p.go(page);
    const asked = await statusAsks(page);
    await page.clock.runFor(p.every - 1);
    expect(await statusAsks(page)).toBe(asked);
    await page.clock.runFor(1);
    expect(await statusAsks(page)).toBe(asked + 1);
  });
}
