// The details panel (Pass 7 "Inspector per kind", mockup B order, D11 actions).

import { command, refreshCommand, SCAN_COMMAND } from "./cli";
import { h } from "./dom";
import * as fmt from "./format";
import { parentPlace, type Place, type Route } from "./router";
import { isFolder, lowerBound, type Row } from "./rows";
import { copy } from "./toast";
import type { Entry } from "./types";

export interface InspectorHost {
  route: Route;
  scope: string;
  /** The scope with its raw bytes. */
  place: Place;
  folderSize: number;
  now: number;
  biggest(path: string): Promise<Entry[] | null>;
  hide(r: Row): void;
  unhide(path: string): void;
  open(r: Row): void;
  showInFolder(r: Row): void;
  showUnreadable(): void;
}

function base(p: string): string {
  if (p === "/") return "/";
  return p.slice(p.lastIndexOf("/") + 1) || p;
}

function cliLine(text: string): HTMLElement {
  return h(
    "div",
    { class: "cmd" },
    h("span", { class: "label", text: "CLI command" }),
    h(
      "div",
      { class: "cmdline" },
      // one box per word: a long path may wrap inside, "--json" never splits
      h("code", { class: "mono" }, text.split(" ").flatMap((w, i) => [i ? " " : "", h("span", { text: w })])),
      h("button", { type: "button", class: "link", onclick: () => copy(text, "Command copied"), text: "Copy" }),
    ),
  );
}

function biggestInside(host: InspectorHost, r: Row): HTMLElement {
  const box = h("div", { class: "mini", "aria-busy": "true" }, h("span", { class: "muted", text: "Loading…" }));
  const wrap = h("section", { class: "biggest" }, h("h2", { text: "Biggest inside" }), box);
  void host.biggest(r.path).then((list) => {
    box.removeAttribute("aria-busy");
    box.replaceChildren();
    if (!list || list.length === 0) {
      box.appendChild(h("span", { class: "muted", text: list ? "Empty" : "Unavailable" }));
      return;
    }
    for (const e of list) {
      const w = r.allocated > 0 ? Math.max(1, Math.round((e.allocated / r.allocated) * 100)) : 0;
      box.appendChild(
        h(
          "div",
          { class: "minirow", style: { "--w": `${w}%` } },
          h("span", { class: "nm", text: e.name }),
          h("span", { class: "num", text: fmt.size(e.allocated) }),
        ),
      );
    }
  });
  return wrap;
}

export function inspector(host: InspectorHost, r: Row | null): HTMLElement[] {
  if (!r) return [h("p", { class: "muted empty", text: "Nothing selected." })];
  const out: HTMLElement[] = [];
  const share = host.folderSize > 0 && r.status !== "gone" ? `${fmt.pct(r.allocated, host.folderSize)} of ${base(host.scope)} · ` : "";
  const sizeLine = r.mount
    ? null
    : h(
        "div",
        { class: "big" },
        h("span", { class: "v num", text: `${lowerBound(r) ? "≥ " : ""}${fmt.size(r.status === "gone" ? r.allocatedPrev ?? 0 : r.allocated)}` }),
        h("span", { class: "of", text: r.status === "gone" ? "before it was removed" : `${share}apparent ${fmt.size(r.apparent)}` }),
      );
  out.push(
    h(
      "section",
      { class: "head" },
      h("div", { class: "title", text: r.name }),
      sizeLine,
      h("div", { class: "path mono", text: r.path }),
    ),
  );

  if (r.hidden) {
    out.push(h("p", { class: "note", text: "Hidden until the next scan." }));
  }
  if (r.status && r.delta !== undefined && r.delta !== 0) {
    out.push(
      h(
        "p",
        { class: `note ${r.delta > 0 ? "grow" : "shrink"}` },
        `Δ since prev: ${r.delta > 0 ? "▲" : "▼"} ${fmt.delta(r.delta)}`,
        r.status === "new" ? " (new)" : r.status === "gone" ? " (gone)" : "",
      ),
    );
  }

  if (r.mount) {
    const m = r.mount;
    const used = m.used !== null && m.total !== null ? `, ${fmt.size(m.used)} of ${fmt.size(m.total)} used` : "";
    out.push(h("p", { class: "note", text: `Separate filesystem: ${m.fs_type}${used}. Not scanned here.` }));
    if (m.root) {
      out.push(
        h("button", { type: "button", class: "outline", onclick: () => host.open(r), text: "Open this filesystem" }),
      );
    }
    out.push(actions(host, r, false));
    return out;
  }
  if (r.denied) {
    out.push(
      h("p", { class: "note danger", text: "Could not be read as root; sizes below are lower bounds." }),
      h("button", { type: "button", class: "outline", onclick: () => copy(SCAN_COMMAND, "Command copied"), text: `Copy: ${SCAN_COMMAND}` }),
    );
  }
  if (r.kind === "file" && r.nlink > 1) {
    out.push(
      h(
        "p",
        { class: "note muted" },
        r.alias
          ? `0 · counted at ${r.owner ?? "another path"}. rm here may not free it.`
          : `Hardlinked (${r.nlink} paths); space is counted once, here. rm here may not free it.`,
      ),
    );
  }
  if (r.kind === "dir" && r.status !== "gone" && r.items > 0 && !r.denied) {
    out.push(biggestInside(host, r));
  }

  const dl = h("dl", null);
  const row = (k: string, v: string | Node) => dl.append(h("dt", { text: k }), h("dd", null, v));
  if (r.kind === "dir") row("Items", h("span", { class: "num", text: `${lowerBound(r) ? "≥ " : ""}${fmt.count(r.items)}` }));
  if (r.status !== "gone") row("Modified", h("span", { class: "num", text: fmt.dateTime(r.mtime) }));
  if (r.kind === "dir") {
    row(
      "Newest file",
      h("span", { class: "num", title: "Newest file inside, by modification time", text: r.newest ? fmt.date(r.newest) : "unknown" }),
    );
  }
  if (r.errors > 0) {
    row(
      "Unreadable",
      h(
        "span",
        null,
        h("span", { class: "num", text: fmt.count(r.errors) }),
        " ",
        h("button", { type: "button", class: "link", onclick: () => host.showUnreadable(), text: "Show unreadable paths" }),
      ),
    );
  }
  if (dl.childElementCount) out.push(dl);
  out.push(actions(host, r, true));
  return out;
}

function actions(host: InspectorHost, r: Row, withCli: boolean): HTMLElement {
  const folder: Place = isFolder(r) ? r : (parentPlace(r) ?? { path: "/" });
  const mode = host.route.mode;
  return h(
    "div",
    { class: "actions" },
    withCli ? cliLine(command(host.route, mode === "browse" && isFolder(r) ? r : host.place)) : null,
    r.status === "gone"
      ? null
      : r.hidden
        ? h("button", { type: "button", class: "primary", onclick: () => host.unhide(r.path), text: "Unhide" })
        : h(
            "button",
            { type: "button", class: "primary", onclick: () => host.hide(r), "aria-keyshortcuts": "h" },
            "Hide until next scan · h",
          ),
    mode === "top" || mode === "find"
      ? h("button", { type: "button", class: "outline", onclick: () => host.showInFolder(r), text: "Show in folder" })
      : null,
    h(
      "div",
      { class: "links" },
      h("button", { type: "button", class: "link", onclick: () => copy(r.path, "Path copied"), text: "Copy path" }),
      h("button", {
        type: "button",
        class: "link",
        onclick: () => copy(refreshCommand(folder), "Command copied"),
        text: "Copy refresh command",
      }),
    ),
  );
}
