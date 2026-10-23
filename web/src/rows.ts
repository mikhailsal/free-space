// One row shape for all four modes; the table and the inspector read only this.

import type { DiffEntry, DiffStatus, Entry, FindEntry, Kind, MountOut } from "./types";

export interface Row {
  path: string;
  raw?: string;
  /** Shown name: the entry name, or the path relative to the scope (Find). */
  label: string;
  name: string;
  /** Second line (Top: parent path relative to the scope). */
  sub?: string;
  kind: Kind;
  allocated: number;
  apparent: number;
  mtime: number;
  newest: number;
  items: number;
  nlink: number;
  errors: number;
  denied: boolean;
  alias: boolean;
  owner?: string;
  mount?: MountOut;
  /** Find: inside another match, counted in its total. */
  nested?: boolean;
  status?: DiffStatus;
  delta?: number;
  deltaItems?: number;
  allocatedPrev?: number;
  /** Hidden until the next scan, shown dimmed while "show hidden" is on. */
  hidden: boolean;
}

export function fromEntry(e: Entry | FindEntry, label = e.name, sub?: string): Row {
  return {
    path: e.path,
    raw: e.path_raw,
    label,
    name: e.name,
    sub,
    kind: e.kind,
    allocated: e.allocated,
    apparent: e.apparent,
    mtime: e.mtime,
    newest: e.newest_mtime,
    items: e.items,
    nlink: e.nlink,
    errors: e.errors,
    denied: !!e.denied,
    alias: !!e.hardlink_alias,
    owner: e.hardlink_owner,
    mount: e.mount,
    nested: "nested" in e ? e.nested : undefined,
    hidden: false,
  };
}

export function fromDiff(e: DiffEntry): Row {
  return {
    path: e.path,
    raw: e.path_raw,
    label: e.name,
    name: e.name,
    kind: e.kind,
    allocated: e.allocated,
    apparent: e.allocated,
    mtime: e.mtime,
    newest: e.newest_mtime,
    items: e.items,
    nlink: 1,
    errors: 0,
    denied: false,
    alias: false,
    status: e.status,
    delta: e.delta,
    deltaItems: e.items - e.items_prev,
    allocatedPrev: e.allocated_prev,
    hidden: false,
  };
}

/** Sizes under this row are lower bounds (D5: "?" marker and "≥"). */
export function lowerBound(r: Row): boolean {
  return r.denied || r.errors > 0;
}

export function isFolder(r: Row): boolean {
  return r.kind === "dir" || r.kind === "mount";
}
