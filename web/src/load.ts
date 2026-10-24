// The request coordinator (D19): one load at a time, the newest wins. A mode's
// controller fetches through it and never applies a response itself; only
// load() puts an answer on screen, and only while it is still the newest.

import * as api from "./api";
import { ApiError } from "./api";
import type { App } from "./app";
import { h, replace } from "./dom";
import { WHEN_RE } from "./find";
import { controllers } from "./modes";
import { must } from "./must";
import { stateBox } from "./page";
import { format, parentOf, type Route } from "./router";
import { continuity, nearestExisting } from "./snapshot";
import { toast } from "./toast";
import type { LsResponse } from "./types";

export class Loader {
  private abort: AbortController | null = null;
  private seq = 0;
  private timer = 0;
  private loading = false;

  constructor(private app: App) {}

  async load(): Promise<void> {
    const app = this.app;
    const route = { ...app.route };
    const seq = ++this.seq;
    this.abort?.abort();
    const ac = new AbortController();
    this.abort = ac;
    this.setLoading(true);
    try {
      const view = await controllers[route.mode].fetch(route, must(app.status?.snapshot_id, "the snapshot"), ac.signal);
      if (seq !== this.seq) return;
      app.view = view;
      app.bodyState = null;
      app.fieldError = null;
      app.cmd.buildRows();
      app.cmd.selectPath(route.sel);
      app.renderAll();
    } catch (e) {
      if (seq !== this.seq) return;
      this.loadError(e, route);
    } finally {
      if (seq === this.seq) this.setLoading(false);
    }
  }

  /** Drops the request in flight; the session ended. */
  stop(): void {
    this.abort?.abort();
  }

  private loadError(e: unknown, route: Route): void {
    const app = this.app;
    if (!(e instanceof ApiError) || e.kind !== "api" || !e.detail) {
      app.poller.onFailure(e);
      return;
    }
    const d = e.detail;
    switch (d.code) {
      case "PATH_NOT_FOUND":
        app.view = null;
        app.allRows = app.rows = [];
        app.bodyState = stateBox(
          "This folder is not in the snapshot.",
          parentOf(route.path) !== null ? { label: "Go to parent", run: () => void this.goNearest(route.path) } : null,
          d,
        );
        break;
      case "USAGE": {
        const m = d.message;
        const field = /size/.test(m) ? "min" : /date/.test(m) ? (route.before && !WHEN_RE.test(route.before) ? "before" : "after") : "name";
        const text =
          field === "min"
            ? "Not a size. Try 100M or 2G."
            : field === "name"
              ? "Invalid pattern."
              : "Enter a date as YYYY-MM-DD or a duration such as 180d.";
        app.fieldError = { field, text };
        app.layout.renderControls(true);
        return;
      }
      case "NO_PREV":
        app.view = null;
        app.allRows = app.rows = [];
        app.bodyState = stateBox("No previous snapshot to compare yet.", { label: "Browse", run: () => app.cmd.setMode("browse") }, d);
        break;
      default:
        app.poller.onFailure(e);
        return;
    }
    app.renderAll();
  }

  private async goNearest(path: string): Promise<void> {
    try {
      const to = await nearestExisting(parentOf(path) ?? "/", (p) => this.exists(p));
      this.app.cmd.navigate({ path: to ?? this.app.status?.roots[0]?.path ?? "/", raw: undefined, sel: undefined });
    } catch (e) {
      this.app.poller.onFailure(e);
    }
  }

  private async exists(p: string): Promise<boolean> {
    try {
      await api.get<LsResponse>("ls", { path: p, limit: 1 });
      return true;
    } catch (e) {
      if (e instanceof ApiError && e.code === "PATH_NOT_FOUND") return false;
      throw e;
    }
  }

  /** "Load latest" (D10): same path, mode, filters, sort and selection. */
  async loadLatest(): Promise<void> {
    const app = this.app;
    const s = app.latest;
    if (!s?.snapshot_id) return;
    app.status = s;
    app.statusAt = app.latestAt;
    app.rotated = false;
    app.errorNotice = null;
    app.dismissedNewer = null;
    app.hidden.load(s.host, s.snapshot_id);
    app.sheet.clearCache();
    app.layout.animatedFill = false;
    const wanted = app.route.path;
    const sel = app.route.sel;
    let landed = wanted;
    if (!app.route.raw) {
      try {
        landed = (await nearestExisting(wanted, (p) => this.exists(p))) ?? s.roots[0]?.path ?? "/";
      } catch (e) {
        app.poller.onFailure(e);
        return;
      }
    }
    if (landed !== wanted) {
      app.route = { ...app.route, path: landed, sel: undefined };
      history.replaceState(null, "", format(app.route));
    }
    await this.load();
    const c = continuity(
      wanted,
      landed,
      landed === wanted ? sel : undefined,
      app.allRows.map((r) => r.path),
    );
    if (c.kind === "path-gone") toast(`“${c.from}” is gone in the new snapshot; showing ${c.to}`);
    else if (c.kind === "row-gone") toast(`“${c.name}” is no longer here`);
  }

  private setLoading(on: boolean): void {
    const app = this.app;
    this.loading = on;
    window.clearTimeout(this.timer);
    const ui = app.ui;
    if (!ui) return;
    if (!on) {
      ui.progress.classList.remove("on");
      ui.main.setAttribute("aria-busy", "false");
      ui.wrap.classList.remove("stale");
      if (app.route.mode === "find") app.layout.renderSummary();
      return;
    }
    // 300 ms grace before any indicator (D7)
    this.timer = window.setTimeout(() => {
      if (!this.loading || !app.ui) return;
      ui.progress.classList.add("on");
      ui.main.setAttribute("aria-busy", "true");
      if (app.route.mode === "find" && app.view?.find) {
        ui.wrap.classList.add("stale");
        replace(
          ui.summary,
          h(
            "div",
            { class: "total" },
            h("p", null, "Searching… name filters can take a few seconds"),
            h(
              "p",
              { class: "muted" },
              "Showing previous results. ",
              h("button", { type: "button", class: "link", onclick: () => this.cancel(), text: "Cancel" }),
            ),
          ),
        );
      }
    }, 300);
  }

  private cancel(): void {
    this.abort?.abort();
    this.seq++;
    this.setLoading(false);
    toast("Search cancelled");
  }
}
