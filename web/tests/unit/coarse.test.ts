import { expect, test } from "bun:test";
import { findHint, FOLDED_LABEL, foldedBelowOf, foldedNote, noticeText, sizeBytes, topHint } from "../../src/coarse";
import type { View } from "../../src/page";
import { fromEntry } from "../../src/rows";
import type { Entry } from "../../src/types";

const M = 2 ** 20;

const entry = (over: Partial<Entry> = {}): Entry => ({
  path: "/usr/lib/[small items 0-1071]",
  name: "[small items 0-1071]",
  kind: "other",
  allocated: 631 * M,
  apparent: 600 * M,
  mtime: 0,
  newest_mtime: 0,
  nlink: 1,
  items: 3068,
  errors: 0,
  scanned_at: 0,
  folded: true,
  ...over,
});

/** A view holding the given answers (only their headers matter here) or fields. */
const view = (parts: Record<string, unknown>): View => ({ route: {}, folder: entry(), ...parts }) as unknown as View;

// Value: protects=a view's threshold is the number its answer reports, and null (no notice) for a full snapshot, whose answers leave
//   the field out; fails_when=a full snapshot reads as coarse, or Find's first screen, which holds no answer of its own, loses the
//   threshold its folder's answer reported; why_new=the page read no coarse field before; seam=none;
//   no_claim=the loader's use of the result (the e2e tests cover it)
test("the threshold of a view: a number, or null for a full snapshot", () => {
  expect(foldedBelowOf(view({ ls: { folded_below: 300 * M } }))).toBe(300 * M);
  expect(foldedBelowOf(view({ ls: {} }))).toBeNull();
  expect(foldedBelowOf(view({ ls: { folded_below: null } }))).toBeNull();
  expect(foldedBelowOf(view({}))).toBeNull();
});

test("Find's first screen reports the threshold of its folder's answer", () => {
  expect(foldedBelowOf(view({ idle: true, foldedBelow: 300 * M }))).toBe(300 * M);
  expect(foldedBelowOf(view({ idle: true, foldedBelow: null }))).toBeNull();
});

test("every mode's answer carries the threshold", () => {
  for (const mode of ["ls", "top", "find", "diff"]) {
    expect(foldedBelowOf(view({ [mode]: { folded_below: 5 * M } })), mode).toBe(5 * M);
  }
});

test("sizes as the API reads them: binary multiples, whole numbers", () => {
  expect(sizeBytes("100M")).toBe(100 * M);
  expect(sizeBytes("1G")).toBe(2 ** 30);
  expect(sizeBytes("10m")).toBe(10 * M);
  expect(sizeBytes(" 2K ")).toBe(2048);
  expect(sizeBytes("512")).toBe(512);
  expect(sizeBytes("1.5G")).toBeNull();
  expect(sizeBytes("big")).toBeNull();
  expect(sizeBytes("")).toBeNull();
});

test("the notice names the threshold and what it limits", () => {
  const text = noticeText(452 * M);
  expect(text).toContain("smaller than 452 M");
  expect(text).toContain(`“${FOLDED_LABEL}”`);
  expect(text).toContain("Top files, Find and Changes");
});

test("a folded row's note counts its entries and says the total is exact", () => {
  expect(foldedNote(3068, 452 * M)).toBe(
    "3\u202f068 entries smaller than 452 M, folded into one row. The total is exact; the names are not in the snapshot.",
  );
  expect(foldedNote(1, null)).toBe("1 entry, folded into one row. The total is exact; the names are not in the snapshot.");
});

test("Top files hints only when the chosen size is below the threshold", () => {
  expect(topHint(300 * M, "100M")).toBe("Coarse snapshot: entries smaller than 300 M are folded and not listed.");
  expect(topHint(300 * M, "300M")).toBeNull();
  expect(topHint(300 * M, "1G")).toBeNull();
  // a size this reader cannot parse is not assumed to be large
  expect(topHint(300 * M, "1.5G")).not.toBeNull();
  expect(topHint(null, "100M")).toBeNull();
  expect(topHint(undefined, "100M")).toBeNull();
});

test("Find hints on a coarse snapshot only", () => {
  expect(findHint(300 * M)).toBe("Coarse snapshot: entries smaller than 300 M are folded, so matches among them are not found.");
  expect(findHint(null)).toBeNull();
  expect(findHint(undefined)).toBeNull();
});

test('a folded entry becomes a muted "small items" row that keeps its own name and its count', () => {
  const r = fromEntry(entry());
  expect(r.folded).toBe(true);
  expect(r.label).toBe("small items");
  expect(r.name).toBe("[small items 0-1071]");
  expect(r.items).toBe(3068);
});

test("an ordinary entry keeps its name or the label it is given", () => {
  const e = entry({ folded: undefined, name: "lib", path: "/usr/lib", kind: "dir" });
  expect(fromEntry(e).label).toBe("lib");
  expect(fromEntry(e).folded).toBe(false);
  expect(fromEntry(e, "lib/x", "./").label).toBe("lib/x");
});
