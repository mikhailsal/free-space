// The page: one results table in four modes (D6), the inspector, the notice
// slot (D5), full-page states (D7, D12) and snapshot refresh continuity (D10).

import * as api from "./api";
import { ApiError } from "./api";
import { months, refreshCommand, SCAN_COMMAND, untouched } from "./cli";
import { h, replace } from "./dom";
import * as fmt from "./format";
import { openHelp, trapFocus } from "./help";
import { Hidden } from "./hidden";
import { inspector, type InspectorHost } from "./inspector";
import { details, renderNotices, type Notice } from "./notices";
import { fromDiff, fromEntry, type Row } from "./rows";
import { format, parentOf, parse, relative, sameView, type Mode, type Route } from "./router";
import { continuity, nearestExisting } from "./snapshot";
import * as pages from "./states";
import { copy, mountToasts, toast } from "./toast";
import type { DiffResponse, Entry, ErrorDetail, FindResponse, LsResponse, RootOut, Status, TopResponse } from "./types";
import { columns, type Column } from "./views";

type Page = "boot" | "main" | "nosnap" | "session" | "access";

interface View {
  route: Route;
  folder: Entry;
  ls?: LsResponse;
  top?: TopResponse;
  find?: FindResponse;
  diff?: DiffResponse;
  /** Find mode before the first submit. */
  idle?: boolean;
}

interface Ui {
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

const DEFAULT_LIMIT = 30;
const TOP_SIZES = ["10M", "100M", "1G"];
const TOP_MONTHS = [3, 6, 12];
const now = () => Date.now() / 1000;
const narrow = () => window.matchMedia("(max-width: 700px)").matches;

const SIZE_RE = /^\s*\d+\s*[KMGTkmgt]?\s*$/;
const WHEN_RE = /^\s*(\d{4}-\d{2}-\d{2}|\d+(d|w|mo|y))\s*$/;

function findSubmitted(r: Route): boolean {
  return !!(r.run || r.name || r.before || r.after || r.min);
}

export class App {
  page: Page = "boot";
  route: Route;
  /** The snapshot on screen and when its status was fetched. */
  status: Status | null = null;
  statusAt = 0;
  /** The newest poll (live scanning, last_error, newer snapshot). */
  latest: Status | null = null;
  latestAt = 0;
  hidden = new Hidden();
  view: View | null = null;
  allRows: Row[] = [];
  rows: Row[] = [];
  sel = 0;
  showHidden = false;
  rotated = false;
  failure = false;
  errorNotice: ErrorDetail | null = null;
  dismissedNewer: string | null = null;
  noticesExpanded = false;
  unreadableOpen = false;
  bodyState: HTMLElement | null = null;
  fieldError: { field: string; text: string } | null = null;
  ui: Ui | null = null;
  private abort: AbortController | null = null;
  private loadSeq = 0;
  private loadingTimer = 0;
  private loading = false;
  private polling = false;
  private pollTimer = 0;
  private biggestCache = new Map<string, Promise<Entry[] | null>>();
  private formKey = "";
  private lastFind: Partial<Route> = {};
  private animatedFill = false;
  private sheetOpen = false;

  constructor(private root: HTMLElement) {
    this.route = parse(location.hash);
  }

