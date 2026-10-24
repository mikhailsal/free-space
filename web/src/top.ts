// Top files (D6): the largest files or folders within the scope, larger than a
// size and, optionally, untouched for some months.

import * as api from "./api";
import type { App } from "./app";
import { months, untouched } from "./cli";
import { h } from "./dom";
import * as fmt from "./format";
import { DEFAULT_LIMIT, folderEntry, pathParams, sameSnapshot, stateBox, type ViewController } from "./page";
import { parentOf, relative, type Route } from "./router";
import { fromEntry } from "./rows";
import type { TopResponse } from "./types";

const TOP_SIZES = ["10M", "100M", "1G"];
const TOP_MONTHS = [3, 6, 12];

export function topKind(r: Route): "file" | "dir" {
  return r.kind === "dir" ? "dir" : "file";
}

function topForm(app: App): HTMLElement {
  const r = app.route;
  const kind = topKind(r);
  const u = untouched(r);
  const min = r.min ?? "100M";
  const seg = (k: "file" | "dir", label: string) =>
    h("button", {
      type: "button",
      role: "radio",
      "aria-checked": String(kind === k),
      class: kind === k ? "on" : "",
      onclick: () => app.cmd.navigate({ kind: k === "dir" ? "dir" : undefined, untouched: undefined, sel: undefined, limit: undefined }),
      text: label,
    });
  const sizeSel = h(
    "select",
    { id: "t-min", "aria-label": "Larger than" },
    TOP_SIZES.map((s) => h("option", { value: s, selected: s === min, text: s.replace(/([KMGT])$/, " $1") })),
  );
  sizeSel.addEventListener("change", () =>
    app.cmd.navigate({ min: sizeSel.value === "100M" ? undefined : sizeSel.value, sel: undefined, limit: undefined }),
  );
  const monthSel = h(
    "select",
    { id: "t-months", "aria-label": "Months untouched" },
    TOP_MONTHS.map((m) => h("option", { value: String(m), selected: m === (u || 6), text: String(m) })),
  );
  const check = h("input", { type: "checkbox", id: "t-untouched", checked: u > 0 });
  const apply = () => app.cmd.navigate({ untouched: check.checked ? Number(monthSel.value) : 0, sel: undefined, limit: undefined });
  monthSel.addEventListener("change", apply);
  check.addEventListener("change", apply);
  return h(
    "div",
    { class: "topform" },
    h("div", { class: "seg", role: "radiogroup", "aria-label": "Show" }, seg("file", "Files"), seg("dir", "Folders")),
    h(
      "p",
      null,
      `Top ${kind === "dir" ? "folders" : "files"} within `,
      h("b", { class: "mono", text: app.scope }),
      " · larger than ",
      sizeSel,
    ),
    h("label", { class: "check", title: "Not modified since: mtime is modification, not last open" }, check, " Untouched for "),
    monthSel,
    h("span", { text: " months" }),
  );
}

function topSummary(t: TopResponse, r: Route): HTMLElement {
  const kind = topKind(r);
  const noun = kind === "dir" ? (t.total_count === 1 ? "folder" : "folders") : t.total_count === 1 ? "file" : "files";
  const u = untouched(r);
  return h(
    "div",
    { class: "total" },
    h(
      "p",
      null,
      h("b", { class: "mono", text: fmt.count(t.total_count) }),
      ` ${noun} larger than ${(r.min ?? "100M").replace(/([KMGT])$/, " $1")}${u ? `, untouched for ${u} months` : ""} · `,
      h("b", { class: "mono", text: fmt.size(t.total_allocated) }),
    ),
    t.truncated ? h("p", { class: "muted", text: `Showing the largest ${fmt.count(t.entries.length)}` }) : null,
  );
}

export const top: ViewController = {
  async fetch(r, sid, signal) {
    const u = untouched(r);
    const [folder, t] = await Promise.all([
      folderEntry(r, sid, signal),
      api.get<TopResponse>(
        "top",
        {
          ...pathParams(r),
          kind: topKind(r),
          min_size: r.min ?? "100M",
          older_than: u ? months(u) : undefined,
          limit: r.limit ?? DEFAULT_LIMIT,
        },
        signal,
      ),
    ]);
    sameSnapshot(t.snapshot_id, sid);
    return { route: r, folder, top: t };
  },

  rows: (v, scope) =>
    (v.top?.entries ?? []).map((e) => {
      const parent = parentOf(e.path) ?? "/";
      const rel = parent === scope ? "./" : `${relative(parent, scope)}/`;
      return fromEntry(e, e.name, rel);
    }),

  // worded from the current route, not from the route the view was fetched for
  summary: (v, app) => (v.top ? [topSummary(v.top, app.route)] : []),

  form: topForm,

  empty: (v) => stateBox(`Nothing this large within ${v.folder.path}.`, null),
};
