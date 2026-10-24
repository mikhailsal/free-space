// The page's frame (D6): identity line, fill line, notice slot, crumbs, tabs,
// the mode's form and summary, the work area and the keys footer.

import type { App } from "./app";
import { refreshCommand } from "./cli";
import { h, replace } from "./dom";
import * as fmt from "./format";
import { openHelp, trapFocus } from "./help";
import { controllers } from "./modes";
import { must } from "./must";
import { now } from "./page";
import { ancestors, format, type Mode, type Place } from "./router";
import * as pages from "./states";
import { copy } from "./toast";
import type { RootOut } from "./types";

export interface Ui {
  identity: HTMLElement;
  fill: HTMLElement;
  notices: HTMLElement;
  crumbs: HTMLElement;
  tabs: HTMLElement;
  formrow: HTMLElement;
  progress: HTMLElement;
  main: HTMLElement;
  summary: HTMLElement;
  wrap: HTMLElement;
  table: HTMLTableElement;
  body: HTMLElement;
  more: HTMLElement;
  aside: HTMLElement;
  overlay: HTMLElement;
}

export class Layout {
  /** false until the fill line has grown in once; a new layout or snapshot grows it again */
  animatedFill = false;
  private formKey = "";

  constructor(private app: App) {}

  build(): void {
    const ui: Ui = {
      identity: h("div", { class: "identity" }),
      fill: h("div", { class: "fillline", "aria-hidden": "true" }, h("i", null)),
      notices: h("div", { class: "notice-slot" }),
      crumbs: h("nav", { class: "crumbs", "aria-label": "Folder" }),
      tabs: this.tabs(),
      formrow: h("div", { class: "formrow" }),
      progress: h("div", { class: "progress", "aria-hidden": "true" }),
      main: h("main", { class: "results", "aria-busy": "false" }),
      summary: h("div", { class: "summary" }),
      wrap: h("div", { class: "tablewrap" }),
      table: h("table", { class: "grid" }),
      body: h("div", { class: "bodystate" }),
      more: h("div", { class: "more" }),
      aside: h("aside", { class: "inspector", "aria-label": "Details" }),
      overlay: h("div", { class: "overlay", hidden: true }),
    };
    ui.wrap.appendChild(ui.table);
    ui.main.append(ui.summary, ui.wrap, ui.body, ui.more);
    ui.aside.addEventListener("keydown", (e) => {
      if (this.app.sheet.isOpen) trapFocus(ui.aside, e);
    });
    this.app.root.replaceChildren(this.shell(ui), ui.overlay);
    this.app.ui = ui;
    this.formKey = "";
    this.animatedFill = false;
  }

  private tabs(): HTMLElement {
    const app = this.app;
    const tab = (m: Mode, label: string, key: string) =>
      h("button", {
        type: "button",
        role: "tab",
        "data-mode": m,
        "aria-keyshortcuts": key,
        onclick: () => app.cmd.setMode(m),
        text: label,
      });
    return h(
      "div",
      { class: "tabs" },
      h(
        "div",
        { role: "tablist", "aria-label": "View" },
        tab("browse", "Browse", "Escape"),
        tab("top", "Top files", "t"),
        tab("find", "Find", "/"),
        tab("changes", "Changes", "d"),
      ),
      h("button", {
        type: "button",
        class: "ghost",
        onclick: () => void copy(refreshCommand(app.place.path ? app.place : { path: "/" }), "Command copied"),
        text: "Copy refresh command",
      }),
      h("button", {
        type: "button",
        class: "help-btn",
        "aria-label": "Keyboard help",
        onclick: () => openHelp(must(app.ui, "the main page").overlay),
        text: "?",
      }),
    );
  }

  private shell(ui: Ui): HTMLElement {
    const key = (k: string, what: string) => h("span", null, h("b", { text: k }), ` ${what}`);
    return h(
      "div",
      { class: "shell" },
      h(
        "header",
        { class: "top" },
        ui.identity,
        ui.fill,
        ui.notices,
        h("div", { class: "folder" }, ui.crumbs, ui.tabs),
        ui.formrow,
        ui.progress,
      ),
      h("div", { class: "work" }, ui.main, ui.aside),
      h(
        "footer",
        { class: "keys" },
        key("↑↓", "move"),
        key("→", "open"),
        key("←", "up"),
        key("t", "top files"),
        key("/", "find"),
        key("d", "changes"),
        key("h", "hide"),
        key("?", "help"),
      ),
    );
  }

  private rootFor(path: string): RootOut | null {
    let best: RootOut | null = null;
    for (const r of this.app.status?.roots ?? []) {
      const under = path === r.path || r.path === "/" || path.startsWith(`${r.path}/`);
      if (under && (!best || r.path.length > best.path.length)) best = r;
    }
    return best;
  }