  async start(): Promise<void> {
    const t = this.route.t || api.storedToken();
    if (t) {
      api.setToken(t);
      this.route.t = t;
    }
    mountToasts(document.body);
    window.addEventListener("hashchange", () => this.apply(parse(location.hash)));
    document.addEventListener("keydown", (e) => this.onKey(e));
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) void this.poll();
    });
    window.addEventListener("focus", () => void this.poll());
    window.setInterval(() => this.tick(), 1000);
    if (!api.hasToken()) {
      this.showSession();
      return;
    }
    await this.poll();
  }

  // ---- status polling (D10, D12) ----

  async poll(): Promise<void> {
    if (this.polling) return;
    this.polling = true;
    window.clearTimeout(this.pollTimer);
    try {
      const s = await api.get<Status>("status");
      this.failure = false;
      this.onStatus(s);
    } catch (e) {
      this.onFailure(e);
    } finally {
      this.polling = false;
      this.schedule();
    }
  }

  private schedule(): void {
    window.clearTimeout(this.pollTimer);
    if (document.hidden) return; // resumes on visibilitychange
    const ms = this.page === "nosnap" ? 5000 : this.page === "session" ? 10000 : 60000;
    this.pollTimer = window.setTimeout(() => void this.poll(), ms);
  }

  private onStatus(s: Status): void {
    this.latest = s;
    this.latestAt = now();
    if (s.generation === null || s.snapshot_id === null) {
      this.status = s;
      this.statusAt = this.latestAt;
      this.showNoSnapshot();
      return;
    }
    if (this.page !== "main") {
      this.status = s;
      this.statusAt = this.latestAt;
      this.enterMain();
      return;
    }
    this.renderIdentity();
    this.renderNotices();
    this.renderTabs();
  }

  private onFailure(e: unknown): void {
    if (!(e instanceof ApiError)) {
      console.error(e);
      return;
    }
    if (e.kind === "aborted") return;
    if (e.kind === "session") {
      this.showSession();
      return;
    }
    if (e.kind === "failure") {
      this.failure = true;
      if (this.page === "main") this.renderNotices();
      return;
    }
    const d = e.detail!;
    switch (d.code) {
      case "NO_SNAPSHOT":
        void this.poll();
        return;
      case "SNAPSHOT_ACCESS":
        this.page = "access";
        this.ui = null;
        this.root.replaceChildren(pages.noAccess(d));
        return;
      case "ROTATED":
        this.rotated = true;
        void this.poll();
        break;
      default:
        this.errorNotice = d;
    }
    if (this.page === "main") this.renderNotices();
    else {
      this.page = "access";
      this.root.replaceChildren(pages.fatal(d));
    }
  }

  private showNoSnapshot(): void {
    this.page = "nosnap";
    this.ui = null;
    this.root.replaceChildren(pages.noSnapshot(this.latest!, now(), this.latestAt));
  }

  private showSession(): void {
    if (this.page === "session") return;
    this.abort?.abort();
    this.page = "session";
    this.ui = null;
    this.root.replaceChildren(pages.sessionEnded(this.route.path || "/"));
    this.schedule();
  }

  private tick(): void {
    if (this.page === "nosnap" && this.latest?.scanning) {
      const el = document.getElementById("fp-scan");
      if (el) el.textContent = pages.scanningLine(this.latest, now(), this.latestAt);
    } else if (this.page === "main" && this.latest?.scanning) {
      this.renderIdentity();
    }
  }

  // ---- layout ----

  private enterMain(): void {
    const s = this.status!;
    this.page = "main";
    this.hidden.load(s.host, s.snapshot_id!);
    this.buildLayout();
    if (!this.route.path) {
      this.route.path = s.roots[0]?.path ?? "/";
      history.replaceState(null, "", format(this.route));
    }
    void this.load();
  }

  private buildLayout(): void {
    const tab = (m: Mode, label: string, key: string) =>
      h("button", {
        type: "button",
        role: "tab",
        "data-mode": m,
        "aria-keyshortcuts": key,
        onclick: () => this.setMode(m),
        text: label,
      });
    const ui: Ui = {
      identity: h("div", { class: "identity" }),
      fill: h("div", { class: "fillline", "aria-hidden": "true" }, h("i", null)),
      notices: h("div", { class: "notice-slot" }),
      crumbs: h("nav", { class: "crumbs", "aria-label": "Folder" }),
      tabs: h(
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
          onclick: () => void copy(refreshCommand(this.route.path || "/"), "Command copied"),
          text: "Copy refresh command",
        }),
        h("button", {
          type: "button",
          class: "help-btn",
          "aria-label": "Keyboard help",
          onclick: () => openHelp(this.ui!.overlay),
          text: "?",
        }),
      ),
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
      if (this.sheetOpen) trapFocus(ui.aside, e);
    });
    const key = (k: string, what: string) => h("span", null, h("b", { text: k }), ` ${what}`);
    this.root.replaceChildren(
      h(
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
      ),
      ui.overlay,
    );
    this.ui = ui;
    this.formKey = "";
    this.animatedFill = false;
  }

  // ---- navigation ----

  navigate(patch: Partial<Route>, push = true): void {
    const next: Route = { ...this.route, ...patch, t: this.route.t };
    const hash = format(next);
    if (push && hash !== location.hash) {
      location.hash = hash; // hashchange → apply
    } else {
      history.replaceState(null, "", hash);
      this.apply(next);
    }
  }

  private apply(next: Route): void {
    next.t = this.route.t;
    const prev = this.route;
    this.route = next;
    if (this.page !== "main") return;
    if (this.view && sameView(prev, next)) {
      this.selectPath(next.sel);
      this.renderSelection();
      return;
    }
    if (prev.path !== next.path || prev.mode !== next.mode) {
      this.showHidden = false;
      this.closeSheet(false);
    }
    // the tab and the form follow the key press at once; rows follow the answer
    this.renderTabs();
    if (this.ui) this.renderControls();
    void this.load();
  }

  setMode(m: Mode): void {
    if (m === this.route.mode) return;
    if (m === "changes" && !this.status?.prev) {
      toast("No previous snapshot to compare yet.");
      return;
    }
    if (this.route.mode === "find") {
      const r = this.route;
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
    this.navigate(m === "find" ? { ...clean, ...this.lastFind } : clean);
    if (m === "find") window.setTimeout(() => document.getElementById("f-name")?.focus(), 0);
  }

  open(r: Row): void {
    if (r.mount) {
      this.navigate({ mode: "browse", path: r.mount.root ?? r.path, sel: undefined, limit: undefined });
      return;
    }
    if (r.kind !== "dir" || r.status === "gone") {
      if (this.route.mode === "top" || this.route.mode === "find") this.showInFolder(r);
      return;
    }
    if (this.route.mode === "browse" || this.route.mode === "changes") {
      this.navigate({ path: r.path, raw: r.raw, sel: undefined, limit: undefined });
    } else {
      this.navigate({ mode: "browse", path: r.path, raw: r.raw, sel: undefined, limit: undefined, ...this.noFilters() });
    }
  }

  private noFilters(): Partial<Route> {
    return { kind: undefined, min: undefined, untouched: undefined, name: undefined, icase: undefined, before: undefined, after: undefined, run: undefined, sort: undefined };
  }

  up(): void {
    const p = parentOf(this.route.path);
    if (p !== null) this.navigate({ path: p, raw: undefined, sel: this.route.path, limit: undefined });
  }

  showInFolder(r: Row): void {
    const p = parentOf(r.path) ?? "/";
    this.navigate({ mode: "browse", path: p, raw: undefined, sel: r.path, limit: undefined, ...this.noFilters() });
  }

  // ---- loading ----

  private pathParams(r: Route): api.Params {
    return r.raw ? { path_raw: r.raw } : { path: r.path };
  }

  private topKind(r: Route = this.route): "file" | "dir" {
    return r.kind === "dir" ? "dir" : "file";
  }

  private untouched(r: Route = this.route): number {
    return untouched(r);
  }

  private rotatedError(): ApiError {
    return new ApiError("api", 409, { code: "ROTATED", exit: 4, message: "snapshot rotated", fix: null, docs: "" });
  }

  private async folderEntry(r: Route, signal: AbortSignal): Promise<Entry> {
    const ls = await api.get<LsResponse>("ls", { ...this.pathParams(r), limit: 1, snapshot: this.status!.snapshot_id }, signal);
    return ls.entry;
  }

  private async fetchView(r: Route, signal: AbortSignal): Promise<View> {
    const sid = this.status!.snapshot_id!;
    const pp = this.pathParams(r);
    const limit = r.limit ?? DEFAULT_LIMIT;
    const same = (id: string) => {
      if (id !== sid) throw this.rotatedError();
    };
    switch (r.mode) {
      case "browse": {
        const ls = await api.get<LsResponse>("ls", { ...pp, sort: r.sort, limit, snapshot: sid }, signal);
        return { route: r, folder: ls.entry, ls };
      }
      case "top": {
        const u = this.untouched(r);
        const [folder, top] = await Promise.all([
          this.folderEntry(r, signal),
          api.get<TopResponse>(
            "top",
            { ...pp, kind: this.topKind(r), min_size: r.min ?? "100M", older_than: u ? months(u) : undefined, limit },
            signal,
          ),
        ]);
        same(top.snapshot_id);
        return { route: r, folder, top };
      }
      case "find": {
        if (!findSubmitted(r)) return { route: r, folder: await this.folderEntry(r, signal), idle: true };
        const [folder, find] = await Promise.all([
          this.folderEntry(r, signal),
          api.get<FindResponse>(
            "find",
            {
              ...pp,
              name: r.icase ? undefined : r.name,
              iname: r.icase ? r.name : undefined,
              older_than: r.before,
              newer_than: r.after,
              min_size: r.min,
              kind: r.kind ?? "any",
              limit,
              snapshot: sid,
            },
            signal,
          ),
        ]);
        return { route: r, folder, find };
      }
      case "changes": {
        const [folder, diff] = await Promise.all([
          this.folderEntry(r, signal),
          api.get<DiffResponse>("diff", { ...pp, depth: 1, kind: "any", limit }, signal),
        ]);
        same(diff.snapshot_id);
        return { route: r, folder, diff };
      }
    }
  }

  async load(): Promise<void> {
    const route = { ...this.route };
    const seq = ++this.loadSeq;
    this.abort?.abort();
    const ac = new AbortController();
    this.abort = ac;
    this.setLoading(true);
    try {
      const view = await this.fetchView(route, ac.signal);
      if (seq !== this.loadSeq) return;
      this.view = view;
      this.bodyState = null;
      this.fieldError = null;
      this.buildRows();
      this.selectPath(route.sel);
      this.renderAll();
    } catch (e) {
      if (seq !== this.loadSeq) return;
      this.loadError(e, route);
    } finally {
      if (seq === this.loadSeq) this.setLoading(false);
    }
  }

  private loadError(e: unknown, route: Route): void {
    if (!(e instanceof ApiError) || e.kind !== "api") {
      this.onFailure(e);
      return;
    }
    const d = e.detail!;
    switch (d.code) {
      case "PATH_NOT_FOUND":
        this.view = null;
        this.allRows = this.rows = [];
        this.bodyState = this.stateBox(
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
        this.fieldError = { field, text };
        this.formKey = "";
        this.renderControls();
        return;
      }
      case "NO_PREV":
        this.view = null;
        this.allRows = this.rows = [];
        this.bodyState = this.stateBox("No previous snapshot to compare yet.", { label: "Browse", run: () => this.setMode("browse") }, d);
        break;
      default:
        this.onFailure(e);
        return;
    }
    this.renderAll();
  }

  private async goNearest(path: string): Promise<void> {
    try {
      const to = await nearestExisting(parentOf(path) ?? "/", (p) => this.exists(p));
      this.navigate({ path: to ?? this.status?.roots[0]?.path ?? "/", raw: undefined, sel: undefined });
    } catch (e) {
      this.onFailure(e);
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
    const s = this.latest;
    if (!s?.snapshot_id) return;
    this.status = s;
    this.statusAt = this.latestAt;
    this.rotated = false;
    this.errorNotice = null;
    this.dismissedNewer = null;
    this.hidden.load(s.host, s.snapshot_id);
    this.biggestCache.clear();
    this.animatedFill = false;
    const wanted = this.route.path;
    const sel = this.route.sel;
    let landed = wanted;
    if (!this.route.raw) {
      try {
        landed = (await nearestExisting(wanted, (p) => this.exists(p))) ?? s.roots[0]?.path ?? "/";
      } catch (e) {
        this.onFailure(e);
        return;
      }
    }
    if (landed !== wanted) {
      this.route = { ...this.route, path: landed, sel: undefined };
      history.replaceState(null, "", format(this.route));
    }
    await this.load();
    const c = continuity(wanted, landed, landed === wanted ? sel : undefined, this.allRows.map((r) => r.path));
    if (c.kind === "path-gone") toast(`“${c.from}” is gone in the new snapshot; showing ${c.to}`);
    else if (c.kind === "row-gone") toast(`“${c.name}” is no longer here`);
  }

  private setLoading(on: boolean): void {
    this.loading = on;
    window.clearTimeout(this.loadingTimer);
    const ui = this.ui;
    if (!ui) return;
    if (!on) {
      ui.progress.classList.remove("on");
      ui.main.setAttribute("aria-busy", "false");
      ui.wrap.classList.remove("stale");
      if (this.route.mode === "find") this.renderSummary();
      return;
    }
    // 300 ms grace before any indicator (D7)
    this.loadingTimer = window.setTimeout(() => {
      if (!this.loading || !this.ui) return;
      ui.progress.classList.add("on");
      ui.main.setAttribute("aria-busy", "true");
      if (this.route.mode === "find" && this.view?.find) {
        ui.wrap.classList.add("stale");
        replace(
          ui.summary,
          h(
            "div",
            { class: "total" },
            h("p", null, "Searching… name filters can take a few seconds"),
            h("p", { class: "muted" }, "Showing previous results. ", h("button", { type: "button", class: "link", onclick: () => this.cancel(), text: "Cancel" })),
          ),
        );
      }
    }, 300);
  }

  private cancel(): void {
    this.abort?.abort();
    this.loadSeq++;
    this.setLoading(false);
    toast("Search cancelled");
  }

  // ---- rows and selection ----

  get scope(): string {
    return this.view?.folder.path ?? this.route.path;
  }

  private buildRows(): void {
    const v = this.view;
    const scope = this.scope;
    let rows: Row[] = [];
    if (v?.ls) rows = v.ls.entries.map((e) => fromEntry(e));
    else if (v?.top) {
      rows = v.top.entries.map((e) => {
        const parent = parentOf(e.path) ?? "/";
        const rel = parent === scope ? "./" : `${relative(parent, scope)}/`;
        return fromEntry(e, e.name, rel);
      });
    } else if (v?.find) rows = v.find.entries.map((e) => fromEntry(e, relative(e.path, scope)));
    // Changes lists what changed; unchanged children are the API's tail (|Δ| = 0)
    else if (v?.diff) rows = v.diff.entries.filter((e) => e.status !== "same").map(fromDiff);
    for (const r of rows) r.hidden = this.hidden.has(r.path);
    this.allRows = rows;
    this.rows = this.showHidden ? rows : rows.filter((r) => !r.hidden);
  }

  private selectPath(path: string | undefined): void {
    const i = path ? this.rows.findIndex((r) => r.path === path) : -1;
    this.sel = i >= 0 ? i : 0;
  }

  get selected(): Row | null {
    return this.rows[this.sel] ?? null;
  }

  selectIdx(i: number, focus = true): void {
    if (this.rows.length === 0) return;
    this.sel = Math.max(0, Math.min(this.rows.length - 1, i));
    const r = this.rows[this.sel]!;
    this.route = { ...this.route, sel: r.path };
    history.replaceState(null, "", format(this.route));
    this.renderSelection(focus);
  }

  private move(d: number): void {
    this.selectIdx(this.sel + d);
  }

  hide(r: Row | null): void {
    if (!r || r.status === "gone") return;
    this.hidden.add({ path: r.path, allocated: r.allocated });
    const i = this.sel;
    this.buildRows();
    this.sel = Math.max(0, Math.min(i, this.rows.length - 1));
    if (this.rows[this.sel]) this.route = { ...this.route, sel: this.rows[this.sel]!.path };
    history.replaceState(null, "", format(this.route));
    this.renderAll(true);
    toast("Hidden until the next scan", { label: "Undo", run: () => this.unhide(r.path) });
  }

  /** Undo and Unhide: the row comes back selected. */
  unhide(path: string): void {
    this.hidden.remove(path);
    this.buildRows();
    this.selectPath(path);
    this.route = { ...this.route, sel: this.selected?.path };
    history.replaceState(null, "", format(this.route));
    this.renderAll(true);
  }

  private toggleHidden(): void {
    this.showHidden = !this.showHidden;
    const keep = this.selected?.path;
    this.buildRows();
    this.selectPath(keep);
    this.renderAll();
  }

  private showMore(): void {
    const cur = this.view?.ls?.limit ?? this.view?.top?.limit ?? this.view?.find?.limit ?? this.view?.diff?.limit ?? DEFAULT_LIMIT;
    this.navigate({ limit: Math.min(cur * 2, 10000), sel: this.selected?.path }, false);
  }

  // ---- keyboard (Pass 6) ----

  private onKey(e: KeyboardEvent): void {
    if (this.page !== "main" || !this.ui || !this.ui.overlay.hidden) return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const t = e.target as HTMLElement;
    const typing = t.closest("input, select, textarea") !== null;
    if (e.key === "Escape") {
      if (this.sheetOpen) {
        this.closeSheet(true);
      } else if (this.route.mode !== "browse") {
        if (typing) t.blur();
        this.setMode("browse");
      } else {
        return;
      }
      e.preventDefault();
      return;
    }
    if (typing) return;
    if ((e.key === "Enter" || e.key === " ") && t.closest("button, a")) return;
    switch (e.key) {
      case "ArrowDown":
        this.move(1);
        break;
      case "ArrowUp":
        this.move(-1);
        break;
      case "PageDown":
        this.move(10);
        break;
      case "PageUp":
        this.move(-10);
        break;
      case "Home":
        this.selectIdx(0);
        break;
      case "End":
        this.selectIdx(this.rows.length - 1);
        break;
      case "ArrowRight":
      case "Enter":
        if (this.selected) this.open(this.selected);
        break;
      case "ArrowLeft":
      case "Backspace":
        this.up();
        break;
      case "t":
        this.setMode("top");
        break;
      case "d":
        this.setMode("changes");
        break;
      case "/":
        if (this.route.mode === "find") document.getElementById("f-name")?.focus();
        else this.setMode("find");
        break;
      case "h":
        this.hide(this.selected);
        break;
      case "r":
        if (this.newerVisible()) void this.loadLatest();
        break;
      case "?":
        openHelp(this.ui.overlay);
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  // ---- rendering ----

  private renderAll(focus = false): void {
    if (!this.ui) return;
    this.renderIdentity();
    this.renderNotices();
    this.renderFolder();
    this.renderTabs();
    this.renderControls();
    this.renderSummary();
    this.renderTable(focus);
    this.renderInspector();
    this.renderMore();
  }

  private rootFor(path: string): RootOut | null {
    let best: RootOut | null = null;
    for (const r of this.status?.roots ?? []) {
      const under = path === r.path || r.path === "/" || path.startsWith(r.path + "/");
      if (under && (!best || r.path.length > best.path.length)) best = r;
    }
    return best;
  }

  private renderIdentity(): void {
    const ui = this.ui!;
    const s = this.status!;
    const live = this.latest ?? s;
    const ageS = (s.age_seconds ?? 0) + (now() - this.statusAt);
    const ageCls = ageS > 7 * 86400 ? "danger" : ageS > 36 * 3600 ? "warn" : "muted";
    const ageText =
      ageS > 7 * 86400 ? `snapshot is ${Math.floor(ageS / 86400)} days old, is the timer running?` : `snapshot ${fmt.age(ageS)} ago`;
    const root = this.rootFor(this.route.path) ?? s.roots[0] ?? null;
    let fsText = "";
    let pct = 0;
    if (root && root.fs_total > 0) {
      pct = Math.round((root.fs_used / root.fs_total) * 100);
      fsText = `Filesystem ${root.path} · ${pct}% used · ${fmt.size(root.fs_avail)} free at scan`;
    }
    const fsCls = pct >= 90 ? "danger" : pct >= 80 ? "warn" : "";
    const scanning = live.scanning ? pages.scanningLine(live, now(), this.latestAt) : "";
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

  private newerVisible(): boolean {
    const s = this.status;
    const live = this.latest;
    const newer = !!(s && live?.snapshot_id && live.snapshot_id !== s.snapshot_id);
    return (newer && this.dismissedNewer !== live!.snapshot_id) || this.rotated;
  }

  private notices(): Notice[] {
    const s = this.status!;
    const live = this.latest ?? s;
    const out: Notice[] = [];
    const scan = { label: `Copy: ${SCAN_COMMAND}`, run: () => void copy(SCAN_COMMAND, "Command copied") };
    if (s.incomplete) {
      const n = s.unknown_subtrees.length;
      const count = n >= 100 ? "100+" : String(n);
      out.push({
        id: "incomplete",
        priority: 1,
        tone: "danger",
        body: [
          h("b", { text: "Scan incomplete:" }),
          ` ${count} ${n === 1 ? "directory" : "directories"} could not be read as root. Sizes below are lower bounds.`,
        ],
        actions: [
          {
            label: this.unreadableOpen ? "Hide unreadable paths" : "Show unreadable paths",
            run: () => {
              this.unreadableOpen = !this.unreadableOpen;
              this.renderNotices();
            },
          },
          scan,
        ],
        extra: this.unreadableOpen
          ? h(
              "ul",
              { class: "paths mono" },
              s.unknown_subtrees.map((p) => h("li", { text: p })),
            )
          : null,
      });
    }
    const le = live.last_error;
    if (le && (!live.last_success_ts || le.ts > live.last_success_ts)) {
      out.push({
        id: "last-error",
        priority: 2,
        tone: "danger",
        body: [
          h("b", { text: `Last ${le.op} failed ${fmt.age(now() - le.ts)} ago:` }),
          ` ${le.message}. Showing the snapshot from ${fmt.dateTime(s.scanned_at ?? 0)}.`,
        ],
        actions: [scan],
      });
    }
    const err = this.errorNotice;
    if (err) {
      const text =
        err.code === "SCHEMA"
          ? "Snapshot was made by another version. Rescan to compare."
          : err.code === "HASH_COLLISION"
            ? "Snapshot inconsistency detected. Rescan."
            : `freespace could not answer: ${err.message}`;
      out.push({ id: "error", priority: 2, tone: "danger", body: [text], actions: [scan], extra: details(err.code, err.exit, err.message) });
    }
    if (this.failure) {
      out.push({
        id: "failure",
        priority: 2,
        tone: "danger",
        body: ["Can't reach freespace. Is ", h("code", { text: "freespace serve" }), " still running?"],
        actions: [
          {
            label: "Retry",
            run: () => {
              this.failure = false;
              this.renderNotices();
              void this.poll();
              void this.load();
            },
          },
        ],
        dismiss: () => {
          this.failure = false;
          this.renderNotices();
        },
      });
    }
    if (this.newerVisible()) {
      const newer = live.snapshot_id !== s.snapshot_id;
      out.push({
        id: "newer",
        priority: 3,
        tone: "neutral",
        body: [
          newer
            ? `A newer snapshot is ready (${fmt.dateTime(live.scanned_at ?? 0)}). Δ will now compare to the previous scan.`
            : "A new scan just finished.",
        ],
        actions: [{ label: "Load latest · r", run: () => void this.loadLatest() }],
        dismiss: () => {
          this.dismissedNewer = live.snapshot_id;
          this.rotated = false;
          this.renderNotices();
        },
      });
    }
    return out;
  }

  private renderNotices(): void {
    if (!this.ui) return;
    renderNotices(this.ui.notices, this.notices(), this.noticesExpanded, () => {
      this.noticesExpanded = !this.noticesExpanded;
      this.renderNotices();
    });
  }

  private hiddenSummary(): { count: number; allocated: number } {
    if (this.route.mode === "browse") return this.hidden.under(this.scope);
    let count = 0;
    let allocated = 0;
    for (const r of this.allRows) {
      if (r.hidden) {
        count++;
        allocated += r.allocated;
      }
    }
    return { count, allocated };
  }

  private renderFolder(): void {
    const ui = this.ui!;
    const path = this.scope;
    const segs = path === "/" ? [] : path.split("/").slice(1);
    const parts: Node[] = [];
    const link = (p: string, label: string) =>
      h("a", { href: format({ ...this.route, path: p, raw: undefined, sel: undefined, limit: undefined }), text: label });
    if (segs.length === 0) {
      parts.push(h("h1", { text: "/" }));
    } else {
      parts.push(link("/", "/"));
      segs.forEach((s, i) => {
        const p = "/" + segs.slice(0, i + 1).join("/");
        if (i > 0) parts.push(h("span", { class: "sep", "aria-hidden": "true", text: "›" }));
        parts.push(i === segs.length - 1 ? h("h1", { text: s }) : link(p, s));
      });
    }
    const f = this.view?.folder;
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
          "aria-pressed": String(this.showHidden),
          onclick: () => this.toggleHidden(),
          text: this.showHidden ? `Showing ${hs.count} hidden` : `${hs.count} hidden · ${fmt.size(hs.allocated)}`,
        }),
      );
    }
    replace(ui.crumbs, parts);
  }

  private renderTabs(): void {
    if (!this.ui) return;
    const noPrev = !this.status?.prev;
    for (const b of this.ui.tabs.querySelectorAll<HTMLButtonElement>("[role=tab]")) {
      const m = b.dataset.mode as Mode;
      const on = m === this.route.mode;
      b.setAttribute("aria-selected", String(on));
      b.tabIndex = on ? 0 : -1;
      b.classList.toggle("on", on);
      if (m === "changes") {
        b.setAttribute("aria-disabled", String(noPrev));
        b.title = noPrev ? "No previous snapshot to compare yet." : "What changed since the previous scan · d";
      }
    }
  }

  private renderControls(): void {
    const ui = this.ui!;
    const r = this.route;
    const key = JSON.stringify([r.mode, this.scope, r.kind, r.min, r.untouched, r.name, r.icase, r.before, r.after, r.run, this.fieldError]);
    if (key === this.formKey) return;
    this.formKey = key;
    if (r.mode === "find") {
      ui.formrow.hidden = false;
      replace(ui.formrow, this.findForm());
    } else if (r.mode === "top") {
      ui.formrow.hidden = false;
      replace(ui.formrow, this.topForm());
    } else {
      ui.formrow.hidden = true;
      replace(ui.formrow);
    }
  }

  private field(id: string, label: string, value: string | undefined, placeholder: string): HTMLElement {
    const err = this.fieldError?.field === id ? this.fieldError.text : "";
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

  private findForm(): HTMLElement {
    const r = this.route;
    const kind = r.kind ?? "any";
    const opt = (v: string, label: string) => h("option", { value: v, selected: kind === v, text: label });
    const form = h(
      "form",
      { class: "findform", novalidate: true },
      h("p", { class: "scope" }, "Find within ", h("b", { class: "mono", text: this.scope }), ", including subfolders"),
      h(
        "div",
        { class: "fields" },
        h(
          "div",
          { class: "namegroup" },
          this.field("name", "Name pattern", r.name, "*.iso"),
          h("label", { class: "check" }, h("input", { type: "checkbox", id: "f-icase", checked: !!r.icase }), " ignore case"),
        ),
        this.field("before", "Modified before", r.before, "2025-01-01"),
        this.field("after", "Modified after", r.after, "2025-01-01"),
        this.field("min", "Min size", r.min, "100M"),
        h("div", { class: "field" }, h("label", { for: "f-kind", text: "Kind" }), h("select", { id: "f-kind" }, opt("file", "files"), opt("dir", "folders"), opt("any", "any"))),
        h(
          "div",
          { class: "buttons" },
          h("button", { type: "submit", class: "primary", text: "Find" }),
          h("button", {
            type: "button",
            class: "outline",
            onclick: () => {
              this.fieldError = null;
              this.lastFind = {};
              this.navigate({ name: undefined, icase: undefined, before: undefined, after: undefined, min: undefined, kind: undefined, run: undefined, sel: undefined, limit: undefined });
            },
            text: "Clear",
          }),
        ),
      ),
      h("p", { class: "hint muted", text: "Name only · all filters must match · press Enter to search" }),
    );
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      const val = (id: string) => (form.querySelector<HTMLInputElement>(`#f-${id}`)?.value ?? "").trim();
      const v = { name: val("name"), before: val("before"), after: val("after"), min: val("min") };
      const kindV = form.querySelector<HTMLSelectElement>("#f-kind")!.value;
      const icase = form.querySelector<HTMLInputElement>("#f-icase")!.checked;
      let err: { field: string; text: string } | null = null;
      if (v.min && !SIZE_RE.test(v.min)) err = { field: "min", text: "Not a size. Try 100M or 2G." };
      for (const f of ["after", "before"] as const) {
        if (v[f] && !WHEN_RE.test(v[f])) err = { field: f, text: "Enter a date as YYYY-MM-DD or a duration such as 180d." };
      }
      if (err) {
        this.fieldError = err;
        this.formKey = "";
        this.renderControls();
        document.getElementById(`f-${err.field}`)?.focus();
        return;
      }
      this.fieldError = null;
      this.navigate({
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
    });
    return form;
  }

  private topForm(): HTMLElement {
    const r = this.route;
    const kind = this.topKind();
    const u = this.untouched();
    const min = r.min ?? "100M";
    const seg = (k: "file" | "dir", label: string) =>
      h("button", {
        type: "button",
        role: "radio",
        "aria-checked": String(kind === k),
        class: kind === k ? "on" : "",
        onclick: () => this.navigate({ kind: k === "dir" ? "dir" : undefined, untouched: undefined, sel: undefined, limit: undefined }),
        text: label,
      });
    const sizeSel = h(
      "select",
      { id: "t-min", "aria-label": "Larger than" },
      TOP_SIZES.map((s) => h("option", { value: s, selected: s === min, text: s.replace(/([KMGT])$/, " $1") })),
    );
    sizeSel.addEventListener("change", () => this.navigate({ min: sizeSel.value === "100M" ? undefined : sizeSel.value, sel: undefined, limit: undefined }));
    const monthSel = h(
      "select",
      { id: "t-months", "aria-label": "Months untouched" },
      TOP_MONTHS.map((m) => h("option", { value: String(m), selected: m === (u || 6), text: String(m) })),
    );
    const check = h("input", { type: "checkbox", id: "t-untouched", checked: u > 0 });
    const apply = () => this.navigate({ untouched: check.checked ? Number(monthSel.value) : 0, sel: undefined, limit: undefined });
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
        h("b", { class: "mono", text: this.scope }),
        " · larger than ",
        sizeSel,
      ),
      h(
        "label",
        { class: "check", title: "Not modified since: mtime is modification, not last open" },
        check,
        " Untouched for ",
      ),
      monthSel,
      h("span", { text: " months" }),
    );
  }

  private chip(label: string, clear: Partial<Route>): HTMLElement {
    return h(
      "button",
      { type: "button", class: "chip", "aria-label": `Remove filter ${label}`, onclick: () => this.navigate({ ...clear, sel: undefined, limit: undefined }) },
      label,
      h("span", { class: "x", "aria-hidden": "true", text: "×" }),
    );
  }

  private renderSummary(): void {
    const ui = this.ui!;
    const v = this.view;
    const r = this.route;
    const parts: Node[] = [];
    if (v?.find) {
      const f = v.find;
      const noun = (r.kind ?? "any") === "file" ? (f.total_count === 1 ? "file" : "files") : f.total_count === 1 ? "match" : "matches";
      parts.push(
        h(
          "div",
          { class: "total" },
          h("p", null, h("b", { class: "mono", text: fmt.count(f.total_count) }), ` ${noun} · `, h("b", { class: "mono", text: fmt.size(f.total_allocated) }), " allocated"),
          h(
            "p",
            { class: "muted" },
            f.truncated ? `Showing the largest ${fmt.count(f.entries.length)}. ` : "",
            "Total includes all matches; nested folders counted once.",
          ),
        ),
      );
      const chips: HTMLElement[] = [];
      if (r.name) chips.push(this.chip(`${r.icase ? "iname" : "name"}: ${r.name}`, { name: undefined, icase: undefined, run: true }));
      if (r.min) chips.push(this.chip(`min: ${r.min}`, { min: undefined, run: true }));
      if (r.before) chips.push(this.chip(`before: ${r.before}`, { before: undefined, run: true }));
      if (r.after) chips.push(this.chip(`after: ${r.after}`, { after: undefined, run: true }));
      if (r.kind) chips.push(this.chip(`kind: ${r.kind === "dir" ? "folders" : r.kind === "file" ? "files" : "any"}`, { kind: undefined, run: true }));
      if (chips.length) parts.push(h("div", { class: "chips", "aria-label": "Active filters" }, chips));
    } else if (v?.top) {
      const t = v.top;
      const kind = this.topKind();
      const noun = kind === "dir" ? (t.total_count === 1 ? "folder" : "folders") : t.total_count === 1 ? "file" : "files";
      const u = this.untouched();
      parts.push(
        h(
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
        ),
      );
    } else if (v?.diff) {
      const d = v.diff;
      parts.push(
        h(
          "div",
          { class: "context" },
          h(
            "p",
            null,
            `vs ${fmt.dateTime(d.prev_scanned_at)} (gen ${d.prev_generation ?? "?"}) · depth 1 · files included · `,
            h("span", { class: d.delta > 0 ? "grow" : d.delta < 0 ? "shrink" : "", text: `${d.delta > 0 ? "▲ " : d.delta < 0 ? "▼ " : ""}${fmt.delta(d.delta)}` }),
            " here",
          ),
          d.partial
            ? h("span", {
                class: "badge warn",
                title: d.refreshed.map((x) => `${x.path} at ${fmt.dateTime(x.scanned_at)}`).join("\n"),
                text: `partial: ${d.refreshed.map((x) => x.path).join(", ")} rescanned after prev`,
              })
            : null,
          d.roots_changed ? h("span", { class: "badge warn", title: "The scanned roots differ between the two scans", text: "roots changed" }) : null,
        ),
      );
    }
    replace(ui.summary, parts);
    ui.summary.hidden = parts.length === 0;
  }

  private stateBox(text: string, action: { label: string; run: () => void } | null, d?: ErrorDetail): HTMLElement {
    return h(
      "div",
      { class: "state" },
      h("p", { text }),
      action ? h("button", { type: "button", class: "outline", onclick: action.run, text: action.label }) : null,
      d ? details(d.code, d.exit, d.message) : null,
    );
  }

  private emptyState(): HTMLElement | null {
    const v = this.view;
    if (!v) return this.bodyState;
    if (this.rows.length > 0) return null;
    const f = v.folder;
    const up = parentOf(f.path) !== null ? { label: "Up one level", run: () => this.up() } : null;
    if (this.allRows.length > 0) {
      return this.stateBox("Everything here is hidden until the next scan.", { label: "Show hidden", run: () => this.toggleHidden() });
    }
    switch (this.route.mode) {
      case "browse": {
        if (f.kind === "mount" || f.mount) {
          const m = f.mount;
          const used = m && m.used !== null && m.total !== null ? `, used ${fmt.size(m.used)} of ${fmt.size(m.total)}` : "";
          const root = m?.root;
          return this.stateBox(
            `Not scanned here: separate filesystem (${m?.fs_type ?? "unknown"}${used}).`,
            root ? { label: "Open this filesystem", run: () => this.navigate({ path: root, sel: undefined }) } : null,
          );
        }
        if (f.denied) {
          return this.stateBox("Contents unavailable: this folder could not be read as root.", {
            label: `Copy: ${SCAN_COMMAND}`,
            run: () => void copy(SCAN_COMMAND, "Command copied"),
          });
        }
        return this.stateBox("Nothing here in this snapshot.", up);
      }
      case "top":
        return this.stateBox(`Nothing this large within ${f.path}.`, null);
      case "find":
        if (v.idle) return this.stateBox("Fill in a name pattern or a filter, then press Find.", null);
        return this.stateBox(`No matches within ${f.path}.`, {
          label: "Clear filters",
          run: () => this.navigate({ name: undefined, icase: undefined, before: undefined, after: undefined, min: undefined, kind: undefined, run: true }),
        });
      case "changes":
        return this.stateBox("Nothing changed here since the previous scan.", up);
    }
  }

  private headerCell(c: Column): HTMLElement {
    const r = this.route;
    const active = r.mode === "browse" && c.sort !== undefined && (r.sort ?? "allocated") === c.sort;
    const th = h("th", {
      scope: "col",
      class: c.cls,
      title: c.title ?? null,
      "aria-sort": active ? (c.sort === "mtime" ? "ascending" : "descending") : null,
    });
    if (r.mode === "browse" && c.sort) {
      th.appendChild(
        h(
          "button",
          { type: "button", class: active ? "sorted" : "", onclick: () => this.navigate({ sort: c.sort === "allocated" ? undefined : c.sort, sel: this.selected?.path }) },
          c.label,
          active ? h("span", { "aria-hidden": "true", text: c.sort === "mtime" ? " ▴" : " ▾" }) : null,
        ),
      );
    } else {
      th.textContent = c.label;
    }
    return th;
  }

  private renderTable(focus = false): void {
    const ui = this.ui!;
    const mode = this.route.mode;
    const cols = columns(mode, this.topKind());
    const ctx = { now: now(), folderSize: this.view?.folder.allocated ?? 0 };
    const state = this.emptyState();
    ui.table.className = `grid mode-${mode}${mode === "top" ? ` top-${this.topKind()}` : ""}`;
    if (state) {
      replace(ui.table);
      ui.wrap.hidden = true;
      replace(ui.body, state);
      ui.body.hidden = false;
      return;
    }
    ui.wrap.hidden = false;
    ui.body.hidden = true;
    replace(ui.body);
    const thead = h("thead", null, h("tr", null, cols.map((c) => this.headerCell(c))));
    const tbody = h("tbody", null);
    this.rows.forEach((r, i) => {
      const cls = [i === this.sel ? "sel" : "", r.kind === "dir" ? "dir" : "", r.status === "gone" ? "ghost" : "", r.hidden ? "hiddenrow" : ""]
        .filter(Boolean)
        .join(" ");
      const tr = h("tr", { tabindex: i === this.sel ? "0" : "-1", "aria-selected": String(i === this.sel), class: cls, "data-i": String(i) });
      if (mode === "browse" && ctx.folderSize > 0 && !r.mount) {
        tr.style.setProperty("--w", `${Math.min(100, (r.allocated / ctx.folderSize) * 100).toFixed(2)}%`);
      }
      for (const c of cols) tr.appendChild(h("td", { class: c.cls }, c.cell(r, ctx)));
      tr.addEventListener("click", () => {
        this.selectIdx(i, false);
        if (narrow()) this.openSheet();
      });
      tr.addEventListener("dblclick", () => this.open(r));
      tbody.appendChild(tr);
    });
    replace(ui.table, thead, tbody);
    if (focus) this.focusRow();
  }

  private focusRow(): void {
    const tr = this.ui?.table.querySelector<HTMLElement>(`tbody tr[data-i="${this.sel}"]`);
    tr?.focus({ preventScroll: false });
  }

  private renderSelection(focus = false): void {
    const ui = this.ui;
    if (!ui) return;
    for (const tr of ui.table.querySelectorAll<HTMLElement>("tbody tr")) {
      const on = Number(tr.dataset.i) === this.sel;
      tr.classList.toggle("sel", on);
      tr.setAttribute("aria-selected", String(on));
      tr.tabIndex = on ? 0 : -1;
    }
    if (focus) this.focusRow();
    this.renderInspector();
  }

  private host(): InspectorHost {
    return {
      route: this.route,
      scope: this.scope,
      folderSize: this.view?.folder.allocated ?? 0,
      now: now(),
      biggest: (p) => this.biggest(p),
      hide: (r) => this.hide(r),
      unhide: (p) => this.unhide(p),
      open: (r) => this.open(r),
      showInFolder: (r) => this.showInFolder(r),
      showUnreadable: () => {
        this.unreadableOpen = true;
        this.noticesExpanded = true;
        this.renderNotices();
        this.ui?.notices.scrollIntoView({ block: "nearest" });
      },
    };
  }

  private biggest(path: string): Promise<Entry[] | null> {
    const sid = this.status?.snapshot_id ?? "";
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

  private renderInspector(): void {
    const ui = this.ui!;
    const content = inspector(this.host(), this.view ? this.selected : null);
    const close = h("button", { type: "button", class: "sheet-close outline", onclick: () => this.closeSheet(true), text: "Close" });
    // <aside> stays the Details landmark; the panel inside is the sheet's dialog
    const panel = h("div", { class: "panel" }, h("div", { class: "handle", "aria-hidden": "true" }), close, ...content);
    this.sheetRole(panel, this.sheetOpen);
    replace(ui.aside, panel);
  }

  private sheetRole(panel: Element | null, on: boolean): void {
    if (!panel) return;
    if (on) {
      panel.setAttribute("role", "dialog");
      panel.setAttribute("aria-modal", "true");
      panel.setAttribute("aria-label", "Details");
    } else {
      for (const a of ["role", "aria-modal", "aria-label"]) panel.removeAttribute(a);
    }
  }

  private openSheet(): void {
    if (!this.ui) return;
    this.sheetOpen = true;
    this.ui.aside.classList.add("open");
    this.sheetRole(this.ui.aside.firstElementChild, true);
    this.ui.aside.querySelector<HTMLElement>(".sheet-close")?.focus();
    // keep the chosen row above the sheet
    this.ui.main.classList.add("sheet-open");
    this.ui.table.querySelector(`tbody tr[data-i="${this.sel}"]`)?.scrollIntoView({ block: "start" });
  }

  private closeSheet(focus: boolean): void {
    if (!this.ui || !this.sheetOpen) return;
    this.sheetOpen = false;
    this.ui.aside.classList.remove("open");
    this.sheetRole(this.ui.aside.firstElementChild, false);
    this.ui.main.classList.remove("sheet-open");
    if (focus) this.focusRow();
  }

  private renderMore(): void {
    const ui = this.ui!;
    const v = this.view;
    const resp = v?.ls ?? v?.top ?? v?.find ?? v?.diff;
    // a diff page that already reached unchanged rows holds every change
    const allChanges = v?.diff?.entries.at(-1)?.status === "same";
    if (!resp || !resp.truncated || this.rows.length === 0 || allChanges) {
      replace(ui.more);
      ui.more.hidden = true;
      return;
    }
    ui.more.hidden = false;
    replace(
      ui.more,
      `Showing ${fmt.count(resp.entries.length)} of ${fmt.count(resp.total_count)} · `,
      h("button", { type: "button", class: "link", onclick: () => this.showMore(), text: "Show more" }),
    );
  }
}
