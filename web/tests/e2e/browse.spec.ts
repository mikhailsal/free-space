import { expect, test } from "@playwright/test";
import { aborted, allRead, DAY, dir, file, loseAborts, M, MockApi, NOW, url, watchAsks } from "./mock";

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
  await expect(page).toHaveURL(/#\/home\/tass\/Downloads$/);
  await expect(selected(page)).toContainText("ubuntu.iso");
  await page.keyboard.press("End");
  await expect(selected(page)).toContainText("notes.txt");
  await page.keyboard.press("ArrowLeft");
  await expect(page.locator(".crumbs h1")).toHaveText("tass");
  await expect(selected(page)).toContainText("Downloads");
  await page.goBack();
  await expect(page.locator(".crumbs h1")).toHaveText("Downloads");
});

// The keys the test above does not press. /home/tass lists .cache, Downloads, old, projects, big.log (largest first),
// then the part-NN.bin a test adds, each smaller than the one before; PageUp and PageDown move ten rows, or to the end.
const KEYS: { key: string; from: string; parts?: number; folder: string; sel: string; at: string }[] = [
  { key: "ArrowUp", from: "/home/tass?sel=/home/tass/old", folder: "tass", sel: "Downloads", at: "#/home/tass?sel=/home/tass/Downloads" },
  { key: "ArrowRight", from: "/home/tass?sel=/home/tass/Downloads", folder: "Downloads", sel: "ubuntu.iso", at: "#/home/tass/Downloads" },
  { key: "Home", from: "/home/tass?sel=/home/tass/big.log", folder: "tass", sel: ".cache", at: "#/home/tass?sel=/home/tass/.cache" },
  {
    key: "PageUp",
    from: "/home/tass?sel=/home/tass/part-15.bin",
    parts: 15,
    folder: "tass",
    sel: "part-05.bin",
    at: "#/home/tass?sel=/home/tass/part-05.bin",
  },
  { key: "PageUp", from: "/home/tass?sel=/home/tass/big.log", folder: "tass", sel: ".cache", at: "#/home/tass?sel=/home/tass/.cache" },
  { key: "PageDown", from: "/home/tass", parts: 15, folder: "tass", sel: "part-06.bin", at: "#/home/tass?sel=/home/tass/part-06.bin" },
  { key: "PageDown", from: "/home/tass", folder: "tass", sel: "big.log", at: "#/home/tass?sel=/home/tass/big.log" },
  { key: "Backspace", from: "/home/tass/Downloads", folder: "tass", sel: "Downloads", at: "#/home/tass?sel=/home/tass/Downloads" },
];

for (const k of KEYS) {
  test(`${k.key} from ${k.from} lands on ${k.sel} in ${k.folder} (Pass 6)`, async ({ page }) => {
    const api = new MockApi();
    const tass = api.kids("/home/tass");
    for (let i = 1; i <= (k.parts ?? 0); i++) tass.push(file(`part-${String(i).padStart(2, "0")}.bin`, (400 - 5 * i) * M, NOW - i * DAY));
    await api.install(page);
    await page.goto(url(k.from));
    await expect(rows(page).first()).toBeVisible();
    await page.keyboard.press(k.key);
    await expect(page.locator(".crumbs h1")).toHaveText(k.folder);
    await expect(selected(page)).toContainText(k.sel);
    await expect(page).toHaveURL((u) => u.hash === k.at);
  });
}

