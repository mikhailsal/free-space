// Keyboard (Pass 6): the page's keys; the commands do the work.

import type { App } from "./app";
import { openHelp } from "./help";

export function onKey(app: App, e: KeyboardEvent): void {
  if (app.page !== "main" || !app.ui || !app.ui.overlay.hidden) return;
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  const t = e.target as HTMLElement;
  const typing = t.closest("input, select, textarea") !== null;
  const cmd = app.cmd;
  if (e.key === "Escape") {
    if (app.sheet.isOpen) {
      app.sheet.close(true);
    } else if (app.route.mode !== "browse") {
      if (typing) t.blur();
      cmd.setMode("browse");
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
      cmd.move(1);
      break;
    case "ArrowUp":
      cmd.move(-1);
      break;
    case "PageDown":
      cmd.move(10);
      break;
    case "PageUp":
      cmd.move(-10);
      break;
    case "Home":
      cmd.selectIdx(0);
      break;
    case "End":
      cmd.selectIdx(app.rows.length - 1);
      break;
    case "ArrowRight":
    case "Enter":
      if (app.selected) cmd.open(app.selected);
      break;
    case "ArrowLeft":
    case "Backspace":
      cmd.up();
      break;
    case "t":
      cmd.setMode("top");
      break;
    case "d":
      cmd.setMode("changes");
      break;
    case "/":
      if (app.route.mode === "find") document.getElementById("f-name")?.focus();
      else cmd.setMode("find");
      break;
    case "h":
      cmd.hide(app.selected);
      break;
    case "r":
      if (app.notices.newerVisible()) void app.loader.loadLatest();
      break;
    case "?":
      openHelp(app.ui.overlay);
      break;
    default:
      return;
  }
  e.preventDefault();
}
