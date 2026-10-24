// The results table (D6): headers, rows, the selected row, the body state when
// there are no rows (D7) and "Show more".

import type { App } from "./app";
import { h, replace } from "./dom";
import * as fmt from "./format";
import { controllers } from "./modes";
import { narrow, now, stateBox } from "./page";
import { topKind } from "./top";
import { type Column, columns } from "./views";

export class Table {
  constructor(private app: App) {}

  private emptyState(): HTMLElement | null {
    const app = this.app;
    const v = app.view;
    if (!v) return app.bodyState;
    if (app.rows.length > 0) return null;
    if (app.allRows.length > 0) {
      return stateBox("Everything here is hidden until the next scan.", { label: "Show hidden", run: () => app.cmd.toggleHidden() });
    }
    return controllers[app.route.mode].empty(v, app);
  }

  private headerCell(c: Column): HTMLElement {
    const app = this.app;
    const r = app.route;
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
          {
            type: "button",
            class: active ? "sorted" : "",
            onclick: () => app.cmd.navigate({ sort: c.sort === "allocated" ? undefined : c.sort, sel: app.selected?.path }),
          },
          c.label,
          active ? h("span", { "aria-hidden": "true", text: c.sort === "mtime" ? " ▴" : " ▾" }) : null,
        ),
      );
    } else {
      th.textContent = c.label;
    }
    return th;
  }

  render(focus = false): void {
    const app = this.app;
    const ui = app.ui!;
    const mode = app.route.mode;
    const cols = columns(mode, topKind(app.route));
    const ctx = { now: now(), folderSize: app.view?.folder.allocated ?? 0 };
    const state = this.emptyState();
    ui.table.className = `grid mode-${mode}${mode === "top" ? ` top-${topKind(app.route)}` : ""}`;
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
    const thead = h(
      "thead",
      null,
      h(
        "tr",
        null,
        cols.map((c) => this.headerCell(c)),
      ),
    );
    const tbody = h("tbody", null);
    app.rows.forEach((r, i) => {
      const cls = [
        i === app.sel ? "sel" : "",
        r.kind === "dir" ? "dir" : "",
        r.status === "gone" ? "ghost" : "",
        r.hidden ? "hiddenrow" : "",
      ]
        .filter(Boolean)
        .join(" ");
      const tr = h("tr", {
        tabindex: i === app.sel ? "0" : "-1",
        "aria-selected": String(i === app.sel),
        class: cls,
        "data-i": String(i),
      });
      if (mode === "browse" && ctx.folderSize > 0 && !r.mount) {
        tr.style.setProperty("--w", `${Math.min(100, (r.allocated / ctx.folderSize) * 100).toFixed(2)}%`);
      }
      for (const c of cols) tr.appendChild(h("td", { class: c.cls }, c.cell(r, ctx)));
      tr.addEventListener("click", () => {
        app.cmd.selectIdx(i, false);
        if (narrow()) app.sheet.show();
      });
      tr.addEventListener("dblclick", () => app.cmd.open(r));
      tbody.appendChild(tr);
    });
    replace(ui.table, thead, tbody);
    if (focus) this.focusRow();
  }

  focusRow(): void {
    const tr = this.app.ui?.table.querySelector<HTMLElement>(`tbody tr[data-i="${this.app.sel}"]`);
    tr?.focus({ preventScroll: false });
  }

  renderSelection(focus = false): void {
    const app = this.app;
    const ui = app.ui;
    if (!ui) return;
    for (const tr of ui.table.querySelectorAll<HTMLElement>("tbody tr")) {
      const on = Number(tr.dataset.i) === app.sel;
      tr.classList.toggle("sel", on);
      tr.setAttribute("aria-selected", String(on));
      tr.tabIndex = on ? 0 : -1;
    }
    if (focus) this.focusRow();
    app.sheet.render();
  }

  renderMore(): void {
    const app = this.app;
    const ui = app.ui!;
    const v = app.view;
    const resp = v?.ls ?? v?.top ?? v?.find ?? v?.diff;
    // a diff page that already reached unchanged rows holds every change
    const allChanges = v?.diff?.entries.at(-1)?.status === "same";
    if (!resp || !resp.truncated || app.rows.length === 0 || allChanges) {
      replace(ui.more);
      ui.more.hidden = true;
      return;
    }
    ui.more.hidden = false;
    replace(
      ui.more,
      `Showing ${fmt.count(resp.entries.length)} of ${fmt.count(resp.total_count)} · `,
      h("button", { type: "button", class: "link", onclick: () => app.cmd.showMore(), text: "Show more" }),
    );
  }
}
