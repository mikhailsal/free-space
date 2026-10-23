// Full-page states (D7, D12): first run, session ended, no access.

import { SCAN_COMMAND, TIMER_COMMAND } from "./cli";
import { h } from "./dom";
import * as fmt from "./format";
import { details } from "./notices";
import { copy } from "./toast";
import type { ErrorDetail, Status } from "./types";

function cmdRow(label: string, cmd: string): HTMLElement {
  return h(
    "div",
    { class: "cmdrow" },
    h("span", { class: "muted", text: label }),
    h("code", { class: "mono", text: cmd }),
    h("button", { type: "button", class: "outline", onclick: () => copy(cmd, "Command copied"), text: "Copy" }),
  );
}

function page(title: string, ...body: (Node | null)[]): HTMLElement {
  return h(
    "main",
    { class: "fullpage", "aria-labelledby": "fp-title" },
    h("div", { class: "fp" }, h("div", { class: "brand", text: "freespace" }), h("h1", { id: "fp-title", text: title }), ...body),
  );
}

export function scanningLine(s: Status, now: number, fetchedAt: number): string {
  const sc = s.scanning;
  if (!sc) return "";
  const tick = Math.max(0, now - fetchedAt);
  const el = fmt.elapsed(sc.elapsed_seconds + tick);
  const p = s.progress;
  const what = sc.op === "refresh" ? "Refreshing" : "Scanning";
  if (p && p.entries_expected) {
    return `${what} ${sc.path} · ${fmt.bigCount(p.entries_seen)} of ~${fmt.bigCount(p.entries_expected)} files · ${p.percent ?? 0}% · ${el}`;
  }
  return `${what} ${sc.path} · ${el}`;
}

export function noSnapshot(s: Status, now: number, fetchedAt: number): HTMLElement {
  if (s.scanning) {
    return page(
      "No snapshot yet",
      h("p", { class: "lead mono", id: "fp-scan", text: scanningLine(s, now, fetchedAt) }),
      h("p", { text: "This page opens the snapshot by itself when the scan finishes." }),
    );
  }
  const le = s.last_error;
  return page(
    "No snapshot yet",
    h("p", { text: "Run this once; it takes a few minutes and you can leave the terminal." }),
    cmdRow("Run in a terminal:", `${SCAN_COMMAND} /`),
    cmdRow("Or install the nightly timer:", TIMER_COMMAND),
    le
      ? h(
          "div",
          { class: "notice danger", role: "alert" },
          h("span", { class: "mark", "aria-hidden": "true", text: "!" }),
          h("p", null, `Last ${le.op} failed ${fmt.dateTime(le.ts)}: ${le.message}`),
        )
      : null,
  );
}

export function sessionEnded(lastPath: string): HTMLElement {
  return page(
    "Session ended",
    h("p", { text: `Session ended: freespace stopped or this tab lost its key. It will reopen at ${lastPath}.` }),
    cmdRow("Start it again:", "freespace gui"),
    h("p", { class: "muted", text: "This page checks every 10 seconds and comes back by itself if the server is still running." }),
  );
}

export function noAccess(e: ErrorDetail): HTMLElement {
  return page(
    "Cannot read the snapshot",
    h("p", { text: e.message }),
    e.fix ? cmdRow("Fix:", e.fix.replace(/^ask a human to run: /, "")) : null,
    details(e.code, e.exit, e.message),
  );
}

export function fatal(e: ErrorDetail): HTMLElement {
  return page(
    "freespace cannot show this snapshot",
    h("p", { text: e.message }),
    e.fix ? cmdRow("Fix:", e.fix.replace(/^ask a human to run: /, "")) : null,
    details(e.code, e.exit, e.message),
  );
}