// The request coordinator drops a late answer twice over: it aborts the request it gave up on, and a load that is no
// longer the latest applies nothing. With the abort lost, the answer arrives and only the second keeps it off the screen,
// rows or an error alike. The checks after the answer wait until the page has read it and that load has run to its end.
for (const [lost, error] of [
  [false, false],
  [true, false],
  [true, true],
] as const) {
  test(`a Browse answer that arrives after Top was chosen leaves Top on screen${lost ? ", even with the abort lost" : ""}${error ? " and an error for an answer" : ""} (D19)`, async ({
    page,
  }) => {
    if (lost) await loseAborts(page);
    await watchAsks(page);
    const api = new MockApi();
    // the page's own listing, not the one-row ls that Top asks for the folder's size
    const browse = api.hold("ls", (q) => q.get("limit") !== "1");
    await api.install(page);
    await page.goto(url());
    const late = await browse.arrived;
    await page.keyboard.press("t");
    if (!lost) await expect.poll(() => aborted(page, late)).toBe(true);
    await expect(selected(page)).toContainText("model.safetensors");
    // the held listing answers as for a folder gone from the snapshot
    if (error) api.failWith = { status: 404, code: "PATH_NOT_FOUND", message: "/home/tass is not in the snapshot" };
    await browse.release();
    await allRead(page);
    await expect(page.getByRole("tab", { name: "Top files" })).toHaveAttribute("aria-selected", "true");
    await expect(page).toHaveURL(/mode=top/);
    await expect(page.locator(".crumbs h1")).toHaveText("tass");
    await expect(page.locator(".summary")).toContainText("files larger than 100 M");
    await expect(page.locator("table.grid")).toHaveClass(/\bmode-top\b/);
    await expect(selected(page)).toContainText("model.safetensors");
    await expect(page.getByText("This folder is not in the snapshot.")).toHaveCount(0);
  });
}

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

