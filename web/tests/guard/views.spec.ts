// refactor-guard's pixel-diff (D16, D23): every view and state the e2e mock reaches, at 1280, 700 and 360 px.
// playwright.guard.config.ts keeps the shots in $GUARD_SHOTS: a run with --update-snapshots=all takes them from one
// build, and the files of two builds (base and head) must be byte-identical. Next to each PNG goes what the page asked
// of the API on the way (`<state>-<width>-requests.txt`), so a build that asks for other rows, drops a parameter or
// asks twice differs too, though it draws the same picture.
// Each state waits for what marks it, so a build that never reaches a state fails here, not in the picture.
import { expect, type Page, test } from "@playwright/test";
import { DAY, dir, file, G, M, MockApi, NOW, url } from "../e2e/mock";
import { ShotApi } from "../shots/home";

type State = { name: string; api?: () => MockApi; go: (page: Page, api: MockApi) => Promise<unknown> };

const rows = (page: Page) => page.locator("table.grid tbody tr");
const selected = (page: Page) => page.locator('table.grid tbody tr[aria-selected="true"]');
const heading = (page: Page) => page.getByRole("heading", { level: 1 });
const notice = (page: Page) => page.locator(".notice").filter({ hasText: "newer snapshot" });

async function open(page: Page, hash?: string) {
  await page.goto(url(hash));
  await expect(rows(page).first()).toBeVisible();
}

/**
 * From here no timer the page sets runs. In real time a toast hides after 6 s, and the page polls every 5 s while it has
 * no snapshot and every 10 s once the session ended; a slow runner may take longer than that to reach the shot, which
 * would then lose the toast or record one poll more. pauseAt only goes forward, and it runs the timers due on the way:
 * 5 s past NOW, before a load or after one (whose next poll is 60 s away), none is due yet. How often the page polls
 * is for tests/e2e/states.spec.ts to pin, on every PR: here no state moves the clock on to the next poll.
 */
async function stopTimers(page: Page) {
  await page.clock.install({ time: NOW * 1000 });
  await page.clock.pauseAt(NOW * 1000 + 5000);
}

/** The API calls the page made, one per line: each query in name order, and the lines sorted, as answers may race. */
const asked = (api: MockApi) => `${api.requests.map(inOrder).sort().join("\n")}\n`;

function inOrder(request: string) {
  const u = new URL(request, "http://mock");
  u.searchParams.sort();
  return u.pathname + u.search;
}

/** A nightly scan publishes snap-2; the page hears of it on the next focus poll. */
async function newerSnapshot(page: Page, api: MockApi) {
  await open(page);
  api.snapshotId = "snap-2";
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
}

function withChanges(): MockApi {
  const api = new MockApi();
  api.prevTree = structuredClone(api.tree);
  const tass = api.tree.children![0]!.children![0]!;
  tass.children = tass.children!.filter((c) => c.name !== "old");
  tass.children.push({ name: "new.bin", kind: "file", allocated: 2 ** 30 });
  return api;
}

function withStatus(set: (api: MockApi) => void): () => MockApi {
  return () => {
    const api = new MockApi();
    set(api);
    return api;
  };
}

const tassDir = (api: MockApi) => api.tree.children![0]!.children![0]!;

/** part-01.bin to part-NN.bin in /home/tass, each a little smaller and older than the one before. */
function addParts(api: MockApi, count: number) {
  for (let i = 1; i <= count; i++) {
    const name = `part-${String(i).padStart(2, "0")}.bin`;
    tassDir(api).children!.push(file(name, (400 - 5 * i) * M, NOW - i * DAY));
  }
}

/** The mock models no mounts: this folder reads as a separate filesystem, as `freespace ls` reports one. */
class MountApi extends MockApi {
  constructor() {
    super();
    tassDir(this).children!.push(dir("backup", []));
  }

  override entry(f: Parameters<MockApi["entry"]>[0]) {
    const e = super.entry(f);
    if (f.path !== "/home/tass/backup") return e;
    return Object.assign(e, {
      kind: "mount",
      mount: { fs_type: "ext4", used: 412 * G, total: 1024 * G, root: f.path },
    });
  }
}

