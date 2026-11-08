// Coarse snapshots (`--backend coarse`, docs/coarse-snapshot.md): entries below a threshold are folded into
// `[small items A-B]` rows, where A-B is a range of hash slots and means nothing to a reader. The page says so once,
// in a notice, shows those rows as "small items" with the number of entries they hold, and tells Top files and Find
// that folded entries are not listed. The words live here as plain functions so that a unit test can read them.

import * as fmt from "./format";
import type { View } from "./page";

/** What a folded row is called on screen; its own name, hash range included, stays in the tooltip and the details panel. */
export const FOLDED_LABEL = "small items";

/** The fold threshold, in bytes, that the answer of a view reports: null for a full snapshot. */
export function foldedBelowOf(v: View): number | null {
  const head = v.ls ?? v.top ?? v.find ?? v.diff;
  return (head ? head.folded_below : v.foldedBelow) ?? null;
}

/** Bytes of a size as the API reads it (100M, 2G: binary multiples); null for anything else. */
export function sizeBytes(s: string): number | null {
  const m = /^(\d+)([KMGT]?)$/i.exec(s.trim());
  if (!m) return null;
  const unit = (m[2] ?? "").toUpperCase();
  return Number(m[1]) * 2 ** (10 * (unit ? "KMGT".indexOf(unit) + 1 : 0));
}

/** The notice's sentence, after its bold lead-in. */
export function noticeText(below: number): string {
  return ` entries smaller than ${fmt.size(below)} are folded into “${FOLDED_LABEL}” rows. Folder totals stay exact; Top files, Find and Changes see only the larger entries.`;
}

/** The details panel's note for a folded row. */
export function foldedNote(items: number, below: number | null): string {
  const n = `${fmt.count(items)} ${items === 1 ? "entry" : "entries"}`;
  const limit = below ? ` smaller than ${fmt.size(below)}` : "";
  return `${n}${limit}, folded into one row. The total is exact; the names are not in the snapshot.`;
}

/** Under Top files; null when the chosen size is at or above the threshold, as nothing the list could hold is folded then. */
export function topHint(below: number | null | undefined, min: string): string | null {
  if (!below) return null;
  const bytes = sizeBytes(min);
  if (bytes !== null && bytes >= below) return null;
  return `Coarse snapshot: entries smaller than ${fmt.size(below)} are folded and not listed.`;
}

/** Under Find: a name inside a folded entry cannot match. */
export function findHint(below: number | null | undefined): string | null {
  return below ? `Coarse snapshot: entries smaller than ${fmt.size(below)} are folded, so matches among them are not found.` : null;
}
