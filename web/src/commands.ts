// Commands: navigation, the four modes, rows and selection, hiding. Keys
// (keys.ts) and clicks call them; the request coordinator loads the result.

import type { App } from "./app";
import { controllers } from "./modes";
import { DEFAULT_LIMIT } from "./page";
import { format, parentPlace, sameView, type Mode, type Route } from "./router";
import type { Row } from "./rows";
import { toast } from "./toast";

function noFilters(): Partial<Route> {
  return {
    kind: undefined,
    min: undefined,
    untouched: undefined,
    name: undefined,
    icase: undefined,
    before: undefined,
    after: undefined,
    run: undefined,
    sort: undefined,
  };
}

export class Commands {
  lastFind: Partial<Route> = {};
  /** set by setMode("find"): the next apply puts the cursor in the name field once it draws the form */
  private focusName = false;

  constructor(private app: App) {}

  // ---- navigation ----

  navigate(patch: Partial<Route>, push = true): void {
    const next: Route = { ...this.app.route, ...patch };
    const hash = format(next);
    if (push && hash !== location.hash) {
      location.hash = hash; // hashchange → apply
    } else {
      history.replaceState(null, "", hash);
      this.apply(next);
    }
  }

  apply(next: Route): void {
    const app = this.app;
    const prev = app.route;
    app.route = next;
    const focusName = this.focusName;
    this.focusName = false;
    if (app.page !== "main") return;
    if (app.view && sameView(prev, next)) {
      this.selectPath(next.sel);
      app.table.renderSelection();
      return;
    }
    if (prev.path !== next.path || prev.mode !== next.mode) {
      app.showHidden = false;
      app.sheet.close(false);
    }
    // the tab and the form follow the key press at once; rows follow the answer
    app.layout.renderTabs();
    if (app.ui) app.layout.renderControls();
    if (focusName) document.getElementById("f-name")?.focus();
    void app.loader.load();
  }

  setMode(m: Mode): void {
    const r = this.app.route;
    if (m === r.mode) return;
    if (m === "changes" && !this.app.status?.prev) {
      toast("No previous snapshot to compare yet.");
      return;
    }
    if (r.mode === "find") {
      this.lastFind = { name: r.name, icase: r.icase, before: r.before, after: r.after, min: r.min, kind: r.kind, run: r.run };
    }
    const clean: Partial<Route> = {
      mode: m,
      sel: undefined,
      limit: undefined,
      sort: undefined,
      kind: undefined,
      min: undefined,
      untouched: undefined,
      name: undefined,
      icase: undefined,
      before: undefined,
      after: undefined,
      run: undefined,
    };
    // a timer could run before the hashchange that draws the form, so apply focuses it
    this.focusName = m === "find";
    this.navigate(m === "find" ? { ...clean, ...this.lastFind } : clean);
  }

  open(r: Row): void {
    const mode = this.app.route.mode;
    if (r.mount) {
      this.navigate({ mode: "browse", path: r.mount.root ?? r.path, sel: undefined, limit: undefined });
      return;
    }
    if (r.kind !== "dir" || r.status === "gone") {
      if (mode === "top" || mode === "find") this.showInFolder(r);
      return;
    }
    if (mode === "browse" || mode === "changes") {
      this.navigate({ path: r.path, raw: r.raw, sel: undefined, limit: undefined });
    } else {
      this.navigate({ mode: "browse", path: r.path, raw: r.raw, sel: undefined, limit: undefined, ...noFilters() });
    }
  }

  up(): void {
    const p = parentPlace(this.app.place);
    if (p) this.navigate({ path: p.path, raw: p.raw, sel: this.app.scope, limit: undefined });
  }

  showInFolder(r: Row): void {
    const p = parentPlace(r) ?? { path: "/" };
    this.navigate({ mode: "browse", path: p.path, raw: p.raw, sel: r.path, limit: undefined, ...noFilters() });
  }

  showMore(): void {
    const v = this.app.view;
    const cur = v?.ls?.limit ?? v?.top?.limit ?? v?.find?.limit ?? v?.diff?.limit ?? DEFAULT_LIMIT;
    this.navigate({ limit: Math.min(cur * 2, 10000), sel: this.app.selected?.path }, false);
  }

  // ---- rows and selection ----

  buildRows(): void {
    const app = this.app;
    const v = app.view;
    const rows = v ? controllers[v.route.mode].rows(v, app.scope) : [];
    for (const r of rows) r.hidden = app.hidden.has(r.path);
    app.allRows = rows;
    app.rows = app.showHidden ? rows : rows.filter((r) => !r.hidden);
  }

  selectPath(path: string | undefined): void {
    const rows = this.app.rows;
    const i = path ? rows.findIndex((r) => r.path === path) : -1;
    this.app.sel = i >= 0 ? i : 0;
  }

  selectIdx(i: number, focus = true): void {
    const app = this.app;
    if (app.rows.length === 0) return;
    app.sel = Math.max(0, Math.min(app.rows.length - 1, i));
    const r = app.rows[app.sel]!;
    app.route = { ...app.route, sel: r.path };
    history.replaceState(null, "", format(app.route));
    app.table.renderSelection(focus);
  }

  move(d: number): void {
    this.selectIdx(this.app.sel + d);
  }

  hide(r: Row | null): void {
    const app = this.app;
    if (!r || r.status === "gone") return;
    app.hidden.add({ path: r.path, allocated: r.allocated });
    const i = app.sel;
    this.buildRows();
    app.sel = Math.max(0, Math.min(i, app.rows.length - 1));
    if (app.rows[app.sel]) app.route = { ...app.route, sel: app.rows[app.sel]!.path };
    history.replaceState(null, "", format(app.route));
    app.renderAll(true);
    toast("Hidden until the next scan", { label: "Undo", run: () => this.unhide(r.path) });
  }

  /** Undo and Unhide: the row comes back selected. */
  unhide(path: string): void {
    const app = this.app;
    app.hidden.remove(path);
    this.buildRows();
    this.selectPath(path);
    app.route = { ...app.route, sel: app.selected?.path };
    history.replaceState(null, "", format(app.route));
    app.renderAll(true);
  }

  toggleHidden(): void {
    const app = this.app;
    app.showHidden = !app.showHidden;
    const keep = app.selected?.path;
    this.buildRows();
    this.selectPath(keep);
    app.renderAll();
  }
}
