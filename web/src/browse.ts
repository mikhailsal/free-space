// Browse (D6): the folder's children, by size, count or date.

import * as api from "./api";
import { SCAN_COMMAND } from "./cli";
import * as fmt from "./format";
import { DEFAULT_LIMIT, pathParams, stateBox, upAction, type ViewController } from "./page";
import { fromEntry } from "./rows";
import { copy } from "./toast";
import type { LsResponse } from "./types";

export const browse: ViewController = {
  async fetch(r, sid, signal) {
    const ls = await api.get<LsResponse>("ls", { ...pathParams(r), sort: r.sort, limit: r.limit ?? DEFAULT_LIMIT, snapshot: sid }, signal);
    return { route: r, folder: ls.entry, ls };
  },

  rows: (v) => (v.ls?.entries ?? []).map((e) => fromEntry(e)),

  summary: () => [],

  form: () => null,

  empty(v, app) {
    const f = v.folder;
    if (f.kind === "mount" || f.mount) {
      const m = f.mount;
      const used = m && m.used !== null && m.total !== null ? `, used ${fmt.size(m.used)} of ${fmt.size(m.total)}` : "";
      const root = m?.root;
      return stateBox(
        `Not scanned here: separate filesystem (${m?.fs_type ?? "unknown"}${used}).`,
        root ? { label: "Open this filesystem", run: () => app.cmd.navigate({ path: root, sel: undefined }) } : null,
      );
    }
    if (f.denied) {
      return stateBox("Contents unavailable: this folder could not be read as root.", {
        label: `Copy: ${SCAN_COMMAND}`,
        run: () => void copy(SCAN_COMMAND, "Command copied"),
      });
    }
    return stateBox("Nothing here in this snapshot.", upAction(app, f.path));
  },
};
