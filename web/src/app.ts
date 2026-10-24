// The page: one results table in four modes (D6), the inspector, the notice
// slot (D5), full-page states (D7, D12) and snapshot refresh continuity (D10).
// App holds the page's state; each part is its own module (D1): status polling
// (poll.ts), the request coordinator (load.ts), commands and keys (commands.ts,
// keys.ts), one controller per mode (modes.ts), notices (page-notices.ts) and
// the layout (layout.ts, table.ts, sheet.ts).

import * as api from "./api";
import { Commands } from "./commands";
import { Hidden } from "./hidden";
import { onKey } from "./keys";
import { Layout, type Ui } from "./layout";
import { Loader } from "./load";
import type { Page, View } from "./page";
import { PageNotices } from "./page-notices";
import { Poller } from "./poll";
import { format, parse, type Place, type Route } from "./router";
import type { Row } from "./rows";
import { Sheet } from "./sheet";
import { Table } from "./table";
import { mountToasts } from "./toast";
import type { ErrorDetail, Status } from "./types";

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
  readonly poller = new Poller(this);
  readonly loader = new Loader(this);
  readonly cmd = new Commands(this);
  readonly layout = new Layout(this);
  readonly notices = new PageNotices(this);
  readonly table = new Table(this);
  readonly sheet = new Sheet(this);

  constructor(readonly root: HTMLElement) {
    this.route = parse(location.hash);
  }

  async start(): Promise<void> {
    const t = this.route.t || api.storedToken();
    if (t) api.setToken(t);
    if (this.route.t) {
      // from here on the token lives in sessionStorage only, never in the
      // address bar, history or a copied link (T7)
      this.route.t = undefined;
      history.replaceState(null, "", format(this.route));
    }
    mountToasts(document.body);
    window.addEventListener("hashchange", () => this.cmd.apply(parse(location.hash)));
    document.addEventListener("keydown", (e) => onKey(this, e));
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) void this.poller.poll();
    });
    window.addEventListener("focus", () => void this.poller.poll());
    window.setInterval(() => this.poller.tick(), 1000);
    if (!api.hasToken()) {
      this.poller.showSession();
      return;
    }
    await this.poller.poll();
  }

  /** The first status with a snapshot: draw the page, then load the route. */
  enterMain(): void {
    const s = this.status!;
    this.page = "main";
    this.hidden.load(s.host, s.snapshot_id!);
    this.layout.build();
    if (!this.route.path) {
      this.route.path = s.roots[0]?.path ?? "/";
      history.replaceState(null, "", format(this.route));
    }
    void this.loader.load();
  }

  get scope(): string {
    return this.view?.folder.path ?? this.route.path;
  }

  get place(): Place {
    const f = this.view?.folder;
    return f ? { path: f.path, raw: f.path_raw } : { path: this.route.path, raw: this.route.raw };
  }

  get selected(): Row | null {
    return this.rows[this.sel] ?? null;
  }

  renderAll(focus = false): void {
    if (!this.ui) return;
    this.layout.renderIdentity();
    this.notices.render();
    this.layout.renderFolder();
    this.layout.renderTabs();
    this.layout.renderControls();
    this.layout.renderSummary();
    this.table.render(focus);
    this.sheet.render();
    this.table.renderMore();
  }
}
