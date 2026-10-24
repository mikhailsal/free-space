// The page's notices (D5 table): an incomplete scan, the last refresh that
// failed, an API error, a lost server and a newer snapshot. notices.ts draws
// them into the one slot.

import type { App } from "./app";
import { SCAN_COMMAND } from "./cli";
import { h } from "./dom";
import * as fmt from "./format";
import { type Action, details, type Notice, renderNotices } from "./notices";
import { now } from "./page";
import { copy } from "./toast";
import type { Status } from "./types";

export class PageNotices {
  constructor(private app: App) {}

  render(): void {
    const app = this.app;
    if (!app.ui) return;
    renderNotices(app.ui.notices, this.list(), app.noticesExpanded, () => {
      app.noticesExpanded = !app.noticesExpanded;
      this.render();
    });
  }

  newerVisible(): boolean {
    const s = this.app.status;
    const live = this.app.latest;
    const newer = !!(s && live?.snapshot_id && live.snapshot_id !== s.snapshot_id);
    return (newer && this.app.dismissedNewer !== live!.snapshot_id) || this.app.rotated;
  }

  private list(): Notice[] {
    const s = this.app.status!;
    const live = this.app.latest ?? s;
    const scan = { label: `Copy: ${SCAN_COMMAND}`, run: () => void copy(SCAN_COMMAND, "Command copied") };
    const all = [this.incomplete(s, scan), this.lastError(s, live, scan), this.error(scan), this.failure(), this.newer(s, live)];
    return all.filter((n): n is Notice => n !== null);
  }

  private incomplete(s: Status, scan: Action): Notice | null {
    if (!s.incomplete) return null;
    const app = this.app;
    const n = s.unknown_subtrees.length;
    const count = n >= 100 ? "100+" : String(n);
    return {
      id: "incomplete",
      priority: 1,
      tone: "danger",
      body: [
        h("b", { text: "Scan incomplete:" }),
        ` ${count} ${n === 1 ? "directory" : "directories"} could not be read as root. Sizes below are lower bounds.`,
      ],
      actions: [
        {
          label: app.unreadableOpen ? "Hide unreadable paths" : "Show unreadable paths",
          run: () => {
            app.unreadableOpen = !app.unreadableOpen;
            this.render();
          },
        },
        scan,
      ],
      extra: app.unreadableOpen
        ? h(
            "ul",
            { class: "paths mono" },
            s.unknown_subtrees.map((p) => h("li", { text: p })),
          )
        : null,
    };
  }

  private lastError(s: Status, live: Status, scan: Action): Notice | null {
    const le = live.last_error;
    if (!le || (live.last_success_ts && le.ts <= live.last_success_ts)) return null;
    return {
      id: "last-error",
      priority: 2,
      tone: "danger",
      body: [
        h("b", { text: `Last ${le.op} failed ${fmt.age(now() - le.ts)} ago:` }),
        ` ${le.message}. Showing the snapshot from ${fmt.dateTime(s.scanned_at ?? 0)}.`,
      ],
      actions: [scan],
    };
  }

  private error(scan: Action): Notice | null {
    const err = this.app.errorNotice;
    if (!err) return null;
    const text =
      err.code === "SCHEMA"
        ? "Snapshot was made by another version. Rescan to compare."
        : err.code === "HASH_COLLISION"
          ? "Snapshot inconsistency detected. Rescan."
          : `freespace could not answer: ${err.message}`;
    return {
      id: "error",
      priority: 2,
      tone: "danger",
      body: [text],
      actions: [scan],
      extra: details(err.code, err.exit, err.message),
    };
  }

  private failure(): Notice | null {
    const app = this.app;
    if (!app.failure) return null;
    return {
      id: "failure",
      priority: 2,
      tone: "danger",
      body: ["Can't reach freespace. Is ", h("code", { text: "freespace serve" }), " still running?"],
      actions: [
        {
          label: "Retry",
          run: () => {
            app.failure = false;
            this.render();
            void app.poller.poll();
            void app.loader.load();
          },
        },
      ],
      dismiss: () => {
        app.failure = false;
        this.render();
      },
    };
  }

  private newer(s: Status, live: Status): Notice | null {
    if (!this.newerVisible()) return null;
    const app = this.app;
    const newer = live.snapshot_id !== s.snapshot_id;
    return {
      id: "newer",
      priority: 3,
      tone: "neutral",
      body: [
        newer
          ? `A newer snapshot is ready (${fmt.dateTime(live.scanned_at ?? 0)}). Δ will now compare to the previous scan.`
          : "A new scan just finished.",
      ],
      actions: [{ label: "Load latest · r", run: () => void app.loader.loadLatest() }],
      dismiss: () => {
        app.dismissedNewer = live.snapshot_id;
        app.rotated = false;
        this.render();
      },
    };
  }
}
