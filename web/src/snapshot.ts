// Snapshot refresh continuity (D10): the ancestor walk after "Load latest".

import { parentOf } from "./router";

/**
 * Nearest folder at or above `path` that `exists` in the new snapshot, or null.
 * `exists` answers false for a gone path and throws for other failures.
 */
export async function nearestExisting(
  path: string,
  exists: (p: string) => Promise<boolean>,
): Promise<string | null> {
  let p: string | null = path;
  while (p !== null) {
    if (await exists(p)) return p;
    p = parentOf(p);
  }
  return null;
}

export type Continuity =
  | { kind: "same" }
  | { kind: "path-gone"; from: string; to: string }
  | { kind: "row-gone"; name: string };

/** What the toast says after reloading a view against a new snapshot. */
export function continuity(
  wanted: string,
  landed: string,
  selected: string | undefined,
  rows: readonly string[],
): Continuity {
  if (landed !== wanted) return { kind: "path-gone", from: wanted, to: landed };
  if (selected && !rows.includes(selected)) {
    const name = selected.slice(selected.lastIndexOf("/") + 1);
    return { kind: "row-gone", name };
  }
  return { kind: "same" };
}
