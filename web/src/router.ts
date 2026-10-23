// Hash state: `#/home/tass?mode=find&name=*.iso`. The token arrives as `t=` in
// the fragment (never sent to a server, OV-14), moves to sessionStorage on load
// and is never written back into the URL (T7).

export type Mode = "browse" | "top" | "find" | "changes";

export interface Route {
  /** Folder (scope); "" = the first scanned root. */
  path: string;
  /** Hex of a non-UTF-8 path. */
  raw?: string;
  mode: Mode;
  /** Selected row (full path). */
  sel?: string;
  sort?: string;
  limit?: number;
  /** top: file | dir; find: file | dir | any. */
  kind?: string;
  /** top threshold / find min size, e.g. 100M. */
  min?: string;
  /** top: months untouched, 0 = off. */
  untouched?: number;
  name?: string;
  icase?: boolean;
  before?: string;
  after?: string;
  /** find was submitted (also with no filters). */
  run?: boolean;
  t?: string;
}

const MODES: Mode[] = ["browse", "top", "find", "changes"];

export function encodePath(p: string): string {
  return p
    .split("/")
    .map((s) => encodeURIComponent(s))
    .join("/");
}

function decodePath(p: string): string {
  try {
    return p
      .split("/")
      .map((s) => decodeURIComponent(s))
      .join("/");
  } catch {
    return p;
  }
}

export function parse(hash: string): Route {
  let h = hash.startsWith("#") ? hash.slice(1) : hash;
  let path = "";
  if (h.startsWith("/")) {
    const q = h.indexOf("?");
    path = decodePath(q < 0 ? h : h.slice(0, q));
    h = q < 0 ? "" : h.slice(q + 1);
  } else if (h.startsWith("?")) {
    h = h.slice(1);
  }
  const p = new URLSearchParams(h);
  const mode = p.get("mode") as Mode | null;
  const num = (k: string): number | undefined => {
    const v = p.get(k);
    if (v === null || v === "") return undefined;
    const n = Number(v);
    return Number.isFinite(n) && n >= 0 ? Math.floor(n) : undefined;
  };
  const str = (k: string): string | undefined => p.get(k) ?? undefined;
  return {
    path,
    raw: str("raw"),
    mode: mode && MODES.includes(mode) ? mode : "browse",
    sel: str("sel"),
    sort: str("sort"),
    limit: num("limit"),
    kind: str("kind"),
    min: str("min"),
    untouched: num("untouched"),
    name: str("name"),
    icase: p.get("icase") === "1",
    before: str("before"),
    after: str("after"),
    run: p.get("run") === "1",
    t: str("t"),
  };
}

export function format(r: Route): string {
  const p = new URLSearchParams();
  const set = (k: string, v: string | number | boolean | undefined) => {
    if (v === undefined || v === "" || v === false) return;
    p.set(k, v === true ? "1" : String(v));
  };
  set("raw", r.raw);
  if (r.mode !== "browse") set("mode", r.mode);
  set("kind", r.kind);
  set("min", r.min);
  set("untouched", r.untouched);
  set("name", r.name);
  set("icase", r.icase);
  set("before", r.before);
  set("after", r.after);
  set("run", r.run);
  set("sort", r.sort);
  set("limit", r.limit);
  set("sel", r.sel);
  const q = p.toString().replace(/%2F/g, "/");
  return `#${encodePath(r.path || "/")}${q ? `?${q}` : ""}`;
}

/** Same view (path, mode and filters), ignoring selection and token. */
export function sameView(a: Route, b: Route): boolean {
  const strip = (r: Route) => format({ ...r, sel: undefined, t: undefined });
  return strip(a) === strip(b);
}

/** A folder as the API names it: the lossy `path`, plus its bytes in hex when
 * the name is not valid UTF-8. */
export interface Place {
  path: string;
  raw?: string;
}

export function hexBytes(raw: string): Uint8Array {
  const b = new Uint8Array(raw.length >> 1);
  for (let i = 0; i < b.length; i++) b[i] = parseInt(raw.slice(2 * i, 2 * i + 2), 16);
  return b;
}

const exact = new TextDecoder("utf-8", { fatal: true });
const lossy = new TextDecoder("utf-8");

function placeOf(b: Uint8Array): Place {
  try {
    return { path: exact.decode(b) };
  } catch {
    return { path: lossy.decode(b), raw: Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("") };
  }
}

/** "/" down to `p`, each folder with its raw form when it needs one, so going
 * up from a non-UTF-8 folder asks for the real bytes, not the lossy name. */
export function ancestors(p: Place): Place[] {
  const out: Place[] = [{ path: "/" }];
  if (p.raw) {
    const b = hexBytes(p.raw);
    for (let i = 1; i < b.length; i++) if (b[i] === 0x2f) out.push(placeOf(b.subarray(0, i)));
    out.push(placeOf(b));
    return out;
  }
  const segs = p.path === "/" || p.path === "" ? [] : p.path.split("/").slice(1);
  segs.forEach((_, i) => out.push({ path: "/" + segs.slice(0, i + 1).join("/") }));
  return out;
}

export function parentPlace(p: Place): Place | null {
  const a = ancestors(p);
  return a.length > 1 ? a[a.length - 2] : null;
}

export function parentOf(path: string): string | null {
  if (path === "/" || path === "") return null;
  const i = path.lastIndexOf("/");
  return i <= 0 ? "/" : path.slice(0, i);
}

export function join(dir: string, name: string): string {
  return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
}

/** Path of `p` relative to `scope` ("Downloads/ubuntu.iso"). */
export function relative(p: string, scope: string): string {
  if (scope === "/") return p.slice(1);
  return p.startsWith(scope + "/") ? p.slice(scope.length + 1) : p;
}

export function isUnder(p: string, dir: string): boolean {
  return p === dir || (dir === "/" ? p.startsWith("/") : p.startsWith(dir + "/"));
}
