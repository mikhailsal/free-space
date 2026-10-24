// Find (D6): a name pattern and filters within the scope, including
// subfolders; it searches on submit, not while typing.

import * as api from "./api";
import type { App } from "./app";
import { h } from "./dom";
import * as fmt from "./format";
import { DEFAULT_LIMIT, folderEntry, pathParams, stateBox, type ViewController } from "./page";
import { relative, type Route } from "./router";
import { fromEntry } from "./rows";
import type { FindResponse } from "./types";

const SIZE_RE = /^\s*\d+\s*[KMGTkmgt]?\s*$/;
export const WHEN_RE = /^\s*(\d{4}-\d{2}-\d{2}|\d+(d|w|mo|y))\s*$/;

function findSubmitted(r: Route): boolean {
  return !!(r.run || r.name || r.before || r.after || r.min);
}

function field(app: App, id: string, label: string, value: string | undefined, placeholder: string): HTMLElement {
  const err = app.fieldError?.field === id ? app.fieldError.text : "";
  return h(
    "div",
    { class: `field f-${id}` },
    h("label", { for: `f-${id}`, text: label }),
    h("input", {
      id: `f-${id}`,
      name: id,
      value: value ?? "",
      placeholder,
      autocomplete: "off",
      spellcheck: "false",
      "aria-invalid": err ? "true" : null,
      "aria-describedby": err ? `f-${id}-err` : null,
    }),
    err ? h("span", { class: "err", id: `f-${id}-err`, text: err }) : null,
  );
}

function clearButton(app: App): HTMLElement {
  return h("button", {
    type: "button",
    class: "outline",
    onclick: () => {
      app.fieldError = null;
      app.cmd.lastFind = {};
      app.cmd.navigate({
        name: undefined,
        icase: undefined,
        before: undefined,
        after: undefined,
        min: undefined,
        kind: undefined,
        run: undefined,
        sel: undefined,
        limit: undefined,
      });
    },
    text: "Clear",
  });
}

function submit(app: App, form: HTMLFormElement): void {
  const val = (id: string) => (form.querySelector<HTMLInputElement>(`#f-${id}`)?.value ?? "").trim();
  const v = { name: val("name"), before: val("before"), after: val("after"), min: val("min") };
  const kindV = form.querySelector<HTMLSelectElement>("#f-kind")?.value ?? "any";
  const icase = form.querySelector<HTMLInputElement>("#f-icase")?.checked ?? false;
  let err: { field: string; text: string } | null = null;
  if (v.min && !SIZE_RE.test(v.min)) err = { field: "min", text: "Not a size. Try 100M or 2G." };
  for (const f of ["after", "before"] as const) {
    if (v[f] && !WHEN_RE.test(v[f])) err = { field: f, text: "Enter a date as YYYY-MM-DD or a duration such as 180d." };
  }
  if (err) {
    app.fieldError = err;
    app.layout.renderControls(true);
    document.getElementById(`f-${err.field}`)?.focus();
    return;
  }
  app.fieldError = null;
  app.cmd.navigate({
    name: v.name || undefined,
    icase: icase || undefined,
    before: v.before || undefined,
    after: v.after || undefined,
    min: v.min ? v.min.replace(/\s+/g, "").toUpperCase() : undefined,
    kind: kindV === "any" ? undefined : kindV,
    run: true,
    sel: undefined,
    limit: undefined,
  });
}

function findForm(app: App): HTMLElement {
  const r = app.route;
  const kind = r.kind ?? "any";
  const opt = (v: string, label: string) => h("option", { value: v, selected: kind === v, text: label });
  const form = h(
    "form",
    { class: "findform", novalidate: true },
    h("p", { class: "scope" }, "Find within ", h("b", { class: "mono", text: app.scope }), ", including subfolders"),
    h(
      "div",
      { class: "fields" },
      h(
        "div",
        { class: "namegroup" },
        field(app, "name", "Name pattern", r.name, "*.iso"),
        h("label", { class: "check" }, h("input", { type: "checkbox", id: "f-icase", checked: !!r.icase }), " ignore case"),
      ),
      field(app, "before", "Modified before", r.before, "2025-01-01"),
      field(app, "after", "Modified after", r.after, "2025-01-01"),
      field(app, "min", "Min size", r.min, "100M"),
      h(
        "div",
        { class: "field" },
        h("label", { for: "f-kind", text: "Kind" }),
        h("select", { id: "f-kind" }, opt("file", "files"), opt("dir", "folders"), opt("any", "any")),
      ),
      h("div", { class: "buttons" }, h("button", { type: "submit", class: "primary", text: "Find" }), clearButton(app)),
    ),
    h("p", { class: "hint muted", text: "Name only · all filters must match · press Enter to search" }),
  );
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    submit(app, form);
  });
  return form;
}

