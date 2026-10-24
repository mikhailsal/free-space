// Per-mode table columns.

import * as fmt from "./format";
import { h } from "./dom";
import type { Mode } from "./router";
import { lowerBound, type Row } from "./rows";

export interface Column {
  key: string;
  label: string;
  cls: string;
  /** API sort key when the header is clickable (Browse). */
  sort?: string;
  title?: string;
  cell: (r: Row, ctx: CellCtx) => Node | string;
}

export interface CellCtx {
  now: number;
  folderSize: number;
}

const ge = (r: Row, s: string) => (lowerBound(r) ? `≥ ${s}` : s);

function nameCell(r: Row, sub = false): Node {
  const glyph = r.kind === "dir" ? "▸" : r.kind === "mount" ? "⌂" : r.kind === "symlink" ? "↪" : "";
  return h(
    "span",
    { class: "namecell" },
    h("span", { class: "k", "aria-hidden": "true", text: glyph }),
    h(
      "span",
      { class: "nm" },
      h("span", { class: "label", text: r.label }),
      sub && r.sub ? h("span", { class: "sub mono", text: r.sub }) : null,
    ),
    lowerBound(r) ? h("span", { class: "q", title: "Some folders inside could not be read: sizes are lower bounds", text: "?" }) : null,
    r.nlink > 1 && r.kind === "file"
      ? h("span", {
          class: "link-ic",
          title: r.alias ? `Hardlink: counted at ${r.owner ?? "another path"}` : `Hardlinked (${r.nlink} paths); counted here`,
          text: "⛓",
        })
      : null,
    r.mount ? h("span", { class: "badge", text: "mount" }) : null,
    r.status === "new" ? h("span", { class: "badge new", text: "new" }) : null,
    r.status === "gone" ? h("span", { class: "badge gone", text: "gone" }) : null,
    r.nested ? h("span", { class: "badge muted", text: "in parent total" }) : null,
    r.hidden ? h("span", { class: "badge muted", text: "hidden" }) : null,
  );
}

const allocated: Column = {
  key: "allocated",
  label: "Allocated",
  cls: "num",
  sort: "allocated",
  title: "Disk blocks used; apparent size is in the details panel",
  cell: (r) => (r.mount ? "—" : r.alias ? `0 · counted elsewhere` : ge(r, fmt.size(r.allocated))),
};

const modified: Column = {
  key: "mtime",
  label: "Modified",
  cls: "date",
  sort: "mtime",
  title: "Last modification of the entry itself",
  cell: (r, c) => fmt.ago(r.mtime, c.now),
};

const items: Column = {
  key: "items",
  label: "Items",
  cls: "num items",
  cell: (r) => (r.kind === "dir" ? ge(r, fmt.count(r.items)) : "—"),
};

export function columns(mode: Mode, topKind: string | undefined): Column[] {
  switch (mode) {
    case "browse":
      return [
        { key: "name", label: "Name", cls: "name", cell: (r) => nameCell(r) },
        allocated,
        {
          key: "share",
          label: "Share",
          cls: "num share",
          title: "Share of this folder's allocated size",
          cell: (r, c) => (r.mount ? "—" : fmt.pct(r.allocated, c.folderSize)),
        },
        items,
        modified,
      ];
    case "top":
      return topKind === "dir"
        ? [
            { key: "name", label: "Name", cls: "name", cell: (r) => nameCell(r, true) },
            { ...allocated, sort: undefined },
            items,
            {
              key: "newest",
              label: "Newest file",
              cls: "date",
              title: "Newest file inside (modification time)",
              cell: (r) => fmt.date(r.newest),
            },
          ]
        : [
            { key: "name", label: "Name", cls: "name", cell: (r) => nameCell(r, true) },
            { ...allocated, sort: undefined },
            { ...modified, sort: undefined, cell: (r) => fmt.date(r.mtime) },
          ];
    case "find":
      return [
        { key: "name", label: "Name", cls: "name", cell: (r) => nameCell(r) },
        { ...allocated, sort: undefined },
        { ...modified, sort: undefined },
        {
          key: "kind",
          label: "Kind",
          cls: "kind",
          cell: (r) => (r.kind === "dir" ? "folder" : r.kind),
        },
      ];
    case "changes":
      return [
        { key: "name", label: "Name", cls: "name", cell: (r) => nameCell(r) },
        { ...allocated, sort: undefined, cell: (r) => (r.status === "gone" ? "—" : fmt.size(r.allocated)) },
        {
          key: "delta",
          label: "Δ allocated",
          cls: "num delta",
          title: "Change since the previous scan",
          cell: (r) => {
            const d = r.delta ?? 0;
            if (d === 0) return "—";
            return h(
              "span",
              { class: d > 0 ? "grow" : "shrink" },
              h("span", { "aria-hidden": "true", text: d > 0 ? "▲ " : "▼ " }),
              fmt.delta(d),
            );
          },
        },
        {
          key: "ditems",
          label: "Δ items",
          cls: "num items",
          cell: (r) => (r.kind === "dir" ? fmt.signed(r.deltaItems ?? 0) : "—"),
        },
        { ...modified, sort: undefined, cell: (r, c) => (r.status === "gone" ? "—" : fmt.ago(r.mtime, c.now)) },
      ];
  }
}
