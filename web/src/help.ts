// "?" overlay: keys and recent messages; traps focus, restores it on close.

import { h } from "./dom";
import { recentMessages } from "./toast";

const KEYS: [string, string][] = [
  ["↑ ↓", "move"],
  ["→ Enter", "open folder"],
  ["← Backspace", "up one level"],
  ["Home End PgUp PgDn", "jump"],
  ["t", "Top files"],
  ["/", "Find"],
  ["d", "Changes"],
  ["Esc", "back to Browse, close"],
  ["h", "hide until next scan"],
  ["r", "load the latest snapshot (when offered)"],
  ["?", "this help"],
];

export function trapFocus(box: HTMLElement, e: KeyboardEvent): void {
  if (e.key !== "Tab") return;
  const f = [...box.querySelectorAll<HTMLElement>("button, [href], input, select, [tabindex='0']")].filter(
    (x) => !x.hasAttribute("disabled"),
  );
  const first = f[0];
  const last = f[f.length - 1];
  if (!first || !last) return;
  if (e.shiftKey && document.activeElement === first) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && document.activeElement === last) {
    e.preventDefault();
    first.focus();
  }
}

export function openHelp(host: HTMLElement): void {
  const back = document.activeElement as HTMLElement | null;
  const close = () => {
    host.replaceChildren();
    host.hidden = true;
    back?.focus();
  };
  const recent = recentMessages();
  const box = h(
    "div",
    { class: "dialog", role: "dialog", "aria-modal": "true", "aria-labelledby": "help-title" },
    h("h2", { id: "help-title", text: "Keys" }),
    h(
      "dl",
      { class: "keys-list" },
      KEYS.flatMap(([k, v]) => [h("dt", { class: "mono", text: k }), h("dd", { text: v })]),
    ),
    h("h2", { text: "Recent messages" }),
    recent.length
      ? h(
          "ul",
          { class: "recent" },
          recent.map((m) => h("li", { text: m })),
        )
      : h("p", { class: "muted", text: "None yet." }),
    h("button", { type: "button", class: "primary", onclick: close, text: "Close · Esc" }),
  );
  box.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else {
      trapFocus(box, e);
    }
  });
  host.replaceChildren(h("div", { class: "scrim", onclick: close }), box);
  host.hidden = false;
  box.querySelector<HTMLElement>("button")?.focus();
}
