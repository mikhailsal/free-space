// A tiny element builder. No innerHTML anywhere: the CSP is `default-src 'self'`.

type Child = Node | string | number | null | undefined | false;
type Value = string | number | boolean | null | undefined;
export type Attrs = { [k: string]: Value | EventListener | Record<string, string> };

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs?: Attrs | null,
  ...children: (Child | Child[])[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) setAttr(el, k, v);
  }
  append(el, children);
  return el;
}

/** `on…` takes a listener, `class` and `text` set those properties, `style` takes a map; null, undefined and false set nothing. */
function setAttr(el: HTMLElement, k: string, v: Attrs[string]): void {
  if (v === null || v === undefined || v === false) return;
  if (typeof v === "function") {
    el.addEventListener(k.slice(2), v as EventListener);
  } else if (k === "class") {
    el.className = String(v);
  } else if (k === "style" && typeof v === "object") {
    for (const [p, s] of Object.entries(v)) el.style.setProperty(p, s);
  } else if (k === "text") {
    el.textContent = String(v);
  } else {
    el.setAttribute(k, v === true ? "" : String(v));
  }
}

export function append(el: Node, children: (Child | Child[])[]): void {
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    el.appendChild(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

export function clear(el: Element): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

export function replace(el: Element, ...children: (Child | Child[])[]): void {
  clear(el);
  append(el, children);
}

export function $(sel: string, root: ParentNode = document): HTMLElement | null {
  return root.querySelector(sel);
}