test("a toast stays 6 s, then goes (Pass 6)", async ({ page }) => {
  await page.clock.install({ time: NOW * 1000 });
  await page.clock.pauseAt(NOW * 1000 + 1000);
  await new MockApi().install(page);
  await page.goto(url());
  await expect(selected(page)).toContainText(".cache");
  await page.keyboard.press("d");
  const toast = page.locator(".toast");
  await expect(toast).toHaveText("No previous snapshot to compare yet.");
  await page.clock.runFor(5_999);
  await expect(toast).toBeVisible();
  await page.clock.runFor(1);
  await expect(toast).toHaveCount(0);
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

test("Changes the server cannot compare any more offer the way back to Browse", async ({ page }) => {
  const api = new MockApi();
  api.prevTree = structuredClone(api.tree);
  await api.install(page);
  await page.goto(url());
  await expect(page.getByRole("tab", { name: "Changes" })).toHaveAttribute("aria-disabled", "false");
  // the previous snapshot goes between the status that offered it and the diff
  api.prevTree = null;
  await page.keyboard.press("d");
  await expect(page.locator(".bodystate")).toContainText("No previous snapshot to compare yet.");
  await page.locator(".bodystate").getByRole("button", { name: "Browse" }).click();
  await expect(page.getByRole("tab", { name: "Browse" })).toHaveAttribute("aria-selected", "true");
  await expect(rows(page).first()).toContainText(".cache");
});

test("Changes lists growth and removals with glyphs, not color alone (D8)", async ({ page }) => {
  const api = new MockApi();
  api.prevTree = structuredClone(api.tree);
  const tass = api.node("/home/tass");
  tass.children = api.kids("/home/tass").filter((c) => c.name !== "old");
  tass.children.push({ name: "new.bin", kind: "file", allocated: 2 ** 30 });
  await api.install(page);
  await page.goto(url());
  // "d" before the status answer finds no previous snapshot yet and only shows a toast
  await expect(page.getByRole("tab", { name: "Changes" })).toHaveAttribute("aria-disabled", "false");
  await page.keyboard.press("d");
  await expect(page.locator(".summary .context")).toContainText("depth 1 · files included");
  await expect(rows(page)).toHaveCount(2);
  await expect(rows(page).filter({ hasText: "old" })).toContainText("▼");
  await expect(rows(page).filter({ hasText: "old" })).toContainText("gone");
  await expect(rows(page).filter({ hasText: "new.bin" })).toContainText("▲");
  await expect(rows(page).filter({ hasText: "new.bin" })).toContainText("new");
});

/** The mock with some entries' fields replaced, as `freespace ls` reports hardlinks and mount points. */
class PatchedApi extends MockApi {
  patch: Record<string, Record<string, unknown>> = {};

  override entry(f: Parameters<MockApi["entry"]>[0]) {
    return Object.assign(super.entry(f), this.patch[f.path]);
  }
}

/** The details panel's parts in order, after the sheet's handle and Close button. */
const parts = (page: import("@playwright/test").Page) =>
  page
    .locator("aside .panel > :not(.handle, .sheet-close)")
    .evaluateAll((els) => els.map((e) => [e.tagName.toLowerCase(), ...e.classList].join(".")));

// Value: protects=an unreadable folder's details warn that its sizes are lower bounds and offer the root scan, before its facts, with no Biggest inside;
//   fails_when=the denied note, its Copy button or their place in the panel is lost, or Biggest inside opens for a folder that cannot be read;
//   why_new=no test or guard state selects a denied row; seam=none;
//   no_claim=the scan's permission handling (the mock marks projects/secret denied; the patch gives it items, so only `denied` keeps Biggest inside shut)
test("an unreadable folder's details say its sizes are lower bounds and offer the root scan (D13)", async ({ page }) => {
  const api = new PatchedApi();
  api.patch = { "/home/tass/projects/secret": { items: 3 } };
  await api.install(page);
  await page.goto(url("/home/tass/projects?sel=/home/tass/projects/secret"));
  await expect(page.locator("aside .title")).toHaveText("secret");
  await expect(page.locator("aside .note.danger")).toHaveText("Could not be read as root; sizes below are lower bounds.");
  await expect(page.locator("aside").getByRole("button", { name: "Copy: sudo freespace scan" })).toBeVisible();
  expect(await parts(page)).toEqual(["section.head", "p.note.danger", "button.outline", "dl", "div.actions"]);
  const asked = api.requests.map((r) => new URL(r, "http://mock").searchParams.get("path"));
  expect(asked).not.toContain("/home/tass/projects/secret");
});

// Value: protects=a hardlinked file's details warn that rm may not free it, and an alias names the path its space is counted at;
//   fails_when=the nlink > 1 note is lost or moves, or an alias reads as the counted path; why_new=the mock reports every entry with nlink 1;
//   seam=none; no_claim=the scan's hardlink accounting (the mock reports the links)
for (const c of [
  { name: "movie.mp4", note: "Hardlinked (2 paths); space is counted once, here. rm here may not free it." },
  { name: "notes.txt", note: "0 · counted at /home/tass/old/notes.txt. rm here may not free it." },
]) {
  test(`a hardlinked file's details say rm may not free it: ${c.name}`, async ({ page }) => {
    const api = new PatchedApi();
    api.patch = {
      "/home/tass/Downloads/movie.mp4": { nlink: 2 },
      "/home/tass/Downloads/notes.txt": { nlink: 2, hardlink_alias: true, hardlink_owner: "/home/tass/old/notes.txt" },
    };
    await api.install(page);
    await page.goto(url(`/home/tass/Downloads?sel=/home/tass/Downloads/${c.name}`));
    await expect(page.locator("aside .title")).toHaveText(c.name);
    await expect(page.locator("aside .note")).toHaveText(c.note);
    expect(await parts(page)).toEqual(["section.head", "p.note.muted", "dl", "div.actions"]);
  });
}

// Value: protects=a mount point the snapshot does not cover names its filesystem, with no usage it lacks, no Open button and no CLI line;
//   fails_when=the panel offers "Open this filesystem" with nowhere to go, prints null usage, or adds a size line or the CLI command;
//   why_new=the guard's mount point has a root and usage; seam=none; no_claim=mount detection (the mock reports the mount)
test("a mount point the snapshot does not cover offers no way in", async ({ page }) => {
  const api = new PatchedApi();
  api.kids("/home/tass").push(dir("nas", []));
  api.patch = { "/home/tass/nas": { kind: "mount", mount: { fs_type: "nfs4", used: null, total: null, root: null } } };
  await api.install(page);
  await page.goto(url("/home/tass?sel=/home/tass/nas"));
  await expect(page.locator("aside .title")).toHaveText("nas");
  await expect(page.locator("aside .note")).toHaveText("Separate filesystem: nfs4. Not scanned here.");
  expect(await parts(page)).toEqual(["section.head", "p.note", "div.actions"]);
  await expect(page.locator("aside .head .big")).toHaveCount(0);
  await expect(page.locator("aside .cmd")).toHaveCount(0);
});
