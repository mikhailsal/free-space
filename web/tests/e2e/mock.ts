// A fake `/api/*` over an in-memory tree, answering with the same JSON shapes
// as `freespace serve` (src/types.ts). Each test builds one and installs it on
// its page; `requests` records every call for "no request while typing" checks,
// and `hold` keeps one request unanswered until the test releases it (D19).
import type { Page, Request, Route } from "@playwright/test";

export const TOKEN = "e2etoken";
export const NOW = 1_792_000_000; // 2026-10-15
const DAY = 86_400;
const G = 2 ** 30;
const M = 2 ** 20;

export interface Node {
  name: string;
  kind?: "dir" | "file";
  allocated?: number;
  mtime?: number;
  denied?: boolean;
  /** Entries below this folder beyond its modelled children (screenshots). */
  items?: number;
  children?: Node[];
}

interface Flat {
  path: string;
  node: Node;
  allocated: number;
  items: number;
  newest: number;
  errors: number;
}

const dir = (name: string, children: Node[], mtime = NOW - 10 * DAY): Node => ({ name, kind: "dir", children, mtime });
const file = (name: string, allocated: number, mtime = NOW - 40 * DAY): Node => ({ name, kind: "file", allocated, mtime });

/** /home/tass with a few folders and big files (the mockup's shape). */
export function homeTree(unreadable = true): Node {
  return dir("/", [
    dir("home", [
      dir("tass", [
        dir(".cache", [dir("huggingface", [file("model.safetensors", 61 * G)]), dir("pip", [file("wheels.tar", 22 * G, NOW - 300 * DAY)])]),
        dir("Downloads", [file("ubuntu.iso", 5 * G, NOW - 400 * DAY), file("movie.mp4", 3 * G), file("notes.txt", 4096)]),
        dir("old", [file("backup.tar.gz", 2 * G, NOW - 700 * DAY)]),
        dir("projects", [
          dir("app", [file("node.bin", 900 * M)]),
          ...(unreadable ? [{ name: "secret", kind: "dir" as const, denied: true, children: [] }] : []),
        ]),
        file("big.log", 700 * M, NOW - 2 * DAY),
      ]),
    ]),
  ]);
}

function flatten(root: Node): Map<string, Flat> {
  const out = new Map<string, Flat>();
  const walk = (n: Node, path: string): Flat => {
    let f: Flat;
    if (n.kind === "file") {
      f = { path, node: n, allocated: n.allocated ?? 0, items: 0, newest: n.mtime ?? 0, errors: 0 };
    } else {
      f = { path, node: n, allocated: 0, items: n.items ?? 0, newest: 0, errors: n.denied ? 1 : 0 };
      for (const c of n.children ?? []) {
        const cf = walk(c, path === "/" ? `/${c.name}` : `${path}/${c.name}`);
        f.allocated += cf.allocated;
        f.items += cf.items + 1;
        f.newest = Math.max(f.newest, cf.newest);
        f.errors += cf.errors;
      }
    }
    out.set(path, f);
    return f;
  };
  walk(root, "/");
  return out;
}

/** A request `hold` keeps back: `arrived` once the page sent it; `release` answers it and resolves when that is done. */
export interface Held {
  arrived: Promise<Request>;
  release: () => Promise<void>;
}

interface Hold {
  endpoint: string;
  match: (q: URLSearchParams) => boolean;
  arrive: (r: Request) => void;
  gate: Promise<void>;
  done: () => void;
}

/**
 * From here the page's AbortController.abort does nothing, so a request the page gives up on still gets its answer:
 * the request coordinator must then drop it by its own count of loads (D19).
 */
export async function loseAborts(page: Page): Promise<void> {
  await page.addInitScript(() => {
    AbortController.prototype.abort = () => {};
  });
}

function sizeArg(s: string | null): number {
  if (!s) return 0;
  const m = /^(\d+)([KMGT]?)$/i.exec(s.trim());
  if (!m) return NaN;
  const mul = { "": 1, K: 2 ** 10, M: 2 ** 20, G: 2 ** 30, T: 2 ** 40 }[m[2]!.toUpperCase()]!;
  return Number(m[1]) * mul;
}