const STATES: State[] = [
  { name: "browse", go: (p) => open(p) },
  {
    name: "browse-unreadable-paths",
    go: async (p) => {
      await open(p);
      await p.getByRole("alert").getByRole("button", { name: "Show unreadable paths" }).click();
      await expect(p.getByRole("alert")).toContainText("/home/tass/projects/secret");
    },
  },
  { name: "browse-complete", api: () => new MockApi(false), go: (p) => open(p) },
  { name: "browse-many", api: () => new ShotApi(), go: (p) => open(p, "/home/alex") },
  {
    // 40 children and the page asks for 30: the list stops and offers the rest
    name: "browse-truncated",
    api: withStatus((a) => addParts(a, 35)),
    go: async (p) => {
      await open(p);
      await expect(rows(p)).toHaveCount(30);
      // the list scrolls inside the page: the shot shows its end, where the button is
      await p.locator(".more").scrollIntoViewIfNeeded();
      await expect(p.locator(".more")).toContainText("Showing 30 of 40");
      await expect(p.getByRole("button", { name: "Show more" })).toBeInViewport();
    },
  },
  { name: "folder", go: (p) => open(p, "/home/tass/Downloads") },
  {
    name: "empty-folder",
    api: withStatus((a) => tassDir(a).children!.push(dir("empty", []))),
    go: async (p) => {
      await p.goto(url("/home/tass/empty"));
      await expect(p.locator(".bodystate")).toContainText("Nothing here in this snapshot.");
    },
  },
  {
    // the one file in the folder is hidden
    name: "hidden-all",
    go: async (p) => {
      await open(p, "/home/tass/old");
      await p.keyboard.press("h");
      await expect(p.locator(".toast")).toBeVisible();
      // hiding lasts for the snapshot; the reload drops the toast
      await p.reload();
      await expect(p.locator(".bodystate")).toContainText("Everything here is hidden until the next scan.");
    },
  },
  {
    // the badge in the row; the details panel, on wide screens, names the filesystem
    name: "mount-row",
    api: () => new MountApi(),
    go: async (p) => {
      await open(p, "/home/tass?sel=/home/tass/backup");
      await expect(selected(p)).toContainText("mount");
    },
  },
  {
    name: "mount-folder",
    api: () => new MountApi(),
    go: async (p) => {
      await p.goto(url("/home/tass/backup"));
      await expect(p.locator(".bodystate")).toContainText("Not scanned here: separate filesystem (ext4, used 412 G of 1.0 T).");
      await expect(p.getByRole("button", { name: "Open this filesystem" })).toBeVisible();
    },
  },
  {
    name: "row-selected",
    go: async (p) => {
      await open(p);
      await p.keyboard.press("ArrowDown");
      await p.keyboard.press("ArrowDown");
      await expect(selected(p)).toContainText("old");
    },
  },
  {
    // under 700 px the inspector opens as a sheet
    name: "row-clicked",
    go: async (p) => {
      await open(p);
      await rows(p).nth(1).click();
      await expect(selected(p)).toContainText("Downloads");
    },
  },
  {
    // the Modified column is hidden under 700 px, so the address asks for the sort
    name: "sorted-by-modified",
    go: async (p) => {
      await open(p, "/home/tass?sort=mtime");
      await expect(p.locator('th[aria-sort="ascending"]')).toHaveCount(1);
    },
  },
  {
    name: "hidden",
    go: async (p) => {
      await open(p);
      await stopTimers(p);
      await p.keyboard.press("h");
      await expect(p.locator(".crumbs .hid")).toHaveText(/^1 hidden · 83 G$/);
      await expect(p.locator(".toast")).toBeVisible();
    },
  },
  {
    name: "hidden-listed",
    go: async (p) => {
      await open(p);
      await p.keyboard.press("h");
      await expect(p.locator(".crumbs .hid")).toBeVisible();
      // hiding lasts for the snapshot; the reload drops the toast
      await p.reload();
      await p.locator(".crumbs .hid").click();
      await expect(rows(p).filter({ hasText: ".cache" })).toContainText("hidden");
    },
  },
  {
    name: "unreadable-folder",
    go: async (p) => {
      await p.goto(url("/home/tass/projects/secret"));
      await expect(p.locator(".bodystate")).toContainText("Contents unavailable");
    },
  },
  {
    name: "not-in-snapshot",
    go: async (p) => {
      await p.goto(url("/home/tass/nope/deeper"));
      await expect(p.locator(".bodystate")).toContainText("This folder is not in the snapshot.");
    },
  },
  {
    name: "top-files",
    go: async (p) => {
      await open(p);
      await p.keyboard.press("t");
      await expect(selected(p)).toContainText("model.safetensors");
    },
  },
  {
    // the biggest file under projects is 900 M, the top size menu offers up to 1 G
    name: "top-empty",
    go: async (p) => {
      await p.goto(url("/home/tass/projects?mode=top&min=1G"));
      await expect(p.locator(".bodystate")).toContainText("Nothing this large within /home/tass/projects.");
    },
  },
  {
    name: "top-folders",
    go: async (p) => {
      await open(p);
      await p.keyboard.press("t");
      await p.getByRole("radio", { name: "Folders" }).click();
      await expect(p.locator(".summary")).toContainText("untouched for 6 months");
    },
  },
  {
    name: "changes",
    api: () => new ShotApi(),
    go: async (p) => {
      await open(p, "/home/alex");
      await p.keyboard.press("d");
      await expect(rows(p).filter({ hasText: ".ollama" })).toBeVisible();
    },
  },
  {
    name: "changes-gone-and-new",
    api: withChanges,
    go: async (p) => {
      await open(p);
      await p.keyboard.press("d");
      await expect(rows(p).filter({ hasText: "new.bin" })).toContainText("new");
    },
  },
  {
    // the previous scan saw the same tree
    name: "changes-empty",
    api: withStatus((a) => {
      a.prevTree = structuredClone(a.tree);
    }),
    go: async (p) => {
      await p.goto(url("/home/tass?mode=changes"));
      await expect(p.locator(".bodystate")).toContainText("Nothing changed here since the previous scan.");
    },
  },
  {
    name: "changes-unavailable",
    go: async (p) => {
      await open(p);
      await stopTimers(p);
      await p.keyboard.press("d");
      await expect(p.locator(".toast")).toHaveText("No previous snapshot to compare yet.");
    },
  },
  {
    name: "find-form",
    go: async (p) => {
      await open(p);
      await p.keyboard.press("/");
      // a base from before the fix in app.ts setMode may lose this focus to a race; the shot needs it either way
      await p.locator("#f-name").focus();
      await expect(p.locator("#f-name")).toBeFocused();
    },
  },
  {
    name: "find-results",
    go: async (p) => {
      await p.goto(url("/home/tass?mode=find&name=*.iso&run=1"));
      await expect(rows(p)).toHaveCount(1);
    },
  },
  {
    name: "find-no-matches",
    go: async (p) => {
      await p.goto(url("/home/tass?mode=find&name=*.nothing&run=1"));
      await expect(p.locator(".bodystate")).toContainText("No matches within /home/tass.");
    },
  },
  {
    name: "find-filters",
    go: async (p) => {
      await p.goto(url("/home/tass?mode=find&name=*.mp4&min=1G&run=1"));
      await expect(p.getByRole("button", { name: "Remove filter name: *.mp4" })).toBeVisible();
    },
  },
  {
    name: "find-invalid",
    go: async (p) => {
      await p.goto(url("/home/tass?mode=find"));
      await p.locator("#f-min").fill("lots");
      await p.locator("#f-before").fill("last week");
      await p.getByRole("button", { name: "Find", exact: true }).click();
      await expect(p.locator("#f-before")).toBeFocused();
    },
  },
  {
    name: "help",
    go: async (p) => {
      await open(p);
      await p.keyboard.press("?");
      await expect(p.getByRole("dialog")).toContainText("hide until next scan");
    },
  },
  {
    name: "newer-snapshot",
    api: () => new MockApi(false),
    go: async (p, api) => {
      await newerSnapshot(p, api);
      await expect(notice(p)).toBeVisible();
    },
  },
  {
    name: "notice-waiting",
    go: async (p, api) => {
      await newerSnapshot(p, api);
      await expect(p.getByRole("button", { name: "+1 notice" })).toBeVisible();
    },
  },
  {
    // a complete scan, so the error is the one notice shown in full
    name: "notice-error",
    api: () => new MockApi(false),
    go: async (p, api) => {
      await open(p);
      api.failWith = { status: 500, code: "INTERNAL", message: "database is locked" };
      await p.evaluate(() => window.dispatchEvent(new Event("focus")));
      await expect(p.locator(".notice")).toContainText("freespace could not answer: database is locked");
    },
  },
  {
    name: "first-run",
    api: withStatus((a) => {
      a.noSnapshot = true;
      a.lastError = { ts: NOW, op: "scan", message: "disk full" };
    }),
    go: async (p) => {
      await stopTimers(p);
      await p.goto(url());
      await expect(heading(p)).toHaveText("No snapshot yet");
    },
  },
  {
    name: "first-scan",
    api: withStatus((a) => {
      a.noSnapshot = true;
      a.scanning = { op: "scan", path: "/", pid: 42, since: NOW, elapsed_seconds: 48 };
    }),
    go: async (p) => {
      await stopTimers(p);
      await p.goto(url());
      await expect(p.locator("#fp-scan")).toContainText("Scanning / · 48 s");
    },
  },
  {
    name: "session-ended",
    go: async (p) => {
      await stopTimers(p);
      await p.goto("/#/home/tass");
      await expect(heading(p)).toHaveText("Session ended");
    },
  },
  {
    name: "server-gone",
    go: async (p, api) => {
      await open(p);
      await stopTimers(p);
      api.sessionGone = true;
      await p.keyboard.press("ArrowDown");
      await p.keyboard.press("Enter");
      await expect(p.locator("main")).toContainText("It will reopen at /home/tass/Downloads");
    },
  },
  {
    name: "snapshot-unreadable",
    api: withStatus((a) => {
      a.failWith = {
        status: 403,
        code: "SNAPSHOT_ACCESS",
        message: "cannot read /var/lib/freespace/snapshot.db",
        fix: "sudo freespace install-timer",
      };
    }),
    go: async (p) => {
      await p.goto(url());
      await expect(heading(p)).toHaveText("Cannot read the snapshot");
    },
  },
  {
    name: "snapshot-error",
    api: withStatus((a) => {
      a.failWith = { status: 500, code: "INTERNAL", message: "database is locked" };
    }),
    go: async (p) => {
      await p.goto(url());
      await expect(heading(p)).toHaveText("freespace cannot show this snapshot");
    },
  },
];

for (const [width, height] of [
  [1280, 800],
  [700, 900],
  [360, 740],
] as const) {
  test.describe(`${width} px`, () => {
    test.use({ viewport: { width, height } });
    for (const s of STATES) {
      test(s.name, async ({ page }) => {
        // the ages on the page read the same on any day
        await page.clock.setFixedTime(new Date(NOW * 1000));
        const api = await (s.api?.() ?? new MockApi()).install(page);
        await s.go(page, api);
        // the same frame every run: a click leaves the pointer over a row the layout then moves, and Chromium updates
        // :hover late; the inspector's Biggest inside and a slow answer finish loading (aria-busy) after the state's mark
        await page.mouse.move(0, 0);
        await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
        // soft: a run against another build's files reports both differences, the picture and the requests
        await expect.soft(page).toHaveScreenshot(`${s.name}-${width}.png`, { fullPage: true });
        expect(asked(api)).toMatchSnapshot(`${s.name}-${width}-requests.txt`);
      });
    }
  });
}
