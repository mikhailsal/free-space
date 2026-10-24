// Changes (D6): what grew, shrank, appeared or went since the previous scan,
// one level deep, files included.

import * as api from "./api";
import { h } from "./dom";
import * as fmt from "./format";
import { DEFAULT_LIMIT, folderEntry, pathParams, sameSnapshot, stateBox, upAction, type ViewController } from "./page";
import { fromDiff } from "./rows";
import type { DiffResponse } from "./types";

function diffContext(d: DiffResponse): HTMLElement {
  return h(
    "div",
    { class: "context" },
    h(
      "p",
      null,
      `vs ${fmt.dateTime(d.prev_scanned_at)} (gen ${d.prev_generation ?? "?"}) · depth 1 · files included · `,
      h("span", {
        class: d.delta > 0 ? "grow" : d.delta < 0 ? "shrink" : "",
        text: `${d.delta > 0 ? "▲ " : d.delta < 0 ? "▼ " : ""}${fmt.delta(d.delta)}`,
      }),
      " here",
    ),
    d.partial
      ? h("span", {
          class: "badge warn",
          title: d.refreshed.map((x) => `${x.path} at ${fmt.dateTime(x.scanned_at)}`).join("\n"),
          text: `partial: ${d.refreshed.map((x) => x.path).join(", ")} rescanned after prev`,
        })
      : null,
    d.roots_changed
      ? h("span", { class: "badge warn", title: "The scanned roots differ between the two scans", text: "roots changed" })
      : null,
  );
}

export const changes: ViewController = {
  async fetch(r, sid, signal) {
    const [folder, diff] = await Promise.all([
      folderEntry(r, sid, signal),
      api.get<DiffResponse>("diff", { ...pathParams(r), depth: 1, kind: "any", limit: r.limit ?? DEFAULT_LIMIT }, signal),
    ]);
    sameSnapshot(diff.snapshot_id, sid);
    return { route: r, folder, diff };
  },

  // Changes lists what changed; unchanged children are the API's tail (|Δ| = 0)
  rows: (v) => (v.diff?.entries ?? []).filter((e) => e.status !== "same").map(fromDiff),

  summary: (v) => (v.diff ? [diffContext(v.diff)] : []),

  form: () => null,

  empty: (v, app) => stateBox("Nothing changed here since the previous scan.", upAction(app, v.folder.path)),
};