function cutoff(s: string | null): number | null {
  if (!s) return null;
  const m = /^(\d+)(d|w|mo|y)$/.exec(s);
  if (m) return NOW - Number(m[1]) * { d: 1, w: 7, mo: 30, y: 365 }[m[2] as "d"]! * DAY;
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return Date.parse(`${s}T00:00:00`) / 1000;
  return NaN;
}

export class MockApi {
  constructor(unreadable = true) {
    this.tree = homeTree(unreadable);
  }

  host = "tass-vm";
  snapshotId = "snap-1";
  generation = 3;
  tree: Node;
  prevTree: Node | null = null;
  roots = ["/"];
  /** Status only: generation null (first run). */
  noSnapshot = false;
  scanning: { op: string; path: string; pid: number; since: number; elapsed_seconds: number } | null = null;
  lastError: { ts: number; op: string; message: string } | null = null;
  /** Every request answers 403 without JSON, like a restarted server. */
  sessionGone = false;
  /** Every request answers this error body. */
  failWith: { status: number; code: string; message: string; fix?: string } | null = null;
  requests: string[] = [];
  private holds: Hold[] = [];

  private flat(tree = this.tree) {
    return flatten(tree);
  }

  /** The node at this path of the tree (a test changes it in place). */
  node(path: string, tree = this.tree): Node {
    const f = this.flat(tree).get(path);
    if (!f) throw new Error(`mock: ${path} is not in the tree`);
    return f.node;
  }

  header() {
    const unknown = [...this.flat().values()].filter((f) => f.node.denied).map((f) => f.path);
    return {
      host: this.host,
      os: "linux",
      generation: this.generation,
      prev_generation: this.prevTree ? this.generation - 1 : null,
      scanned_at: NOW - 7 * 3600,
      age_seconds: 7 * 3600,
      schema_version: 1,
      snapshot_id: this.snapshotId,
      incomplete: unknown.length > 0,
      unknown_subtrees: unknown,
    };
  }

  entry(f: Flat) {
    const n = f.node;
    return {
      path: f.path,
      name: n.name === "/" ? "/" : n.name,
      kind: n.kind ?? "dir",
      allocated: f.allocated,
      apparent: f.allocated,
      mtime: n.mtime ?? NOW - 10 * DAY,
      newest_mtime: f.newest,
      nlink: 1,
      items: f.items,
      errors: f.errors,
      scanned_at: NOW - 7 * 3600,
      ...(n.denied ? { denied: true } : {}),
    };
  }

  status() {
    const h = this.header();
    const empty = this.noSnapshot;
    const root = this.flat().get("/")!;
    return {
      host: this.host,
      os: "linux",
      version: "0.1.0",
      snapshot_dir: "/var/lib/freespace",
      generation: empty ? null : h.generation,
      prev_generation: empty ? null : h.prev_generation,
      schema_version: empty ? null : 1,
      snapshot_id: empty ? null : this.snapshotId,
      scanned_at: empty ? null : h.scanned_at,
      age_seconds: empty ? null : h.age_seconds,
      entries: empty ? null : this.flat().size,
      prev: !empty && this.prevTree ? { generation: this.generation - 1, scanned_at: NOW - 31 * 3600 } : null,
      refreshed: [],
      roots: empty
        ? []
        : this.roots.map((p) => ({
            path: p,
            fs_type: "ext4",
            allocated: root.allocated,
            items: root.items,
            fs_used: 97 * G,
            fs_total: 100 * G,
            fs_avail: 3 * G,
          })),
      fs: [],
      incomplete: !empty && h.incomplete,
      unknown_subtrees: empty ? [] : h.unknown_subtrees,
      last_error: this.lastError,
      last_success_ts: this.lastError ? this.lastError.ts - DAY : null,
      warnings: [],
      scanning: this.scanning,
    };
  }