function chip(app: App, label: string, clear: Partial<Route>): HTMLElement {
  return h(
    "button",
    {
      type: "button",
      class: "chip",
      "aria-label": `Remove filter ${label}`,
      onclick: () => app.cmd.navigate({ ...clear, sel: undefined, limit: undefined }),
    },
    label,
    h("span", { class: "x", "aria-hidden": "true", text: "×" }),
  );
}

function chips(app: App): HTMLElement[] {
  const r = app.route;
  const out: HTMLElement[] = [];
  if (r.name) out.push(chip(app, `${r.icase ? "iname" : "name"}: ${r.name}`, { name: undefined, icase: undefined, run: true }));
  if (r.min) out.push(chip(app, `min: ${r.min}`, { min: undefined, run: true }));
  if (r.before) out.push(chip(app, `before: ${r.before}`, { before: undefined, run: true }));
  if (r.after) out.push(chip(app, `after: ${r.after}`, { after: undefined, run: true }));
  if (r.kind)
    out.push(chip(app, `kind: ${r.kind === "dir" ? "folders" : r.kind === "file" ? "files" : "any"}`, { kind: undefined, run: true }));
  return out;
}

// worded from the current route, not from the route the view was fetched for
function findSummary(f: FindResponse, app: App): Node[] {
  const r = app.route;
  const noun = (r.kind ?? "any") === "file" ? (f.total_count === 1 ? "file" : "files") : f.total_count === 1 ? "match" : "matches";
  const parts: Node[] = [
    h(
      "div",
      { class: "total" },
      h(
        "p",
        null,
        h("b", { class: "mono", text: fmt.count(f.total_count) }),
        ` ${noun} · `,
        h("b", { class: "mono", text: fmt.size(f.total_allocated) }),
        " allocated",
      ),
      h(
        "p",
        { class: "muted" },
        f.truncated ? `Showing the largest ${fmt.count(f.entries.length)}. ` : "",
        "Total includes all matches; nested folders counted once.",
      ),
    ),
  ];
  const list = chips(app);
  if (list.length) parts.push(h("div", { class: "chips", "aria-label": "Active filters" }, list));
  return parts;
}

export const find: ViewController = {
  async fetch(r, sid, signal) {
    if (!findSubmitted(r)) return { route: r, folder: await folderEntry(r, sid, signal), idle: true };
    const [folder, found] = await Promise.all([
      folderEntry(r, sid, signal),
      api.get<FindResponse>(
        "find",
        {
          ...pathParams(r),
          name: r.icase ? undefined : r.name,
          iname: r.icase ? r.name : undefined,
          older_than: r.before,
          newer_than: r.after,
          min_size: r.min,
          kind: r.kind ?? "any",
          limit: r.limit ?? DEFAULT_LIMIT,
          snapshot: sid,
        },
        signal,
      ),
    ]);
    return { route: r, folder, find: found };
  },

  rows: (v, scope) => (v.find?.entries ?? []).map((e) => fromEntry(e, relative(e.path, scope))),

  summary: (v, app) => (v.find ? findSummary(v.find, app) : []),

  form: findForm,

  empty(v, app) {
    if (v.idle) return stateBox("Fill in a name pattern or a filter, then press Find.", null);
    return stateBox(`No matches within ${v.folder.path}.`, {
      label: "Clear filters",
      run: () =>
        app.cmd.navigate({
          name: undefined,
          icase: undefined,
          before: undefined,
          after: undefined,
          min: undefined,
          kind: undefined,
          run: true,
        }),
    });
  },
};
