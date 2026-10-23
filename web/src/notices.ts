// The single notice slot (D5): only the highest-priority notice renders in
// full; the rest collapse into a "+N notice" chip that expands them.

import { h, replace } from "./dom";

export interface Action {
  label: string;
  run: () => void;
}

export interface Notice {
  id: string;
  /** Lower = more urgent (D5 table). */
  priority: number;
  tone: "danger" | "neutral";
  body: (string | Node)[];
  actions: Action[];
  extra?: Node | null;
  dismiss?: () => void;
}

export function renderNotices(slot: HTMLElement, list: Notice[], expanded: boolean, toggle: () => void): void {
  const sorted = [...list].sort((a, b) => a.priority - b.priority);
  if (sorted.length === 0) {
    replace(slot);
    slot.hidden = true;
    return;
  }
  slot.hidden = false;
  const shown = expanded ? sorted : sorted.slice(0, 1);
  const rest = sorted.length - 1;
  replace(
    slot,
    shown.map((n, i) =>
      h(
        "div",
        {
          class: `notice ${n.tone}`,
          role: n.tone === "danger" ? "alert" : "status",
          "data-notice": n.id,
        },
        n.tone === "danger" ? h("span", { class: "mark", "aria-hidden": "true", text: "!" }) : null,
        h("div", { class: "text" }, h("p", null, ...n.body), n.extra ?? null),
        h(
          "div",
          { class: "actions" },
          n.actions.map((a) => h("button", { type: "button", class: "outline", onclick: a.run, text: a.label })),
          n.dismiss ? h("button", { type: "button", class: "link", onclick: n.dismiss, text: "Dismiss" }) : null,
          i === 0 && rest > 0
            ? h("button", {
                type: "button",
                class: "chip-link",
                "aria-expanded": String(expanded),
                onclick: toggle,
                text: expanded ? "Show one" : `+${rest} ${rest === 1 ? "notice" : "notices"}`,
              })
            : null,
        ),
      ),
    ),
  );
}

/** Raw API error under a collapsed disclosure (D7). */
export function details(code: string, exit: number, message: string): HTMLElement {
  return h(
    "details",
    { class: "raw" },
    h("summary", null, "details"),
    h("code", null, `${code} (exit ${exit}): ${message}`),
  );
}