  private error(route: Route, status: number, code: string, message: string, fix: string | null = null) {
    return route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify({
        error: { code, exit: 1, message, fix, recovery: null, context: {}, docs: "" },
      }),
    });
  }

  private target(q: URLSearchParams, route: Route): Flat | null {
    const path = q.get("path") ?? "/";
    const f = this.flat().get(path);
    if (!f) {
      void this.error(route, 404, "PATH_NOT_FOUND", `${path} is not in the snapshot`);
      return null;
    }
    return f;
  }

  private children(path: string, tree = this.tree): Flat[] {
    const flat = this.flat(tree);
    const f = flat.get(path);
    return (f?.node.children ?? []).map((c) => flat.get(path === "/" ? `/${c.name}` : `${path}/${c.name}`)!);
  }

  /**
   * Keep the next request to /api/<endpoint> whose query matches unanswered until `release`. The page may give up on it
   * meanwhile (a newer load aborts the old one); the answer then goes nowhere, as a late answer from `freespace serve` would.
   */
  hold(endpoint: string, match: (q: URLSearchParams) => boolean = () => true): Held {
    let arrive: (r: Request) => void = () => {};
    let open = () => {};
    let done = () => {};
    const arrived = new Promise<Request>((r) => {
      arrive = r;
    });
    const gate = new Promise<void>((r) => {
      open = r;
    });
    const answered = new Promise<void>((r) => {
      done = r;
    });
    this.holds.push({ endpoint, match, arrive, gate, done });
    return {
      arrived,
      release: async () => {
        open();
        await answered;
      },
    };
  }

  private async serve(route: Route): Promise<void> {
    const u = new URL(route.request().url());
    this.requests.push(u.pathname + u.search);
    // as `freespace serve` answers (cli/src/serve/api.rs), so a page that asks another way draws an error in the guard
    if (!["GET", "HEAD"].includes(route.request().method())) {
      return route.fulfill({ status: 405, contentType: "text/plain", body: "only GET" });
    }
    const i = this.holds.findIndex((x) => `/api/${x.endpoint}` === u.pathname && x.match(u.searchParams));
    const held = i < 0 ? undefined : this.holds.splice(i, 1)[0];
    if (!held) return this.handle(route);
    held.arrive(route.request());
    await held.gate;
    try {
      await this.handle(route);
    } finally {
      held.done();
    }
  }

  async handle(route: Route): Promise<void> {
    const req = route.request();
    const u = new URL(req.url());
    if (this.sessionGone || req.headers()["x-freespace-token"] !== TOKEN) {
      return route.fulfill({ status: 403, contentType: "text/plain", body: "forbidden\n" });
    }
    const fail = this.failWith;
    if (fail) return this.error(route, fail.status, fail.code, fail.message, fail.fix ?? null);
    const q = u.searchParams;
    const ep = u.pathname.slice("/api/".length);
    if (ep === "status") return route.fulfill({ json: this.status() });
    if (this.noSnapshot) return this.error(route, 404, "NO_SNAPSHOT", "no snapshot in /var/lib/freespace");
    const snap = q.get("snapshot");
    if (snap && snap !== this.snapshotId) return this.error(route, 409, "ROTATED", "snapshot rotated");
    const limit = Number(q.get("limit") ?? 30);
    const page = <T>(all: T[]) => ({ entries: all.slice(0, limit), total_count: all.length, truncated: all.length > limit, limit });
    const f = this.target(q, route);
    if (!f) return;
    switch (ep) {
      case "ls": {
        const sort = q.get("sort") ?? "allocated";
        const kids = this.children(f.path).sort((a, b) =>
          sort === "mtime" ? (a.node.mtime ?? 0) - (b.node.mtime ?? 0) : b.allocated - a.allocated,
        );
        return route.fulfill({ json: { ...this.header(), entry: this.entry(f), ...page(kids.map((k) => this.entry(k))), sort } });
      }
      case "top": {
        const kind = q.get("kind") ?? "file";
        const min = sizeArg(q.get("min_size"));
        const older = cutoff(q.get("older_than"));
        const all = [...this.flat().values()]
          .filter((x) => x.path.startsWith(f.path === "/" ? "/" : `${f.path}/`) && (x.node.kind ?? "dir") === kind)
          .filter((x) => x.allocated >= min && (older === null || (kind === "dir" ? x.newest : (x.node.mtime ?? 0)) < older))
          .sort((a, b) => b.allocated - a.allocated);
        const total = all.reduce((s, x) => s + x.allocated, 0);
        return route.fulfill({
          json: {
            ...this.header(),
            path: f.path,
            kind,
            min_size: min,
            older_than: older,
            sort: "allocated",
            ...page(all.map((x) => this.entry(x))),
            total: { count: all.length, counted: all.length, allocated: total, apparent: total },
            total_allocated: total,
          },
        });
      }
      case "find": {
        const name = q.get("name") ?? q.get("iname");
        const min = sizeArg(q.get("min_size"));
        const older = cutoff(q.get("older_than"));
        const newer = cutoff(q.get("newer_than"));
        if (Number.isNaN(min)) return this.error(route, 400, "USAGE", `bad size "${q.get("min_size")}"`);
        if (Number.isNaN(older) || Number.isNaN(newer)) return this.error(route, 400, "USAGE", "bad date");
        const re = name
          ? new RegExp(
              `^${name
                .replace(/[.+^${}()|[\]\\]/g, "\\$&")
                .replace(/\*/g, ".*")
                .replace(/\?/g, ".")}$`,
              q.has("iname") ? "i" : "",
            )
          : null;
        const kind = q.get("kind") ?? "any";
        const all = [...this.flat().values()]
          .filter((x) => x.path !== f.path && x.path.startsWith(f.path === "/" ? "/" : `${f.path}/`))
          .filter((x) => kind === "any" || (x.node.kind ?? "dir") === kind)
          .filter((x) => (!re || re.test(x.node.name)) && x.allocated >= min)
          .filter((x) => (older === null || (x.node.mtime ?? 0) < older) && (newer === null || (x.node.mtime ?? 0) >= newer))
          .sort((a, b) => b.allocated - a.allocated);
        const total = all.reduce((s, x) => s + x.allocated, 0);
        return route.fulfill({
          json: {
            ...this.header(),
            path: f.path,
            kind,
            sort: "allocated",
            ...page(all.map((x) => ({ ...this.entry(x), nested: false }))),
            total: { count: all.length, counted: all.length, allocated: total, apparent: total },
            total_allocated: total,
          },
        });
      }
      case "diff": {
        if (!this.prevTree) return this.error(route, 404, "NO_PREV", "no previous snapshot");
        const prevFlat = this.flat(this.prevTree);
        const cur = this.children(f.path);
        const prev = this.children(f.path, this.prevTree);
        const rows = cur.map((c) => {
          const p = prevFlat.get(c.path);
          const delta = c.allocated - (p?.allocated ?? 0);
          return {
            path: c.path,
            name: c.node.name,
            kind: c.node.kind ?? "dir",
            status: !p ? "new" : delta ? "changed" : "same",
            allocated: c.allocated,
            allocated_prev: p?.allocated ?? 0,
            delta,
            items: c.items,
            items_prev: p?.items ?? 0,
            mtime: c.node.mtime ?? 0,
            newest_mtime: c.newest,
          };
        });
        for (const p of prev) {
          if (!cur.some((c) => c.path === p.path)) {
            rows.push({
              path: p.path,
              name: p.node.name,
              kind: p.node.kind ?? "dir",
              status: "gone",
              allocated: 0,
              allocated_prev: p.allocated,
              delta: -p.allocated,
              items: 0,
              items_prev: p.items,
              mtime: 0,
              newest_mtime: 0,
            });
          }
        }
        rows.sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta) || b.delta - a.delta);
        const pf = prevFlat.get(f.path);
        return route.fulfill({
          json: {
            ...this.header(),
            path: f.path,
            prev_scanned_at: NOW - 31 * 3600,
            depth: 1,
            kind: "any",
            partial: false,
            refreshed: [],
            roots_changed: false,
            delta: f.allocated - (pf?.allocated ?? 0),
            ...page(rows),
          },
        });
      }
    }
    return this.error(route, 404, "USAGE", `unknown endpoint ${u.pathname}`);
  }

  async install(page: Page): Promise<this> {
    await page.route("**/api/**", (r) => this.serve(r));
    return this;
  }

  count(endpoint: string): number {
    return this.requests.filter((r) => r.startsWith(`/api/${endpoint}`)).length;
  }
}

export { dir, file, G, M, DAY };

export const url = (hash = "/home/tass") => `/#${hash}${hash.includes("?") ? "&" : "?"}t=${TOKEN}`;
