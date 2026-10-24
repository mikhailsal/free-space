// What the page's modules share: the view a mode's controller fetches and the
// controller itself (D1), the clock, and the box a body state is drawn in (D7).

import * as api from "./api";
import { ApiError } from "./api";
import type { App } from "./app";
import { h } from "./dom";
import { type Action, details } from "./notices";
import { parentOf, type Route } from "./router";
import type { Row } from "./rows";
import type { DiffResponse, Entry, ErrorDetail, FindResponse, LsResponse, TopResponse } from "./types";

export type Page = "boot" | "main" | "nosnap" | "session" | "access";

export interface View {
  route: Route;
  folder: Entry;
  ls?: LsResponse;
  top?: TopResponse;
  find?: FindResponse;
  diff?: DiffResponse;
  /** Find mode before the first submit. */
  idle?: boolean;
}

/** One per mode. It fetches through the request coordinator (load.ts), which applies the view; it never applies one itself (D19). */
export interface ViewController {
  fetch(r: Route, sid: string, signal: AbortSignal): Promise<View>;
  rows(v: View, scope: string): Row[];
  /** The summary above the table; none when empty. */
  summary(v: View, app: App): Node[];
  /** The form under the tabs; null hides the form row. */
  form(app: App): HTMLElement | null;
  /** What the body says when the view has no rows at all. */
  empty(v: View, app: App): HTMLElement;
}

export const DEFAULT_LIMIT = 30;
export const now = () => Date.now() / 1000;
export const narrow = () => window.matchMedia("(max-width: 700px)").matches;

export function pathParams(r: Route): api.Params {
  return r.raw ? { path_raw: r.raw } : { path: r.path };
}

/** An answer from another snapshot than the one on screen. */
export function sameSnapshot(id: string, sid: string): void {
  if (id !== sid) throw new ApiError("api", 409, { code: "ROTATED", exit: 4, message: "snapshot rotated", fix: null, docs: "" });
}

export async function folderEntry(r: Route, sid: string, signal: AbortSignal): Promise<Entry> {
  const ls = await api.get<LsResponse>("ls", { ...pathParams(r), limit: 1, snapshot: sid }, signal);
  return ls.entry;
}

export function stateBox(text: string, action: Action | null, d?: ErrorDetail): HTMLElement {
  return h(
    "div",
    { class: "state" },
    h("p", { text }),
    action ? h("button", { type: "button", class: "outline", onclick: action.run, text: action.label }) : null,
    d ? details(d.code, d.exit, d.message) : null,
  );
}

export function upAction(app: App, path: string): Action | null {
  return parentOf(path) !== null ? { label: "Up one level", run: () => app.cmd.up() } : null;
}
