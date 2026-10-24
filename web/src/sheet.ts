// The details panel beside the table (inspector.ts draws its content) and, on a
// narrow screen, the bottom sheet it opens as.

import * as api from "./api";
import type { App } from "./app";
import { h, replace } from "./dom";
import { inspector, type InspectorHost } from "./inspector";
import { must } from "./must";
import { now } from "./page";
import type { Entry, LsResponse } from "./types";

export class Sheet {
  isOpen = false;
  private biggestCache = new Map<string, Promise<Entry[] | null>>();

  constructor(private app: App) {}

  /** A new snapshot: the biggest children the panel showed are stale. */
  clearCache(): void {
    this.biggestCache.clear();
  }

  private host(): InspectorHost {
    const app = this.app;
    const cmd = app.cmd;
    return {
      route: app.route,
      scope: app.scope,
      place: app.place,
      folderSize: app.view?.folder.allocated ?? 0,
      now: now(),
      biggest: (p) => this.biggest(p),
      hide: (r) => cmd.hide(r),
      unhide: (p) => cmd.unhide(p),
      open: (r) => cmd.open(r),
      showInFolder: (r) => cmd.showInFolder(r),
      showUnreadable: () => {
        app.unreadableOpen = true;
        app.noticesExpanded = true;
        app.notices.render();
        app.ui?.notices.scrollIntoView({ block: "nearest" });
      },
    };
  }

  private biggest(path: string): Promise<Entry[] | null> {
    const sid = this.app.status?.snapshot_id ?? "";
    const key = `${sid}:${path}`;
    let p = this.biggestCache.get(key);
    if (!p) {
      p = api
        .get<LsResponse>("ls", { path, limit: 4, snapshot: sid })
        .then((r) => r.entries)
        .catch(() => null);
      this.biggestCache.set(key, p);
    }
    return p;
  }

  render(): void {
    const app = this.app;
    const ui = must(app.ui, "the main page");
    const content = inspector(this.host(), app.view ? app.selected : null);
    const close = h("button", { type: "button", class: "sheet-close outline", onclick: () => this.close(true), text: "Close" });
    // <aside> stays the Details landmark; the panel inside is the sheet's dialog
    const panel = h("div", { class: "panel" }, h("div", { class: "handle", "aria-hidden": "true" }), close, ...content);
    this.role(panel, this.isOpen);
    replace(ui.aside, panel);
  }

  private role(panel: Element | null, on: boolean): void {
    if (!panel) return;
    if (on) {
      panel.setAttribute("role", "dialog");
      panel.setAttribute("aria-modal", "true");
      panel.setAttribute("aria-label", "Details");
    } else {
      for (const a of ["role", "aria-modal", "aria-label"]) panel.removeAttribute(a);
    }
  }

  show(): void {
    const ui = this.app.ui;
    if (!ui) return;
    this.isOpen = true;
    ui.aside.classList.add("open");
    this.role(ui.aside.firstElementChild, true);
    ui.aside.querySelector<HTMLElement>(".sheet-close")?.focus();
    // keep the chosen row above the sheet
    ui.main.classList.add("sheet-open");
    ui.table.querySelector(`tbody tr[data-i="${this.app.sel}"]`)?.scrollIntoView({ block: "start" });
  }

  close(focus: boolean): void {
    const ui = this.app.ui;
    if (!ui || !this.isOpen) return;
    this.isOpen = false;
    ui.aside.classList.remove("open");
    this.role(ui.aside.firstElementChild, false);
    ui.main.classList.remove("sheet-open");
    if (focus) this.app.table.focusRow();
  }
}