  renderIdentity(): void {
    const app = this.app;
    const ui = must(app.ui, "the main page");
    const s = must(app.status, "the status");
    const live = app.latest ?? s;
    const ageS = (s.age_seconds ?? 0) + (now() - app.statusAt);
    const ageCls = ageS > 7 * 86400 ? "danger" : ageS > 36 * 3600 ? "warn" : "muted";
    const ageText =
      ageS > 7 * 86400 ? `snapshot is ${Math.floor(ageS / 86400)} days old, is the timer running?` : `snapshot ${fmt.age(ageS)} ago`;
    const root = this.rootFor(app.route.path) ?? s.roots[0] ?? null;
    let fsText = "";
    let pct = 0;
    if (root && root.fs_total > 0) {
      pct = Math.round((root.fs_used / root.fs_total) * 100);
      fsText = `Filesystem ${root.path} · ${pct}% used · ${fmt.size(root.fs_avail)} free at scan`;
    }
    const fsCls = pct >= 90 ? "danger" : pct >= 80 ? "warn" : "";
    const scanning = live.scanning ? pages.scanningLine(live, now(), app.latestAt) : "";
    replace(
      ui.identity,
      h("div", { class: "brand" }, "freespace ", h("span", { text: `· ${s.host}` })),
      h(
        "div",
        { class: "right" },
        h("span", {
          class: `age ${ageCls}`,
          title: `generation ${s.generation} · ${fmt.count(s.entries ?? 0)} entries · scanned ${fmt.dateTime(s.scanned_at ?? 0)}`,
          text: ageText,
        }),
        scanning ? h("span", { class: "scanning muted mono", text: scanning }) : null,
        fsText ? h("span", { class: `fs mono ${fsCls}`, text: fsText }) : null,
      ),
    );
    ui.fill.className = `fillline ${fsCls}`;
    const bar = ui.fill.firstElementChild as HTMLElement;
    if (!this.animatedFill) {
      this.animatedFill = true;
      bar.style.width = "0";
      requestAnimationFrame(() => requestAnimationFrame(() => bar.style.setProperty("width", `${pct}%`)));
    } else {
      bar.style.setProperty("width", `${pct}%`);
    }
  }

  private hiddenSummary(): { count: number; allocated: number } {
    const app = this.app;
    if (app.route.mode === "browse") return app.hidden.under(app.scope);
    let count = 0;
    let allocated = 0;
    for (const r of app.allRows) {
      if (r.hidden) {
        count++;
        allocated += r.allocated;
      }
    }
    return { count, allocated };
  }

  renderFolder(): void {
    const app = this.app;
    const ui = must(app.ui, "the main page");
    const chain = ancestors(app.place);
    const parts: Node[] = [];
    const link = (p: Place, label: string) =>
      h("a", { href: format({ ...app.route, path: p.path, raw: p.raw, sel: undefined, limit: undefined }), text: label });
    if (chain.length === 1) {
      parts.push(h("h1", { text: "/" }));
    } else {
      parts.push(link(chain[0], "/"));
      const below = chain.slice(1);
      below.forEach((p, i) => {
        const s = p.path.slice(p.path.lastIndexOf("/") + 1);
        if (i > 0) parts.push(h("span", { class: "sep", "aria-hidden": "true", text: "›" }));
        parts.push(i === below.length - 1 ? h("h1", { text: s }) : link(p, s));
      });
    }
    const f = app.view?.folder;
    if (f) {
      const lb = f.errors > 0 || f.denied;
      parts.push(h("span", { class: "size mono", text: `${lb ? "≥ " : ""}${fmt.size(f.allocated)}` }));
    }
    const hs = this.hiddenSummary();
    if (hs.count > 0) {
      parts.push(
        h("button", {
          type: "button",
          class: "hid",
          "aria-pressed": String(app.showHidden),
          onclick: () => app.cmd.toggleHidden(),
          text: app.showHidden ? `Showing ${hs.count} hidden` : `${hs.count} hidden · ${fmt.size(hs.allocated)}`,
        }),
      );
    }
    replace(ui.crumbs, parts);
  }

  renderTabs(): void {
    const app = this.app;
    if (!app.ui) return;
    const noPrev = !app.status?.prev;
    for (const b of app.ui.tabs.querySelectorAll<HTMLButtonElement>("[role=tab]")) {
      const m = b.dataset.mode as Mode;
      const on = m === app.route.mode;
      b.setAttribute("aria-selected", String(on));
      b.tabIndex = on ? 0 : -1;
      b.classList.toggle("on", on);
      if (m === "changes") {
        b.setAttribute("aria-disabled", String(noPrev));
        b.title = noPrev ? "No previous snapshot to compare yet." : "What changed since the previous scan · d";
      }
    }
  }

  /** Draws the mode's form when what it shows changed; `force` draws it anyway (a field error). */
  renderControls(force = false): void {
    const app = this.app;
    const r = app.route;
    if (force) this.formKey = "";
    const key = JSON.stringify([r.mode, app.scope, r.kind, r.min, r.untouched, r.name, r.icase, r.before, r.after, r.run, app.fieldError]);
    if (key === this.formKey) return;
    this.formKey = key;
    const ui = must(app.ui, "the main page");
    const form = controllers[r.mode].form(app);
    ui.formrow.hidden = form === null;
    replace(ui.formrow, form);
  }

  renderSummary(): void {
    const ui = must(this.app.ui, "the main page");
    const v = this.app.view;
    const parts = v ? controllers[v.route.mode].summary(v, this.app) : [];
    replace(ui.summary, parts);
    ui.summary.hidden = parts.length === 0;
  }
}
