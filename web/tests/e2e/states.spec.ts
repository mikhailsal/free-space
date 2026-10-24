import { expect, type Locator, type Page, test } from "@playwright/test";
import { asks, MockApi, NOW, TOKEN, url, watchAsks } from "./mock";

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

test("a fix with a control character is not offered as a command", async ({ page }) => {
  const api = new MockApi();
  api.failWith = {
    status: 403,
    code: "SNAPSHOT_ACCESS",
    message: "cannot read /var/lib/freespace/snapshot.db",
    fix: "sudo freespace install-timer --user\x1b[8m",
  };
  await api.install(page);
  await page.goto(url());
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Cannot read the snapshot");
  await expect(page.locator(".cmdrow")).toHaveCount(0);
  await expect(page.getByText("Fix: sudo freespace install-timer")).toBeVisible();
});

test("a folder that is not in the snapshot offers the way up", async ({ page }) => {
  await new MockApi().install(page);
  await page.goto(url("/home/tass/nope/deeper"));
  await expect(page.locator(".bodystate")).toContainText("This folder is not in the snapshot.");
  await page.getByRole("button", { name: "Go to parent" }).click();
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
});

const statusAsks = async (page: Page) => (await asks(page)).filter((p) => p === "/api/status").length;

/** No poll within `every` - 1 ms of the page setting its next one, and one at `every`. */
async function pollsAfter(page: Page, every: number) {
  const asked = await statusAsks(page);
  await page.clock.runFor(every - 1);
  expect(await statusAsks(page)).toBe(asked);
  await page.clock.runFor(1);
  expect(await statusAsks(page)).toBe(asked + 1);
}

type Poll = {
  page: string;
  every: number;
  api?: () => MockApi;
  go: (page: Page) => Promise<unknown>;
  /** Changes the next answer, and gives what the page shows once it drew it: it sets the poll after in the same task. */
  next?: (page: Page, api: MockApi) => Locator;
};

// D10, D12: how often each page asks for status. The clock moves only in the steps of pollsAfter, and the page sends a
// poll in its timer's callback, so right after a step the count shows whether a poll came due within it. The session
// page draws nothing new from an answer, so there the test cannot tell when the page set the poll after next; it times
// one poll, which proves the interval, and the others time two, which proves the page keeps polling.
const POLLS: Poll[] = [
  {
    page: "no snapshot",
    every: 5_000,
    api: () => Object.assign(new MockApi(), { noSnapshot: true }),
    go: async (page) => {
      await page.goto(url());
      await expect(page.getByRole("heading", { level: 1 })).toHaveText("No snapshot yet");
    },
    next: (page, api) => {
      api.scanning = { op: "scan", path: "/", pid: 42, since: NOW, elapsed_seconds: 48 };
      return page.locator("#fp-scan");
    },
  },
  {
    page: "the session ended",
    every: 10_000,
    go: async (page) => {
      await page.goto("/#/home/tass");
      await expect(page.getByRole("heading", { level: 1 })).toHaveText("Session ended");
    },
  },
  {
    page: "a snapshot",
    every: 60_000,
    // no notice for unreadable paths, so the one for a newer snapshot shows
    api: () => new MockApi(false),
    go: async (page) => {
      await page.goto(url());
      await expect(page.locator(".crumbs h1")).toHaveText("tass");
    },
    next: (page, api) => {
      api.snapshotId = "snap-2";
      return page.locator(".notice").filter({ hasText: "newer snapshot" });
    },
  },
];

for (const p of POLLS) {
  test(`with ${p.page} the page asks for status every ${p.every / 1000} s, no sooner (D10, D12)`, async ({ page }) => {
    await watchAsks(page);
    await page.clock.install({ time: NOW * 1000 });
    await page.clock.pauseAt(NOW * 1000 + 1000);
    const api = await (p.api?.() ?? new MockApi()).install(page);
    // by the time the page shows its mark, it has set its next poll
    await p.go(page);
    const drawn = p.next?.(page, api);
    await pollsAfter(page, p.every);
    if (!drawn) return;
    await expect(drawn).toBeVisible();
    await pollsAfter(page, p.every);
  });
}
