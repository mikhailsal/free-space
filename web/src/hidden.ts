// "Hide until next scan" (D11): full paths in localStorage under
// freespace.hidden.<host>.<snapshot_id>; other keys are purged on load, so a
// new snapshot clears the list. Nothing goes to the server.

import { isUnder, parentOf } from "./router";

export interface HiddenItem {
  path: string;
  allocated: number;
}

const PREFIX = "freespace.hidden.";

export class Hidden {
  private key = "";
  private items: HiddenItem[] = [];

  load(host: string, snapshotId: string): void {
    this.key = `${PREFIX}${host}.${snapshotId}`;
    this.items = [];
    try {
      for (let i = localStorage.length - 1; i >= 0; i--) {
        const k = localStorage.key(i);
        if (k?.startsWith(PREFIX) && k !== this.key) localStorage.removeItem(k);
      }
      const raw = localStorage.getItem(this.key);
      const parsed: unknown = raw ? JSON.parse(raw) : [];
      if (Array.isArray(parsed)) {
        this.items = parsed.filter((x): x is HiddenItem => typeof x?.path === "string" && typeof x?.allocated === "number");
      }
    } catch {
      this.items = [];
    }
  }

  private save(): void {
    try {
      localStorage.setItem(this.key, JSON.stringify(this.items));
    } catch {
      // storage full or blocked: hiding lasts for this page only
    }
  }

  get all(): readonly HiddenItem[] {
    return this.items;
  }

  /** The path or one of its ancestors is hidden. */
  has(path: string): boolean {
    return this.items.some((i) => isUnder(path, i.path));
  }

  add(item: HiddenItem): void {
    if (this.items.some((i) => i.path === item.path)) return;
    this.items.push(item);
    this.save();
  }

  remove(path: string): void {
    this.items = this.items.filter((i) => i.path !== path);
    this.save();
  }

  /** Hidden direct children of a folder: the folder-row chip. */
  under(folder: string): { count: number; allocated: number } {
    let count = 0;
    let allocated = 0;
    for (const i of this.items) {
      if (parentOf(i.path) === folder) {
        count++;
        allocated += i.allocated;
      }
    }
    return { count, allocated };
  }
}
