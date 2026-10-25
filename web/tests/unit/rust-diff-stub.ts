// The stub freespace that stands in for both builds in the tests of scripts/rust-diff.sh (rust-diff*.test.ts), and what
// those tests share: one run of the job, a base that holds its first call, a poll with a deadline.
import { chmodSync, existsSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const repo = realpathSync(join(import.meta.dir, "../../.."));
export const script = join(repo, "scripts/rust-diff.sh");

// Every call prints new times, ages, durations and live free space; a scan or a refresh writes a new snapshot id, and
// fs_used and fs_avail of the root follow the id, as a stored scan's would. The data is the arguments and ENTRIES. A
// question about nope is refused as the CLI refuses: the message on stderr in plain text, also under --json, the error
// body on stdout under --json, and a non-zero exit status. The other questions follow the real CLI: --version and
// --help name the VERSION (and the file it runs as, which the script fixes to "freespace"); a refresh counts itself in
// the snapshot dir it runs on, and status its visits there and in $HOME; completions, schema and skill print static
// text; the questions the harness expects refused (EXPECTED in scripts/rust-diff/questions.sh) are refused with the
// CLI's status and words, the config phase's included; with no snapshot a question is refused with exit 8 (doctor fails
// with 1), and while the lock is held (a scan running) status still answers and scan and refresh are refused with exit
// 6; `serve` and `gui` are a small server of the same rules (SERVE_JS). A scan and a refresh say on stderr what they
// see in the folder they walk, and add a row to snapshot.db.
// SH is shell code run before the answer; it may set NEED, the space the next scan needs (a figure, not live), FAILS,
// the count a scan or a refresh writes to state.json beside its live time, and BOUND, what --json echoes before the
// entries. EARLY runs before anything else; it may set SCHEMA_EXTRA, SQL a scan runs on snapshot.db. A text answer
// names the time zone, whether the lock is held, what it got of the caller's environment, the folder it runs in when
// that is in a tree, and says when it goes to a terminal. A bound comes back as the CLI echoes it: a date as given, an
// age (`1y`) as now less the age in seconds (live), in text as the date it stands for.
export type Refusal = { exit: number; message: string };
export type StubOptions = { entries?: number; refusal?: Refusal; version?: string; sh?: string; early?: string };
export const REFUSAL: Refusal = { exit: 2, message: "is not in the snapshot" };

// Refusals by the words asked, before any state is read.
const refusals = (refusal: Refusal, version: string) => `case "$*" in
  *nope*)
    printf 'freespace: ${refusal.message}: %s\\nfix: freespace scan\\n' "$*" >&2
    case " $* " in *" --json "*) printf '{"error":{"code":"NOT_FOUND","exit":${refusal.exit}}}\\n' ;; esac
    exit ${refusal.exit} ;;
esac
case " $* " in
  *" --version "*) printf 'freespace ${version}\\n'; exit 0 ;;
  *" --help "*) printf 'freespace ${version} help\\nUsage: %s %s\\n' "\${0##*/}" "$*"; exit 0 ;;
  *" --no-such-flag "*)
    printf 'error: unexpected argument\\nUsage: %s\\n' "\${0##*/}" >&2
    case " $* " in *" --json "*) printf '{"error":{"code":"USAGE","exit":2,"context":{"version":"${version}"}}}\\n' ;; esac
    exit 2 ;;
  *" --snapshot not-this-one "*) echo "freespace: snapshot not-this-one was replaced" >&2; exit 4 ;;
  *" --older-than 3x "*) echo "freespace: bad date" >&2; exit 2 ;;
  *"/top.bin "*) echo "freespace: top.bin is not a directory" >&2; exit 2 ;;
  *" --name x\\ "*) echo "freespace: bad --name pattern: pattern ends with a lone backslash" >&2; exit 2 ;;
  *" --path-raw zz "*) echo "freespace: bad --path-raw: expected hex bytes" >&2; exit 2 ;;
  *" --path-raw 2f6e6f7065 "*) echo "freespace: /nope is outside the scanned roots" >&2; exit 2 ;;
esac
case "$1 $2" in
  "refresh "*/empty) echo "freespace: $2 is no longer a folder on disk" >&2; exit 2 ;;
  "refresh "*/app/src) echo "freespace: $2 resolves to another path on disk (symlink?)" >&2; exit 2 ;;
  "refresh "*/media-link) echo "freespace: $2 is not a directory" >&2; exit 2 ;;
  "refresh "*/media) [ -d "$2" ] || { echo "freespace: root $2 no longer exists; run a full scan" >&2; exit 2; } ;;
esac
`;

// The config file's rules, then each command's own: what it prints, or why it refuses given the lock and the snapshot.
const commands = (version: string, entries: number) => `conf=\${FREESPACE_CONFIG:-/none}
if grep -q keep_old "$conf" 2>/dev/null; then echo "freespace: unknown field keep_old" >&2; exit 2; fi
if [ -f "$conf" ]; then
  [ "$1" != diff ] || { echo "freespace: no previous generation" >&2; exit 3; }
  case "$2" in */projects* | */media* | -*) ;; /*) echo "freespace: outside the scanned roots" >&2; exit 2 ;; esac
  if [ "$*" = scan ]; then
    for r in $(sed -n 's/^roots = \\[\\(.*\\)\\]$/\\1/p' "$conf" | tr -d '",'); do
      [ -d "$r" ] || { echo "freespace: scan failed: No such file or directory" >&2; exit 2; }
    done
  fi
fi
case "$1" in
  completions) printf '# completions for %s\\n' "$2"; exit 0 ;;
  schema) printf '{"commands":["ls","top"]}\\n'; exit 0 ;;
  skill) printf '<!-- freespace-version: ${version} -->\\n'; exit 0 ;;
  install-timer) echo "freespace: install-timer needs root" >&2; exit 1 ;;
  serve) ENTRIES=${entries} exec "\${BUN:?}" -e "$SERVE_JS" ;;
  gui)
    [ -t 0 ] || [ -t 1 ] || { echo "freespace: gui needs a terminal and a browser" >&2; exit 2; }
    frag=
    for a in "$@"; do
      case $a in gui | --port | 0) continue ;; esac
      frag=$a
      printf '%s' "$a" | iconv -f UTF-8 -t UTF-8 >/dev/null 2>&1 || frag="/?raw=$(printf '%s' "$a" | od -An -v -tx1 | tr -d ' \\n')"
    done
    GUI_FRAG=$frag ENTRIES=${entries} exec "\${BUN:?}" -e "$SERVE_JS" ;;
  scan | refresh)
    if $held; then echo "freespace: another scan is running" >&2; exit 6; fi
    [ "$1" = scan ] || [ -f "$dir/id" ] || { printf 'freespace: no snapshot in %s\\n' "$dir" >&2; exit 8; } ;;
  status | ls | top | find | diff | doctor | --json)
    [ -f "$dir/id" ] || case "$held $1" in "true status" | "true --json") ;; *)
      [ "$1" != doctor ] || { echo "FAIL access: no snapshot"; exit 1; }
      w=; if $held; then w=" yet: a scan running since 00:00, wait"; fi
      printf 'freespace: no snapshot in %s%s\\n' "$dir" "$w" >&2
      case " $* " in *" --json "*) printf '{"error":{"code":"NO_SNAPSHOT","exit":8}}\\n' ;; esac
      exit 8 ;;
    esac ;;
esac
`;

// What a call leaves: a scan and a refresh a new id, a row in snapshot.db and what they saw; a refresh its count; status
// its visits. Then the bound the question gives, as the CLI echoes it.
const BOOKKEEPING = `if [ "$1" = scan ] || [ "$1" = refresh ]; then
  mkdir -p "$dir"; echo "$t" > "$dir/id"; echo "sees $(ls "$2" 2>/dev/null | tr '\\n' ' ')" >&2
  sqlite3 "$dir/snapshot.db" "CREATE TABLE IF NOT EXISTS calls(id INTEGER PRIMARY KEY, command TEXT); \${SCHEMA_EXTRA-} INSERT INTO calls(command) VALUES ('$1');"
fi
if [ "$1" = refresh ]; then echo "$2" >> "$dir/refreshed"; fi
id=0; [ ! -f "$dir/id" ] || read -r id < "$dir/id"
r=0; if [ -f "$dir/refreshed" ]; then while read -r _; do r=$((r + 1)); done < "$dir/refreshed"; fi
if [ "$1" = status ]; then echo x >> "$HOME/.visits"; [ ! -d "$dir" ] || echo x >> "$dir/visits"; fi
v=$(cat "$HOME/.visits" "$dir/visits" 2>/dev/null | wc -l)
need="512 M"
fails=0
bound=; since=; k=
for a in "$@"; do
  case "$k" in
    older_than | newer_than)
      case "$a" in
        [0-9]*d | [0-9]*w | [0-9]*mo | [0-9]*y)
          c=\${a%%[a-z]*}; case \${a#"$c"} in d) m=1 ;; w) m=7 ;; mo) m=30 ;; *) m=365 ;; esac
          a=$((s - c * m * 86400)); since=$(date -u -d "@$a" +%F) ;;
        *) since=$a; a="\\"$a\\"" ;;
      esac
      bound=",\\"$k\\":$a" ;;
  esac
  case "$a" in --older-than) k=older_than ;; --newer-than) k=newer_than ;; *) k= ;; esac
done
`;

// The answer, after SH: state.json for a scan and a refresh, then --json or text.
const reply = (
  version: string,
  entries: number,
) => `if [ "$1" = scan ] || [ "$1" = refresh ]; then printf '{"last_success_ts":%s,"consecutive_failures":%s}\\n' "$t" "$fails" > "$dir/state.json"; fi
case " $* " in
  *" --json "*)
    printf '{"snapshot_id":"%s","scanned_at":%s,"prev_scanned_at":%s,"age_seconds":%s,"last_success_ts":%s,' "$id" "$s" "$s" "$n" "$s"
    printf '"walk_seconds":0.%s,"fs":[{"path":"/","fs_type":"ext4","total":%s,"used":%s,"avail":%s}],' "$n" "$n" "$n" "$n"
    printf '"roots":[{"path":"/","allocated":1024,"fs_used":%s,"fs_avail":%s}],' "\${id#?????????????}" "\${id#??????????????}"
    printf '"version":"${version}","refreshed":%s,"detail":"generation 2 · %s min old"%s,"entries":${entries}}\\n' "$r" "$n" "$bound" ;;
  *)
    printf 'args %s\\n%s G free · snapshot %s min old\\n' "$*" "$n.5" "$n"
    printf 'scanned 2026-10-24 %02d:%02d · filesystem %s G used of %s G, %s M free now\\n' "$n" "$n" "$n" "$n" "$n"
    printf 'next scan needs %s free, %s G available; scanned in %s.%s s\\n' "$need" "$n" "$n" "$n"
    printf 'host · linux · freespace ${version}; skill 9.9.9, binary ${version}\\n'
    printf 'snapshot %s was replaced\\nentries ${entries}\\nrefreshed %s\\ntz %s\\nheld %s\\nvisits %s\\n' "$id" "$r" "$TZ" "$held" "$v"
    printf 'env %s %s home %s lc %s\\n' "\${XDG_CONFIG_HOME-unset}" "\${XDG_RUNTIME_DIR-unset}" "$HOME" "$LC_ALL"
    case $PWD in */tree | */tree/*) printf 'cwd %s\\n' "$PWD" ;; esac
    [ -z "$since" ] || printf 'untouched since %s\\n' "$since"
    [ ! -t 1 ] || echo "on a terminal" ;;
esac
`;

export const stub = ({ entries = 3, refusal = REFUSAL, version = "0.1.0", sh = "", early = "" }: StubOptions = {}) =>
  [
    "#!/bin/sh",
    `t=$(date +%s%N); n=$((t / 1000 % 50 + 1)); s=\${t%?????????}`,
    `dir=\${FREESPACE_SNAPSHOT_DIR:?}`,
    early,
    'held=false; [ ! -e "$dir/.lock" ] || flock -n -s "$dir/.lock" true || held=true',
    refusals(refusal, version),
    commands(version, entries),
    BOOKKEEPING,
    sh,
    reply(version, entries),
  ].join("\n");

// The stub's `serve` and `gui`: what `freespace serve --port 0` does, in small. It answers only GET and HEAD, checks the
// Host (127.0.0.1 or localhost with its port), then for /api/* the Origin (its own, either name), Sec-Fetch-Site and the
// token, refuses what the real server refuses with its status (a gone snapshot 409, a path not in it 404, a bad limit
// 400, an unknown endpoint 404), and stops with exit 0 on TERM. serve prints its address with a 64-hex token on stderr.
// gui (GUI_FRAG set, the folder's fragment) over ssh prints the tunnel lines; here it writes a redirect file holding the
// address into XDG_RUNTIME_DIR (mode GUI_MODE, 600), hands it to xdg-open, says so when that fails, and removes the file
// on the first request with the token, and stops with GUI_TERM_EXIT (0) on TERM. The address line of serve, and of gui
// here, comes in two writes. FRAME is its x-frame-options; ALLOW_POST lets a POST through.
export const SERVE_JS = `
const fs = require("node:fs");
const token = "ab".repeat(32);
const frame = process.env.FRAME ?? "DENY";
const frag = process.env.GUI_FRAG;
// plain lines: on a terminal, console.error would color them
const say = (s) => process.stderr.write(s + "\\n");
// a line in two writes, cut AT characters in, the rest 300 ms later: the harness must wait for the whole address, as a
// terminal (script) or a file may hold half a line
const sayCut = (s, at) =>
  new Promise((done) => {
    process.stderr.write(s.slice(0, at));
    setTimeout(() => done(say(s.slice(at))), 300);
  });
let bootstrap = "";
const reply = (status, body, type = "application/json") =>
  new Response(typeof body === "string" ? body : JSON.stringify(body), {
    status,
    headers: { "content-type": type, "cache-control": "no-store", "x-frame-options": frame },
  });
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch(req) {
    const u = new URL(req.url);
    const p = u.searchParams;
    const port = server.port;
    const names = (scheme) => [scheme + "127.0.0.1:" + port, scheme + "localhost:" + port];
    if (req.method !== "GET" && req.method !== "HEAD" && !process.env.ALLOW_POST) return reply(405, "only GET", "text/plain");
    if (!names("").includes(req.headers.get("host")))
      return reply(403, "answers on 127.0.0.1:" + port + " only; ssh -N -L " + port + ":127.0.0.1:" + port, "text/plain");
    if (!u.pathname.startsWith("/api/")) return reply(200, "<!doctype html>" + Date.now(), "text/html");
    const origin = req.headers.get("origin");
    const site = req.headers.get("sec-fetch-site");
    if ((origin && !names("http://").includes(origin)) || site === "cross-site" || site === "same-site") return reply(403, "forbidden", "text/plain");
    if (req.headers.get("x-freespace-token") !== token) return reply(403, "forbidden", "text/plain");
    if (bootstrap) fs.rmSync(bootstrap, { force: true });
    bootstrap = "";
    if (u.pathname === "/api/status") return reply(200, { scanned_at: Date.now(), entries: Number(process.env.ENTRIES) });
    if (!["/api/ls", "/api/top", "/api/find", "/api/diff"].includes(u.pathname)) return reply(404, { error: { code: "USAGE" } });
    if (p.get("snapshot") === "not-this-one") return reply(409, { error: { code: "ROTATED" } });
    if ((p.get("path") ?? "").endsWith("/nope")) return reply(404, { error: { code: "PATH_NOT_FOUND" } });
    if (p.has("limit") && !/^[0-9]+$/.test(p.get("limit"))) return reply(400, { error: { code: "USAGE" } });
    return reply(200, { endpoint: u.pathname, query: Object.fromEntries(p), entries: Number(process.env.ENTRIES) });
  },
});
const port = server.port;
const url = "http://127.0.0.1:" + port + "/#" + (frag ? frag + (frag.includes("?") ? "&" : "?") : "") + "t=" + token;
if (frag === undefined) {
  const line = "freespace serve: " + url;
  sayCut(line + "\\npress Ctrl-C to stop", line.indexOf("t=") + 12); // in the token
} else if (process.env.SSH_CONNECTION && !process.env.DISPLAY) {
  const host = require("node:os").hostname();
  say("freespace gui is running on " + host + ", port " + port + ". Open:\\n  " + url);
  say("first run on your computer:\\n  ssh -N -L " + port + ":127.0.0.1:" + port + " " + host + "\\npress Ctrl-C to stop");
} else {
  bootstrap = process.env.XDG_RUNTIME_DIR + "/freespace-gui-" + process.pid + ".html";
  // the address twice, as the real one writes it: in the refresh and in a link
  const html = '<meta http-equiv="refresh" content="0;url=' + url + '"><a href="' + url + '">Open freespace</a>\\n';
  fs.writeFileSync(bootstrap, html, { mode: Number.parseInt(process.env.GUI_MODE ?? "600", 8) });
  const line = "freespace gui on http://127.0.0.1:" + port + "/ · press Ctrl-C to stop";
  const said = sayCut(line, line.indexOf(":" + port) + 2); // after the port's first digit
  Bun.spawn(["xdg-open", bootstrap], { stdin: "ignore", stdout: "ignore", stderr: "ignore" }).exited.then(async (code) => {
    await said;
    if (code !== 0) say("no browser opened; open " + url);
  });
}
// gui exits GUI_TERM_EXIT on TERM (0 unless given), so an exit status that is not gui's own shows
process.on("SIGTERM", () => process.exit(frag === undefined ? 0 : Number(process.env.GUI_TERM_EXIT ?? 0)));
`;

// A base whose scan number AT fails: it says why on stderr and exits 7. The scans before it work, as far as the script
// can tell (it never reads the snapshot itself).
export const scanFails = (at: number) => `#!/bin/sh
d=\${FREESPACE_SNAPSHOT_DIR:?}; mkdir -p "$d"
n=0; [ ! -f "$d/n" ] || read -r n < "$d/n"; n=$((n + 1)); echo "$n" > "$d/n"
[ "$n" -lt ${at} ] || { echo "disk on fire in scan ${at}" >&2; exit 7; }
`;

// A base that holds its first call, the scan of the fixture tree: it says so in $STUB_HELD, then waits for $STUB_GO
// (10 s at most, so a failed test cannot leave it running) and marks its end in $STUB_HELD.end. The script is then past
// making its work dir and its tree.
export const HOLD = `#!/bin/sh
: > "$STUB_HELD"
n=0
while [ ! -e "$STUB_GO" ] && [ $n -lt 500 ]; do sleep 0.02; n=$((n + 1)); done
: > "$STUB_HELD.end"
`;

// A poll with a deadline (10 s unless given) for something another process does; nothing to wait on but the thing itself.
export async function until(done: () => boolean, ms = 10_000): Promise<boolean> {
  // gstack-timing-ok: poll with a deadline for a marker a child process leaves, the outcome is asserted after it
  for (let i = 0; i < ms / 20 && !done(); i++) await Bun.sleep(20);
  return done();
}

export const writeStub = (dir: string, name: string, body: string) => {
  writeFileSync(join(dir, name), body);
  chmodSync(join(dir, name), 0o755);
};

// Every run: the stub's server needs bun and its code; one folder per level keeps the wide part of the trees small; the
// caller's proxy and XDG folders, which the script keeps from curl and from the CLI.
export const NO_SUCH_PROXY = "http://127.0.0.1:9";
export const STUB_ENV = {
  BUN: process.execPath,
  SERVE_JS,
  RUST_DIFF_FAN: "1",
  ...{ http_proxy: NO_SUCH_PROXY, HTTP_PROXY: NO_SUCH_PROXY, ALL_PROXY: NO_SUCH_PROXY, no_proxy: "", NO_PROXY: "" },
  ...{ XDG_CONFIG_HOME: "/nonexistent/config", XDG_RUNTIME_DIR: "/nonexistent/runtime" },
};

export type Run = { out: string; code: number; stdout: string; stderr: string };
let runs = 0;
// One run of the job in TMP, base and head named by their stub files there; the out dir is new for each run. With
// `tty`, the job runs under script(1), its stdin a terminal as from a shell, its stdout and stderr in files beside the
// out dir.
export async function guard(
  tmp: string,
  base: string,
  head: string,
  env: Record<string, string> = {},
  how: { tty?: boolean } = {},
): Promise<Run> {
  const out = join(tmp, `out-${runs++}`);
  const argv = [script, join(tmp, base), join(tmp, head), out];
  if (how.tty) {
    const q = (s: string) => `'${s.replaceAll("'", "'\\''")}'`;
    const cmd = `exec ${argv.map(q).join(" ")} >${q(`${out}.stdout`)} 2>${q(`${out}.stderr`)}`;
    const p = Bun.spawn(["script", "-qec", cmd, "/dev/null"], {
      stdin: "ignore",
      stdout: "ignore",
      stderr: "pipe",
      env: { ...process.env, ...STUB_ENV, ...env, SHELL: Bun.which("bash") ?? "/bin/bash" },
    });
    const [said, code] = await Promise.all([new Response(p.stderr).text(), p.exited]);
    const file = (f: string) => (existsSync(f) ? readFileSync(f, "utf8") : `(no ${f}; script said: ${said})`);
    return { out, code, stdout: file(`${out}.stdout`), stderr: file(`${out}.stderr`) };
  }
  const p = Bun.spawn(argv, {
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, ...STUB_ENV, ...env },
  });
  const [stdout, stderr, code] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited]);
  return { out, code, stdout, stderr };
}
