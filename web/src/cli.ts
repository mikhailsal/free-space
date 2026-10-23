// The "CLI command" line of the inspector: mirrors the current mode and filters.

import { shellPath, shellWord } from "./format";
import type { Place, Route } from "./router";

export function months(n: number): string {
  return `${n * 30}d`;
}

/** Top's "Untouched" months: on by default for folders, off for files (D18). */
export function untouched(r: Route): number {
  if (r.untouched !== undefined) return r.untouched;
  return r.kind === "dir" ? 6 : 0;
}

export function command(r: Route, at: Place): string {
  const p = shellPath(at);
  switch (r.mode) {
    case "browse": {
      const sort = r.sort && r.sort !== "allocated" ? ` --sort ${r.sort}` : "";
      return `freespace ls ${p}${sort} --json`;
    }
    case "top": {
      const parts = [`freespace top ${p}`];
      if (r.kind === "dir") parts.push("--kind dir");
      parts.push(`--min-size ${r.min ?? "100M"}`);
      const u = untouched(r);
      if (u) parts.push(`--older-than ${months(u)}`);
      return `${parts.join(" ")} --json`;
    }
    case "find": {
      const parts = [`freespace find ${p}`];
      if (r.name) parts.push(`${r.icase ? "--iname" : "--name"} ${shellWord(r.name)}`);
      parts.push(`--kind ${r.kind ?? "any"}`);
      if (r.min) parts.push(`--min-size ${shellWord(r.min)}`);
      if (r.before) parts.push(`--older-than ${shellWord(r.before)}`);
      if (r.after) parts.push(`--newer-than ${shellWord(r.after)}`);
      return `${parts.join(" ")} --json`;
    }
    case "changes":
      return `freespace diff ${p} --kind any --json`;
  }
}

export function refreshCommand(folder: Place): string {
  return `sudo freespace refresh ${shellPath(folder)}`;
}

export const SCAN_COMMAND = "sudo freespace scan";
export const TIMER_COMMAND = "sudo freespace install-timer";
