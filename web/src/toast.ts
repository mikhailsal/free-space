// Toasts: role=status, 6 s, one at a time; kept in a "recent messages" list
// the help overlay shows (Pass 6).

import { h, replace } from "./dom";

const recent: string[] = [];
let region: HTMLElement | null = null;
let timer: number | undefined;

export function recentMessages(): readonly string[] {
  return recent;
}

export function mountToasts(parent: HTMLElement): void {
  region = h("div", { class: "toasts", role: "status", "aria-live": "polite" });
  parent.appendChild(region);
}

export function toast(text: string, action?: { label: string; run: () => void }): void {
  recent.unshift(text);
  recent.length = Math.min(recent.length, 10);
  if (!region) return;
  window.clearTimeout(timer);
  const close = () => region && replace(region);
  const btn = action
    ? h("button", {
        class: "link",
        type: "button",
        onclick: () => {
          close();
          action.run();
        },
        text: action.label,
      })
    : null;
  replace(region, h("div", { class: "toast" }, h("span", null, text), btn));
  timer = window.setTimeout(close, 6000);
}

export async function copy(text: string, what = "Copied"): Promise<void> {
  let ok = false;
  try {
    await navigator.clipboard.writeText(text);
    ok = true;
  } catch {
    const ta = h("textarea", { class: "offscreen", "aria-hidden": "true" });
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    try {
      ok = document.execCommand("copy");
    } catch {
      ok = false;
    }
    ta.remove();
  }
  toast(ok ? `${what}: ${text}` : `Copy failed; select it by hand: ${text}`);
}
