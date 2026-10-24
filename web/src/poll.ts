// Status polling (D10, D12) and the full-page states it leads to: no snapshot
// yet, session ended, no access.

import * as api from "./api";
import { ApiError } from "./api";
import type { App } from "./app";
import { now } from "./page";
import * as pages from "./states";
import type { Status } from "./types";

export class Poller {
  private polling = false;
  private timer = 0;

  constructor(private app: App) {}

  async poll(): Promise<void> {
    if (this.polling) return;
    this.polling = true;
    window.clearTimeout(this.timer);
    try {
      const s = await api.get<Status>("status");
      this.app.failure = false;
      this.onStatus(s);
    } catch (e) {
      this.onFailure(e);
    } finally {
      this.polling = false;
      this.schedule();
    }
  }

  private schedule(): void {
    window.clearTimeout(this.timer);
    if (document.hidden) return; // resumes on visibilitychange
    const page = this.app.page;
    const ms = page === "nosnap" ? 5000 : page === "session" ? 10000 : 60000;
    this.timer = window.setTimeout(() => void this.poll(), ms);
  }

  private onStatus(s: Status): void {
    const app = this.app;
    app.latest = s;
    app.latestAt = now();
    if (s.generation === null || s.snapshot_id === null) {
      app.status = s;
      app.statusAt = app.latestAt;
      this.showNoSnapshot(s);
      return;
    }
    if (app.page !== "main") {
      app.status = s;
      app.statusAt = app.latestAt;
      app.enterMain();
      return;
    }
    app.layout.renderIdentity();
    app.notices.render();
    app.layout.renderTabs();
  }

  onFailure(e: unknown): void {
    const app = this.app;
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
      app.failure = true;
      if (app.page === "main") app.notices.render();
      return;
    }
    const d = e.detail;
    if (!d) {
      console.error(e);
      return;
    }
    switch (d.code) {
      case "NO_SNAPSHOT":
        void this.poll();
        return;
      case "SNAPSHOT_ACCESS":
        app.page = "access";
        app.ui = null;
        app.root.replaceChildren(pages.noAccess(d));
        return;
      case "ROTATED":
        app.rotated = true;
        void this.poll();
        break;
      default:
        app.errorNotice = d;
    }
    if (app.page === "main") app.notices.render();
    else {
      app.page = "access";
      app.root.replaceChildren(pages.fatal(d));
    }
  }

  private showNoSnapshot(s: Status): void {
    const app = this.app;
    app.page = "nosnap";
    app.ui = null;
    app.root.replaceChildren(pages.noSnapshot(s, now(), app.latestAt));
  }

  showSession(): void {
    const app = this.app;
    if (app.page === "session") return;
    app.loader.stop();
    app.page = "session";
    app.ui = null;
    app.root.replaceChildren(pages.sessionEnded(app.route.path || "/"));
    this.schedule();
  }

  tick(): void {
    const app = this.app;
    if (app.page === "nosnap" && app.latest?.scanning) {
      const el = document.getElementById("fp-scan");
      if (el) el.textContent = pages.scanningLine(app.latest, now(), app.latestAt);
    } else if (app.page === "main" && app.latest?.scanning) {
      app.layout.renderIdentity();
    }
  }
}
