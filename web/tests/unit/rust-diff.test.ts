// scripts/rust-diff.sh, the rust-diff job of refactor-guard: answers that differ only in time fields, live numbers,
// the snapshot id and the build's own version match after normalization (D20); one changed data byte in head fails
// with the diff; a base that answers two runs differently stops the job as unreliable before head runs; a base that
// cannot make the fixture snapshot, or no longer answers as the harness expects, stops it at once; bad input, a missing
// tool and root exit 2. A question the CLI refuses is part of the answer too: its exit status and its stderr are
// compared like its stdout, and so are the files each phase leaves (names, types, modes); a call that hangs answers
// exit 124; a build that changes the tree it reads stops the job. A HUP, INT or TERM ends the job with 128 + the
// signal number and removes its work dir. A stub freespace stands in for both builds.
import { afterAll, beforeAll, expect, test } from "bun:test";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const repo = realpathSync(join(import.meta.dir, "../../.."));
const script = join(repo, "scripts/rust-diff.sh");

// Every call prints new times, ages, durations and live free space; a scan or a refresh writes a new snapshot id, and
// fs_used and fs_avail of the root follow the id, as a stored scan's would. The data is the arguments and ENTRIES. A
// question about /nope is refused as the CLI refuses: the message on stderr in plain text, also under --json, the error
// body on stdout under --json, and a non-zero exit status. The other questions follow the real CLI: --version and
// --help name the VERSION (and the file it runs as, which the script fixes to "freespace"); a refresh counts itself in
// the snapshot dir it runs on, and status its visits there and in $HOME; completions, schema and skill print static
// text; the questions the harness expects refused (EXPECTED in scripts/rust-diff/questions.sh) are refused with the
// CLI's status and words, the config phase's included; with no snapshot a question is refused with exit 8 (doctor fails
// with 1), and while the lock is held (a scan running) status still answers and scan and refresh are refused with exit
// 6; `serve` is a small server of the same rules (SERVE_JS). A scan and a refresh say on stderr what they see in the
// folder they walk.
// SH is shell code run before the answer; it may set NEED, the space the next scan needs (a figure, not live), FAILS,
// the count a scan or a refresh writes to state.json beside its live time, and BOUND, what --json echoes before the
// entries. EARLY runs before anything else. A text answer names the time zone, whether the lock is held, what it got of
// the caller's environment, and says when it goes to a terminal. A bound comes back as the CLI echoes it: a date as
// given, an age (`1y`) as now less the age in seconds (live), in text as the date it stands for.
type Refusal = { exit: number; message: string };
type StubOptions = { entries?: number; refusal?: Refusal; version?: string; sh?: string; early?: string };
const REFUSAL: Refusal = { exit: 2, message: "is not in the snapshot" };
const stub = ({ entries = 3, refusal = REFUSAL, version = "0.1.0", sh = "", early = "" }: StubOptions = {}) => `#!/bin/sh
t=$(date +%s%N); n=$((t / 1000 % 50 + 1)); s=\${t%?????????}
dir=\${FREESPACE_SNAPSHOT_DIR:?}
${early}
held=false; [ ! -e "$dir/.lock" ] || flock -n -s "$dir/.lock" true || held=true
case "$*" in
  */nope*)
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
esac
conf=\${FREESPACE_CONFIG:-/none}
if grep -q keep_old "$conf" 2>/dev/null; then echo "freespace: unknown field keep_old" >&2; exit 2; fi
if [ -f "$conf" ]; then
  [ "$1" != diff ] || { echo "freespace: no previous generation" >&2; exit 3; }
  case "$2" in */projects* | */media* | -*) ;; /*) echo "freespace: outside the scanned roots" >&2; exit 2 ;; esac
fi
case "$1" in
  completions) printf '# completions for %s\\n' "$2"; exit 0 ;;
  schema) printf '{"commands":["ls","top"]}\\n'; exit 0 ;;
  skill) printf '<!-- freespace-version: ${version} -->\\n'; exit 0 ;;
  install-timer) echo "freespace: install-timer needs root" >&2; exit 1 ;;
  serve) ENTRIES=${entries} exec "\${BUN:?}" -e "$SERVE_JS" ;;
  scan | refresh) if $held; then echo "freespace: another scan is running" >&2; exit 6; fi ;;
  status | ls | top | find | diff | doctor | --json)
    [ -f "$dir/id" ] || case "$held $1" in "true status" | "true --json") ;; *)
      [ "$1" != doctor ] || { echo "FAIL access: no snapshot"; exit 1; }
      w=; if $held; then w=" yet: a scan running since 00:00, wait"; fi
      printf 'freespace: no snapshot in %s%s\\n' "$dir" "$w" >&2
      case " $* " in *" --json "*) printf '{"error":{"code":"NO_SNAPSHOT","exit":8}}\\n' ;; esac
      exit 8 ;;
    esac ;;
esac
if [ "$1" = scan ] || [ "$1" = refresh ]; then
  mkdir -p "$dir"; echo "$t" > "$dir/id"; echo "sees $(ls "$2" 2>/dev/null | tr '\\n' ' ')" >&2
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
${sh}
if [ "$1" = scan ] || [ "$1" = refresh ]; then printf '{"last_success_ts":%s,"consecutive_failures":%s}\\n' "$t" "$fails" > "$dir/state.json"; fi
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
    [ -z "$since" ] || printf 'untouched since %s\\n' "$since"
    [ ! -t 1 ] || echo "on a terminal" ;;
esac
`;

// The stub's `serve`: what `freespace serve --port 0` does, in small. It prints its address with a 64-hex token on
// stderr, answers only GET, checks the Host, then for /api/* the Origin and the token, refuses what the real server
// refuses with its status (a gone snapshot 409, a path not in it 404, a bad limit 400, an unknown endpoint 404), and stops
// with exit 0 on TERM. FRAME is its x-frame-options; a head may set another; ALLOW_POST lets a POST through.
const SERVE_JS = `
const token = "ab".repeat(32);
const frame = process.env.FRAME ?? "DENY";
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
    if (req.method !== "GET" && !process.env.ALLOW_POST) return reply(405, "only GET", "text/plain");
    if (req.headers.get("host") !== "127.0.0.1:" + server.port)
      return reply(403, "answers on 127.0.0.1:" + server.port + " only; ssh -N -L " + server.port + ":127.0.0.1:" + server.port, "text/plain");
    if (!u.pathname.startsWith("/api/")) return reply(200, "<!doctype html>" + Date.now(), "text/html");
    if (req.headers.get("origin") || req.headers.get("x-freespace-token") !== token) return reply(403, "forbidden", "text/plain");
    if (u.pathname === "/api/status") return reply(200, { scanned_at: Date.now(), entries: Number(process.env.ENTRIES) });
    if (!["/api/ls", "/api/top", "/api/find", "/api/diff"].includes(u.pathname)) return reply(404, { error: { code: "USAGE" } });
    if (p.get("snapshot") === "not-this-one") return reply(409, { error: { code: "ROTATED" } });
    if ((p.get("path") ?? "").endsWith("/nope")) return reply(404, { error: { code: "PATH_NOT_FOUND" } });
    if (p.has("limit") && !/^[0-9]+$/.test(p.get("limit"))) return reply(400, { error: { code: "USAGE" } });
    return reply(200, { endpoint: u.pathname, query: Object.fromEntries(p), entries: Number(process.env.ENTRIES) });
  },
});
console.error("freespace serve: http://127.0.0.1:" + server.port + "/#t=" + token + "\\npress Ctrl-C to stop");
process.on("SIGTERM", () => process.exit(0));
`;

// A base whose scan number AT fails: it says why on stderr and exits 7. The scans before it work, as far as the script
// can tell (it never reads the snapshot itself).
const scanFails = (at: number) => `#!/bin/sh
d=\${FREESPACE_SNAPSHOT_DIR:?}; mkdir -p "$d"
n=0; [ ! -f "$d/n" ] || read -r n < "$d/n"; n=$((n + 1)); echo "$n" > "$d/n"
[ "$n" -lt ${at} ] || { echo "disk on fire in scan ${at}" >&2; exit 7; }
`;

// One pass of the questions is about 255 stub calls, each normalized, and a full run makes three (base, base again,
// head). The ten runs start together in beforeAll, four of them full, the others stopped after one or two passes: about
// 45 s for the whole file on a GitHub runner, 130 s on a busy 8-core machine. afterAll waits for them all, so it gets
// the same bound: a filtered run (-t) must not leave tmp behind.
const RUN_MS = 240_000;
type Run = { out: string; code: number; stdout: string; stderr: string };
let tmp = "";
let runs = 0;
// The runs beforeAll starts.
type RunName = "same" | "byte" | "flaky" | "other" | "unlike" | "stale" | "noserve" | "tamper" | "retouch" | "shrink";
const pending = {} as Record<RunName, Promise<Run>>;

// Every run: the stub's server needs bun and its code; one folder per level keeps the wide part of the trees small; the
// caller's proxy and XDG folders, which the script keeps from curl and from the CLI.
const NO_SUCH_PROXY = "http://127.0.0.1:9";
const STUB_ENV = {
  BUN: process.execPath,
  SERVE_JS,
  RUST_DIFF_FAN: "1",
  ...{ http_proxy: NO_SUCH_PROXY, HTTP_PROXY: NO_SUCH_PROXY, ALL_PROXY: NO_SUCH_PROXY, no_proxy: "", NO_PROXY: "" },
  ...{ XDG_CONFIG_HOME: "/nonexistent/config", XDG_RUNTIME_DIR: "/nonexistent/runtime" },
};

async function guard(base: string, head: string, env: Record<string, string> = {}): Promise<Run> {
  const out = join(tmp, `out-${runs++}`);
  const p = Bun.spawn([script, join(tmp, base), join(tmp, head), out], {
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, ...STUB_ENV, ...env },
  });
  const [stdout, stderr, code] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited]);
  return { out, code, stdout, stderr };
}

beforeAll(() => {
  tmp = realpathSync(mkdtempSync(join(tmpdir(), "rust-diff-")));
  const write = (name: string, body: string) => {
    writeFileSync(join(tmp, name), body);
    chmodSync(join(tmp, name), 0o755);
  };
  write("base", stub());
  write("same", stub({ version: "0.2.0" })); // a release bump alone is no difference
  write("byte", stub({ entries: 4 }));
  // its own pid in every --json answer
  write("flaky", stub({ sh: 'bound="$bound,\\"pid\\":$$"' }));
  // a head that refuses /nope with another status and other words, and whose server lets other pages frame it: three
  // differences in other lines of the answers, so one run shows them all
  write("other", stub({ refusal: { exit: 5, message: "nothing at" }, early: "FRAME=SAMEORIGIN; export FRAME" }));
  // differences no mask may hide, in one head so the suite pays for one more run, not six: the space doctor says the
  // next scan needs, a count in state.json, a wider mode and a stray file in the snapshot folder, a file in $HOME and
  // one in the unit dir, and the first `diff @` (text, of the fixture snapshot) hanging
  const need = '[ "$1" != doctor ] || need="1.0 G"';
  const leak = '[ ! -f "$dir/id" ] || { chmod 666 "$dir/id"; : > "$dir/leftover.tmp"; }';
  const home = '[ "$1" != status ] || { : > "$HOME/.leak"; : > "$FREESPACE_UNIT_DIR/x.service"; }';
  const hang = '[ "$1 $#" != "diff 2" ] || case "$dir" in */snap) exec sleep 30 ;; esac';
  write("unlike", stub({ sh: [need, "fails=1", leak, home, hang].join("\n") }));
  // a base whose CLI has moved on from the harness: a flag it no longer takes, a refusal it no longer makes, one it
  // makes for another reason, a --json answer that is not JSON, and a server that takes a POST
  const moved = [
    `case " $* " in *" --min-size "*) echo "error: unexpected argument '--min-size'" >&2; exit 2 ;; *" install-timer --no-start"*) exit 0 ;; esac`,
    `case "$*" in "ls "*/nope*) echo "error: unexpected argument" >&2; exit 2 ;; "doctor --json") exit 0 ;; esac`,
    "ALLOW_POST=1; export ALLOW_POST",
  ];
  write("stale", stub({ early: moved.join("\n") }));
  write("noserve", stub({ early: '[ "$1" != serve ] || exit 1' }));
  // builds that change the tree they read, each in another phase: a head links a file of the fixture tree; a base
  // dates back and narrows a file of its own tree after its second scan; a base cuts a file in the config phase
  write("tamper", stub({ early: '[ "$1" != ls ] || [ ! -f "$2/top.bin" ] || ln -f "$2/top.bin" "$2/.top.link"' }));
  write(
    "retouch",
    stub({ early: 'case "$1 $2" in "ls "*/ftree) touch -d "2020-01-01 00:00 UTC" "$2/top.bin"; chmod 600 "$2/top.bin" ;; esac' }),
  );
  write(
    "shrink",
    stub({ early: '[ ! -f "$FREESPACE_CONFIG" ] || case "$1 $2" in "ls "*/projects) truncate -s 1 "$2/app/src/main.rs" ;; esac' }),
  );
  write("noscan1", scanFails(1));
  write("noscan2", scanFails(2));
  writeFileSync(join(tmp, "not-executable"), stub());
  pending.same = guard("base", "same");
  pending.byte = guard("base", "byte");
  pending.flaky = guard("flaky", "same");
  pending.other = guard("base", "other");
  pending.unlike = guard("base", "unlike", { RUST_DIFF_TIMEOUT: "5" });
  pending.stale = guard("stale", "base");
  pending.noserve = guard("noserve", "base");
  pending.tamper = guard("base", "tamper");
  pending.retouch = guard("retouch", "base");
  pending.shrink = guard("shrink", "base");
});

afterAll(async () => {
  await Promise.allSettled(Object.values(pending));
  rmSync(tmp, { recursive: true, force: true });
}, RUN_MS);

// Value: protects=a question the CLI refuses is answered as it is: exit status and plain-text stderr recorded, in the
//   --json form too, and the run goes on; fails_when=`|| code=$?` or the `|| cat` fallback of normalize_json is dropped
//   (the job aborts at ls @/nope) or the status is not written; why_new=the stub never refused anything, so no test
//   ran those paths; seam=none; no_claim=the real CLI's wording and codes (a stub refuses /nope as the CLI does)
// Value: protects=a live value keeps its key and becomes its type, so a key that comes or goes still shows; an age
//   bound's echoed time becomes the age in days and its date in text is masked, while a date bound stays as given;
//   state.json's times are masked, its other fields compared; the CLI gets no XDG folder of the caller's, a $HOME of
//   the work dir and LC_ALL=C; fails_when=a live key is dropped again, the day count goes (base against itself differs:
//   exit 3), covers date bounds too, or ask() misses the `d` unit, run_prefix lets an XDG folder through, or a run
//   no longer starts from a copy of the fixture snapshot and an empty $HOME (the visits the stub counts there differ:
//   exit 3);
//   why_new=no stub echoed a bound or wrote state.json, and live keys were deleted (adversarial review); seam=none;
//   no_claim=which fields the real CLI echoes (it echoes both bounds, as numbers) and what its state.json holds
test(
  "answers that differ only in times, live numbers and the snapshot id match (D20)",
  async () => {
    const r = await pending.same;
    expect(r.stderr).toBe("");
    expect(r.code).toBe(0);
    expect(r.stdout).toContain("exactly like base");
    for (const run of ["base", "base2", "head"]) {
      for (const phase of ["fixture", "start", "fresh", "config"]) expect(existsSync(join(r.out, run, `${phase}.txt`))).toBe(true);
    }
    const text = readFileSync(join(r.out, "head/fresh.txt"), "utf8");
    for (const masked of [
      "args scan <work>/ftree --workers 8\n<size> free · snapshot <age> old\n",
      "scanned <time> · filesystem <size> used of <size>, <size> free now\n",
      "needs 512 M free, <size> available; scanned in <seconds>\n",
      "snapshot <snapshot_id> was replaced\n",
    ]) {
      expect(text).toContain(masked);
    }
    const live = [
      "snapshot_id",
      "scanned_at",
      "prev_scanned_at",
      "age_seconds",
      "last_success_ts",
      "walk_seconds",
      "total",
      "used",
      "avail",
    ];
    for (const key of [...live, "fs_used", "fs_avail"]) expect(text, key).toMatch(new RegExp(`"${key}": "<(number|string)>"`));
    // the data stays, and so do the stored root numbers of the fixture snapshot (the fs list's numbers are live in both)
    expect(text).toContain('"entries": 3');
    expect(text).toContain('"fs_type": "ext4"');
    const fixture = readFileSync(join(r.out, "head/fixture.txt"), "utf8");
    for (const key of live) expect(fixture, key).not.toMatch(new RegExp(`"${key}": [^"]`));
    for (const key of ["fs_used", "fs_avail"]) expect(fixture, key).toMatch(new RegExp(`"${key}": \\d+`));
    // an age bound comes back as a live time and becomes the age in days, a date bound comes back as given and stays
    const answer = (q: string) => fixture.slice(fixture.indexOf(`== ${q}\n`)).split("\n== ")[0];
    expect(answer("top @ --older-than 2024-06-01 --json")).toContain('"older_than": "2024-06-01"');
    expect(answer("top @ --older-than 2024-06-01")).toContain("\nuntouched since 2024-06-01\n");
    expect(answer("find @ --newer-than 2024-06-01 --min-size 10K --kind file --json")).toContain('"newer_than": "2024-06-01"');
    for (const [q, days] of [
      ["top @ --older-than 1y --limit 5", '"older_than": 365,'],
      ["top @/media --older-than 400d --limit 3", '"older_than": 400,'],
      ["find @/media --older-than 2w", '"older_than": 14,'],
      ["top @/wide --kind dir --older-than 6mo --limit 6", '"older_than": 180,'],
      ["find @ --newer-than 3y --kind file --limit 8", '"newer_than": 1095,'],
    ]) {
      expect(answer(`${q} --json`)).toContain(days);
      expect(answer(q)).toContain("\nuntouched since <date>\n");
    }
    // state.json is compared with its times masked
    expect(text).toContain('-- state.json\n{\n  "last_success_ts": "<number>",\n  "consecutive_failures": 0\n}\n');
    expect(text).toContain("\nenv unset unset home <work>/home lc C\n");
    // the folder of each phase and what is in it, after its questions
    expect(fixture).toMatch(/^== files in <work>\/snap\n\. d \d+\n\.\/id f \d+\n/m);
    expect(fixture).toMatch(/^== files in <work>\/refreshed\n\. d \d+\n\.\/id f \d+\n\.\/refreshed f \d+\n/m);
    expect(text).toMatch(/^== files in <work>\/fresh\n\. d \d+\n\.\/id f \d+\n/m);
    // a refused question: the status and the stderr are recorded, in plain text also under --json, and the job goes on
    const refused = (json: string) => (tree: string) =>
      `== ls @/nope${json}\nexit 2\n-- stdout\n${json ? '{\n  "error": {\n    "code": "NOT_FOUND",\n    "exit": 2\n  }\n}\n' : ""}` +
      `-- stderr\nfreespace: is not in the snapshot: ls <work>/${tree}/nope${json}\nfix: freespace scan\n`;
    for (const [phase, tree] of [
      ["fixture", "tree"],
      ["fresh", "ftree"],
    ]) {
      const answers = readFileSync(join(r.out, `head/${phase}.txt`), "utf8");
      expect(answers).toContain(refused("")(tree));
      expect(answers).toContain(refused(" --json")(tree));
      expect(answers).toContain("== find @ --name *.iso\nexit 0\n");
    }
  },
  RUN_MS,
);

// Value: protects=a release bump alone does not fail the job: a build's own version (from its --version) is masked in
//   its answers, text and --json alike (help header, the version line, the error context), and nothing else is: another
//   version in an answer (the installed skill's) stays; fails_when=version_re is dropped (head prints 0.2.0 where base
//   prints 0.1.0) or the mask turns into any x.y.z (the skill's 9.9.9 goes); why_new=both stubs printed one version;
//   seam=none; no_claim=which of the real CLI's answers print the version (status, doctor, skill, help, error context)
test(
  "a head that differs only in its version matches: each build's own version is masked, no other",
  async () => {
    const r = await pending.same;
    expect(r.code).toBe(0);
    const [head, base] = ["head", "base"].map((run) => readFileSync(join(r.out, `${run}/fixture.txt`), "utf8"));
    expect(head).toContain("== --version\nexit 0\n-- stdout\nfreespace <version>\n-- stderr\n");
    expect(head).toContain("freespace <version> help\n");
    expect(head).toContain('"version": "<version>"');
    expect(head).toContain("host · linux · freespace <version>; skill 9.9.9, binary <version>\n");
    for (const text of [head, base]) {
      for (const version of ["0.1.0", "0.2.0"]) expect(text).not.toContain(version);
    }
  },
  RUN_MS,
);

// Value: protects=the binary's own texts are asked once each, as written, in the fixture phase: the bare run and its
//   --json, --version, --help, every command's --help, a usage error in both forms, the completions of the three shells,
//   schema and skill print; and every build runs as "freespace" whatever its file is called (base and same differ in
//   name, not in help); fails_when=an entry of ONCE or COMMANDS is dropped or gets a --json twin, they are asked in the
//   fresh phase too, or the link named freespace is dropped (Usage: base ...); why_new=the job asked no text of the
//   binary's own; seam=none; no_claim=the real CLI's texts, and that clap names the file (a stub prints the name)
test(
  "the binary's own texts are asked once, as written, under the name freespace",
  async () => {
    const r = await pending.same;
    const fixture = readFileSync(join(r.out, "head/fixture.txt"), "utf8");
    const commands = "ls top find diff gui serve status doctor scan refresh install-timer skill schema completions".split(" ");
    const asked = [
      "(no arguments)",
      "--json",
      "--version",
      "--help",
      "ls --no-such-flag",
      "ls --no-such-flag --json",
      ...["bash", "zsh", "fish"].map((shell) => `completions ${shell}`),
      "schema",
      "skill print",
      ...commands.map((command) => `${command} --help`),
    ];
    const headings = fixture.split("\n").filter((line) => line.startsWith("== "));
    for (const question of asked)
      expect(
        headings.filter((h) => h === `== ${question}`),
        question,
      ).toHaveLength(1);
    for (const twin of ["--version", "ls --help", "completions bash"]) expect(headings).not.toContain(`== ${twin} --json`);
    expect(readFileSync(join(r.out, "head/fresh.txt"), "utf8")).not.toContain("== --version");
    expect(fixture).toContain("== ls --help\nexit 0\n-- stdout\nfreespace <version> help\nUsage: freespace ls --help\n");
    expect(fixture).toContain("== ls --no-such-flag\nexit 2\n-- stdout\n-- stderr\nerror: unexpected argument\nUsage: freespace\n");
    expect(fixture).toContain('"code": "USAGE"');
    expect(fixture).not.toContain("Usage: base");
  },
  RUN_MS,
);

// Value: protects=refresh is exercised: each build refreshes its own copy of the fixture snapshot (one path, so the
//   answers read the same), five REFRESHES in a row, and the questions after them come from the copy, with the new
//   snapshot id masked; the fixture's own answers before it are not touched; fails_when=the copy is dropped (base2
//   starts from base's refreshes: exit 3), snap_id is not read again after a refresh (the new id stays in the answers:
//   exit 3), the refresh phase asks with fresh=false (live root numbers: exit 3), or an entry is dropped; why_new=no
//   question changed the snapshot; seam=none; no_claim=what the real refresh does (a stub counts and re-ids)
test(
  "refresh runs on a copy of the fixture snapshot, and what it leaves is asked",
  async () => {
    const r = await pending.same;
    const fixture = readFileSync(join(r.out, "head/fixture.txt"), "utf8");
    const fresh = readFileSync(join(r.out, "head/fresh.txt"), "utf8");
    const counts = (heading: string, pattern: string) =>
      [...fixture.matchAll(new RegExp(`^== ${heading}\\nexit 0\\n-- stdout\\n[\\s\\S]*?${pattern} (\\d+)`, "gm"))].map((m) => m[1]);
    // QUERIES ask before any refresh, AFTER_REFRESH after the three that work (the third and fourth are refused)
    for (const ask of ["ls @", "status"]) expect(counts(ask, "\\nrefreshed"), ask).toEqual(["0", "3"]);
    expect(counts("ls @ --json", '"refreshed":'), "json").toEqual(["0", "3"]);
    expect(fixture).toContain("== refresh @/logs --workers 2\nexit 0\n-- stdout\nargs refresh <work>/tree/logs --workers 2\n");
    expect(fixture).toContain("== refresh @/cache --workers 2 --json\nexit 0\n");
    expect(fixture).toContain("== refresh @/nope --workers 2\nexit 2\n");
    expect(fixture).toContain("== refresh @/top.bin --workers 2 --json\nexit 2\n");
    expect(fixture).toContain("== refresh @/wide/d1 --workers 4\nexit 0\n");
    // an id of a snapshot is 19 digits here: none is left in any answer
    for (const text of [fixture, fresh]) expect(text).not.toMatch(/\d{19}/);
    // the fresh phase refreshes the snapshot the build made itself, of its own tree
    expect(fresh).toContain("== refresh @/logs --workers 2\nexit 0\n-- stdout\nargs refresh <work>/ftree/logs --workers 2\n");
    // each scan and refresh walks the tree as the phase left it: the first scan before change_tree, the second after
    // it (cache/ came), the refreshes after drift_tree (old.log and cache/ went)
    const sees = (text: string) =>
      [...text.matchAll(/^== ((?:scan|refresh) \S+)[^\n]*\n(?:(?!== )[^\n]*\n)*?sees (.*)$/gm)].map((m) => `${m[1]}: ${m[2]}`);
    const refreshed = ["refresh @/logs: app.log debian.link new.log ", "refresh @/cache: ", "refresh @/wide/d1: d1 "];
    const root = "empty locked logs media odd projects top.bin wide ";
    expect(sees(fresh)).toEqual([`scan @: ${root}`, `scan @: cache ${root}`, ...refreshed]);
    expect(sees(fixture)).toEqual(refreshed);
  },
  RUN_MS,
);

test(
  "one changed data byte in head fails with the difference on stdout",
  async () => {
    const r = await pending.byte;
    expect(r.code).toBe(1);
    expect(r.stdout).toContain("-entries 3\n+entries 4\n");
    expect(r.stdout).toContain('-  "entries": 3\n+  "entries": 4\n');
    expect(r.stderr).toContain("head answers differently from base");
  },
  RUN_MS,
);

// Value: protects=the space doctor says the next scan needs is compared, only the space available is live; fails_when=
//   the mask takes "needs N free" for live free space again; why_new=the mask hid it, so a refactor that weakens the
//   precheck passed (adversarial review); seam=none; no_claim=the real CLI's figure (a stub prints it)
test(
  "a head whose next scan needs other space fails: that figure is not masked as live",
  async () => {
    const r = await pending.unlike;
    expect(r.code).toBe(1);
    expect(r.stdout).toContain(
      "-next scan needs 512 M free, <size> available; scanned in <seconds>\n+next scan needs 1.0 G free, <size> available; scanned in <seconds>\n",
    );
    expect(r.stderr).toContain("head answers differently from base");
  },
  RUN_MS,
);

// Value: protects=what a head leaves in the snapshot folder, $HOME and the unit dir is compared: a wider mode, a stray
//   file or another count in state.json fails the job; fails_when=files() is dropped, lists names only or skips
//   state.json, or kept() lists only the snapshot folder; why_new=only the answers were compared, and a head that opened
//   the folder to everyone passed (adversarial review); seam=none; no_claim=the real CLI's files and modes
test(
  "a head that leaves a stray file or a wider mode in the snapshot folder, $HOME or the unit dir fails",
  async () => {
    const r = await pending.unlike;
    expect(r.code).toBe(1);
    expect(r.stdout).toMatch(/^\+\.\/id f 666$/m);
    expect(r.stdout).toMatch(/^\+\.\/leftover\.tmp f \d+$/m);
    expect(r.stdout).toMatch(/^\+\.\/\.leak f \d+$/m);
    expect(r.stdout).toMatch(/^\+\.\/x\.service f \d+$/m);
    expect(r.stdout).toContain('\n-  "consecutive_failures": 0\n+  "consecutive_failures": 1\n');
  },
  RUN_MS,
);

// Value: protects=a call that hangs is stopped after RUST_DIFF_TIMEOUT seconds and answers exit 124, so the job names
//   the question; fails_when=run() loses its timeout (each hang then waits out the stub's 30 s sleep, past RUN_MS);
//   why_new=nothing bounded a call; seam=RUST_DIFF_TIMEOUT; no_claim=the 300 s default on CI
test(
  "a head that hangs on a question is stopped, and that question answers exit 124",
  async () => {
    const r = await pending.unlike;
    expect(r.code).toBe(1);
    expect(r.stdout).toContain(" == diff @\n-exit 0\n+exit 124\n");
  },
  RUN_MS,
);

// Value: protects=a head that refuses a question with another exit status than base fails the job, with the status line
//   in the difference; fails_when=the `exit` line leaves the answers or the status is not captured; why_new=no test
//   had head differ in anything but stdout; seam=none; no_claim=the real CLI's exit codes (a stub refuses /nope)
test(
  "a head that refuses a question with another exit status fails, with the status in the difference",
  async () => {
    const r = await pending.other;
    expect(r.code).toBe(1);
    expect(r.stdout).toContain("-exit 2\n+exit 5\n");
    expect(r.stderr).toContain("head answers differently from base");
  },
  RUN_MS,
);

// Value: protects=a head that words a refusal differently fails the job, the --json form too (its stderr is text, not
//   JSON); fails_when=stderr leaves the compared answers or its --json text is lost to the jq fallback; why_new=the
//   stub wrote no stderr before; seam=none; no_claim=the real CLI's messages (a stub refuses /nope as the CLI does)
test(
  "a head that words a refusal differently fails: stderr is compared as text, also under --json",
  async () => {
    const r = await pending.other;
    expect(r.code).toBe(1);
    expect(r.stdout).toContain("-freespace: is not in the snapshot: ls <work>/tree/nope\n+freespace: nothing at: ls <work>/tree/nope\n");
    expect(r.stdout).toContain(
      "-freespace: is not in the snapshot: ls <work>/tree/nope --json\n+freespace: nothing at: ls <work>/tree/nope --json\n",
    );
    expect(r.stderr).toContain("head answers differently from base");
  },
  RUN_MS,
);

test(
  "a base that answers two runs differently is unreliable: exit 3, and head never runs",
  async () => {
    const r = await pending.flaky;
    expect(r.code).toBe(3);
    // the answers are JSON (each pid a number), not text that differs anyway
    expect(r.stdout).toMatch(/^- {2}"pid": \d+,\n\+ {2}"pid": \d+,$/m);
    expect(r.stderr).toContain("the comparison is unreliable");
    expect(existsSync(join(r.out, "head"))).toBe(false);
  },
  RUN_MS,
);

// Value: protects=a base that cannot make the fixture snapshot stops the job at once and says why: exit 1, the base's own
//   output, then the message naming the scan (the first, or the second after the tree changed), and nothing answered (no
//   base/ or head/ in the out dir); fails_when=the `if !` around the scan is dropped (set -e ends the job with the scan's
//   status 7 and no message), the scan's output is not shown, or the step number is not the scan's; why_new=every base so
//   far scanned fine; seam=none; no_claim=what the real scan prints when it fails (a stub says it is on fire)
test("a base that cannot make the fixture snapshot stops the job at once and shows why", async () => {
  for (const step of [1, 2]) {
    const r = await guard(`noscan${step}`, "base");
    expect(r.code, `scan ${step}`).toBe(1);
    expect(r.stderr).toMatch(
      new RegExp(`^disk on fire in scan ${step}\\n.*: base could not make the fixture snapshot \\(scan ${step}\\)\\n$`),
    );
    expect(existsSync(join(r.out, "head"))).toBe(false);
    expect(readdirSync(r.out)).toEqual([]);
  }
}, 30_000);

test("bad input exits 2 before anything runs", async () => {
  const none = Bun.spawnSync([script], { stderr: "pipe" });
  expect(none.exitCode).toBe(2);
  expect(none.stderr.toString()).toContain("usage:");
  for (const bad of ["not-executable", "missing"]) {
    const r = await guard("base", bad);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain(`not an executable file: ${join(tmp, bad)}`);
    expect(existsSync(r.out)).toBe(false);
  }
});

// Value: protects=a missing tool, or a run as root, stops the job before anything runs: exit 2, no out dir;
//   fails_when=a tool leaves the check (the job then dies midway at a command not found) or the root check goes
//   (install-timer would install the build on this host); why_new=nothing ran these checks; seam=PATH; no_claim=a real
//   run as root (a stub `id` says 0)
test("a missing tool or a run as root stops the job before anything runs", () => {
  const [few, root] = ["few", "root"].map((name) => join(tmp, `path-${name}`));
  for (const dir of [few, root]) mkdirSync(dir, { recursive: true });
  for (const tool of ["jq", "curl"]) symlinkSync(Bun.which(tool) ?? tool, join(few, tool));
  writeFileSync(join(root, "id"), "#!/bin/sh\necho 0\n");
  chmodSync(join(root, "id"), 0o755);
  for (const [PATH, said] of [
    [few, "script is required"],
    [`${root}:${process.env.PATH}`, "run it as a user, not root"],
  ] as const) {
    const out = join(tmp, `checked-${runs++}`);
    const r = Bun.spawnSync([Bun.which("bash") ?? "bash", script, join(tmp, "base"), join(tmp, "base"), out], {
      env: { ...process.env, PATH },
      stderr: "pipe",
    });
    expect(r.exitCode, said).toBe(2);
    expect(r.stderr.toString()).toContain(said);
    expect(existsSync(out)).toBe(false);
  }
});

// Value: protects=the floor: base must give the exit status the harness expects for every question (0 unless EXPECTED
//   lists another), a refusal with the words EXPECTED lists for it, a --json answer of 0 in JSON, and each request to
//   serve its HTTP status, or the job stops as stale (exit 4) after base's first run, naming each question, before base
//   again or head run; fails_when=floor() is dropped or not called from ask or http, the EXPECTED lookup takes the wrong
//   phase, the words or the JSON go unchecked, or the check runs after head; why_new=a flag both builds refuse alike
//   compared equal and tested nothing (adversarial review of PR 2: 10 exit 2 blocks, job green), and exit 2 stands for
//   a usage error as much as a refusal (adversarial review of A0); seam=none; no_claim=which flags a future CLI renames
test(
  "a base that refuses a question the harness asks, or answers one it should refuse, stops the job as stale",
  async () => {
    const r = await pending.stale;
    expect(r.code).toBe(4);
    for (const line of [
      "fixture: top @ --min-size 100K --sort mtime: answered 2, the harness expects 0",
      "fresh: top @ --min-size 100K --sort mtime: answered 2, the harness expects 0",
      "once: install-timer --no-start: answered 0, the harness expects 1",
      'fixture: ls @/nope: answered 2 without "is not in the snapshot" on stderr',
      "fixture: doctor --json: answered 0 without JSON",
      "serve: a POST: answered 200, the harness expects 405",
    ]) {
      expect(r.stderr).toContain(`${line}\n`);
    }
    expect(r.stderr).toContain("the harness is stale");
    expect(readdirSync(r.out)).toEqual(["base"]);
  },
  RUN_MS,
);

// Value: protects=a base whose server never prints its address stops the job as stale, and the answers say so;
//   fails_when=the wait for the address or its floor goes; why_new=every stub served (testing review of A0); seam=none;
//   no_claim=why a real server fails to start
test(
  "a base whose server does not start stops the job as stale",
  async () => {
    const r = await pending.noserve;
    expect(r.code).toBe(4);
    expect(r.stderr).toContain("serve: start: answered no address, the harness expects an address\n");
    expect(readFileSync(join(r.out, "base/fixture.txt"), "utf8")).toContain(
      "== serve did not print its address\n== serve stopped by TERM\nexit 1\n",
    );
  },
  RUN_MS,
);

// Value: protects=no command may change the tree it reads: a build that links, dates back, narrows or cuts a file of
//   the tree stops the job at once, with the listing's difference and the phase, in the fixture, fresh and config phases;
//   fails_when=tree_check is dropped from one of them or the listing leaves out names, modes, sizes, links or mtimes;
//   why_new=only the snapshot folder was listed (freespace never changes what it scans, CONTRIBUTING), and only the
//   fixture phase was tried (testing review of A0); seam=none; no_claim=changes the listing does not see (access and
//   change times)
test(
  "a build that changes the tree it reads stops the job with the change and the phase",
  async () => {
    for (const [name, bin, phase, change] of [
      ["tamper", "head", "the fixture phase", /^\+\.top\.link f \d+ 102400 2 [\d.]+$/m],
      ["retouch", "base", "its second scan and the questions", /^\+top\.bin f 600 102400 1 1577836800\.0+$/m],
      ["shrink", "base", "the config phase", /^\+projects\/app\/src\/main\.rs f \d+ 1 1 [\d.]+$/m],
    ] as const) {
      const r = await pending[name];
      expect(r.code, name).toBe(1);
      expect(r.stderr, name).toMatch(change);
      expect(r.stderr, name).toContain(`/bin/${bin}/freespace changed the tree it read`);
      expect(r.stderr, name).toContain(`during ${phase}\n`);
    }
    expect((await pending.tamper).stderr).toMatch(/^\+top\.bin f \d+ 102400 2 /m);
  },
  RUN_MS,
);

// Value: protects=serve is asked: each build serves the fixture snapshot on a free port, the requests with the token and
//   the refusals get their status, headers (no Date, no Content-Length) and normalized body, the page its status and
//   headers only, and the server stops on TERM; port and token are masked; fails_when=the serve phase is dropped, the
//   token is not sent, a header leaves the comparison or the page's bytes enter it (they move with every web build);
//   why_new=the HTTP API was not compared at all (D11); seam=none; no_claim=the real server's rules (a stub serves them)
test(
  "serve answers every request, its refusals too, and stops on TERM",
  async () => {
    const r = await pending.same;
    const fixture = readFileSync(join(r.out, "head/fixture.txt"), "utf8");
    const headers = "-- headers\ncache-control: no-store\ncontent-type: application/json\nx-frame-options: DENY\n-- body\n";
    expect(fixture).toContain(`== serve: status\nhttp 200\n${headers}{\n  "scanned_at": "<number>",\n  "entries": 3\n}\n`);
    expect(fixture).toContain(`== serve: ls of a snapshot that is gone\nhttp 409\n${headers}`);
    expect(fixture).toContain(`== serve: a bad limit\nhttp 400\n${headers}`);
    expect(fixture).toContain('"path": "<work>/tree"');
    for (const [label, code] of [
      ["no token", 403],
      ["a wrong token", 403],
      ["a foreign origin", 403],
      ["another host name", 403],
      ["a POST", 405],
      ["the page", 200],
    ] as const) {
      expect(fixture, label).toContain(`== serve: ${label}\nhttp ${code}\n`);
    }
    // a body without a newline at its end says so on a line of its own
    expect(fixture).toContain(
      "answers on 127.0.0.1:<port> only; ssh -N -L <port>:127.0.0.1:<port>\n\\ no newline at end\n== serve: a POST",
    );
    expect(fixture).toContain("== serve: the page\nhttp 200\n-- headers\ncache-control: no-store\ncontent-type: text/html\n");
    expect(fixture).toContain("(the web bundle: pixel-diff compares it)\n");
    expect(fixture).toContain(
      "== serve stopped by TERM\nexit 0\n-- stdout\n-- stderr\nfreespace serve: http://127.0.0.1:<port>/#t=<token>\npress Ctrl-C to stop\n",
    );
    expect(fixture).not.toMatch(/^(date|content-length):/m);
  },
  RUN_MS,
);

// Value: protects=a header the server answers differently fails the job; fails_when=headers leave the serve answers;
//   why_new=the HTTP API was not compared; seam=none; no_claim=the real server's headers
test(
  "a head whose server answers with another header fails",
  async () => {
    const r = await pending.other;
    expect(r.code).toBe(1);
    expect(r.stdout).toContain("-x-frame-options: DENY\n+x-frame-options: SAMEORIGIN\n");
  },
  RUN_MS,
);

// Value: protects=the date questions are asked again in a time zone at a half-hour offset, and the terminal questions
//   on a terminal (script), text only, with no carriage returns; fails_when=$tz does not reach the CLI's TZ, ask_tty runs
//   the CLI without a terminal, or the CR mask is dropped; why_new=every answer ran in UTC and none on a terminal (D11);
//   seam=none; no_claim=the real CLI's local-midnight rule and its terminal output (the stub prints TZ and the terminal)
test(
  "date questions run in another time zone too, and terminal questions on a terminal",
  async () => {
    const r = await pending.same;
    const fixture = readFileSync(join(r.out, "head/fixture.txt"), "utf8");
    const zone = fixture.slice(fixture.indexOf("== TZ=Asia/Kolkata\n"), fixture.indexOf("== on a terminal: "));
    expect(zone).toContain("== find @ --newer-than 2024-10-01 --kind file\nexit 0\n");
    expect(zone).toContain("\ntz Asia/Kolkata\n");
    expect(zone).not.toContain("\ntz UTC\n");
    expect(fixture.slice(0, fixture.indexOf("== TZ="))).toContain("\ntz UTC\n");
    expect(fixture).toContain("== on a terminal: status\nexit 0\nargs status\n");
    const tty = fixture.slice(fixture.indexOf("== on a terminal: "), fixture.indexOf("== serve: "));
    expect(tty.match(/^on a terminal$/gm)).toHaveLength(4);
    expect(fixture).not.toContain("\r");
  },
  RUN_MS,
);

// Value: protects=the config phase: a config file with two roots, `scan` without a root, a refusal for a path above both
//   and for diff without a previous snapshot, then a config with an unknown key refused; and each part lists what the CLI
//   keeps in $HOME and the unit dir; fails_when=$config does not reach FREESPACE_CONFIG, the phase is dropped, or kept()
//   lists only the snapshot folder; why_new=no answer read a config file, and nothing looked outside the snapshot folder
//   (D11); seam=none; no_claim=the real config's keys and rules (a stub refuses as the CLI does)
test(
  "a config file's roots are scanned and asked, an unknown key refused, and $HOME and the unit dir listed",
  async () => {
    const r = await pending.same;
    const config = readFileSync(join(r.out, "head/config.txt"), "utf8");
    expect(config).toContain("== scan --json\nexit 0\n");
    expect(config).toContain("== diff @/media\nexit 3\n");
    expect(config).toContain("== find @ --name *.iso\nexit 2\n-- stdout\n-- stderr\nfreespace: outside the scanned roots\n");
    expect(config).toContain("== find @/media --name *.iso\nexit 0\n");
    expect(config).toContain("== status\nexit 2\n-- stdout\n-- stderr\nfreespace: unknown field keep_old\n");
    for (const dir of ["home", "units"]) expect(config).toMatch(new RegExp(`^== files in <work>/${dir}\\n\\. d \\d+\\n`, "m"));
  },
  RUN_MS,
);

// Value: protects=the start phase: with no snapshot folder each question is refused as the CLI refuses it and the
//   folder stays missing; while a scan holds the lock, the first scan's questions say so, and over the fixture snapshot
//   status answers while scan and refresh are refused; fails_when=the phase is dropped, the lock is not held while the
//   questions run, or files() fails on a folder that is not there; why_new=no answer came before the first snapshot or
//   during a scan (adversarial review of A0); seam=none; no_claim=the real CLI's lock rules (a stub probes the lock)
test(
  "before the first snapshot and while a scan holds the lock, each question is asked",
  async () => {
    const r = await pending.same;
    const start = readFileSync(join(r.out, "head/start.txt"), "utf8");
    const refused = (q: string, why: string) => `== ${q}\nexit 8\n-- stdout\n-- stderr\nfreespace: no snapshot in <work>/${why}\n`;
    expect(start).toContain(refused("status", "empty"));
    expect(start).toContain("== files in <work>/empty\n(missing)\n");
    expect(start).toContain(refused("ls @", "first yet: a scan running since 00:00, wait"));
    expect(start).toMatch(/^== status\nexit 0\n-- stdout\nargs status\n[^=]*\nheld true\n/m);
    expect(start).toContain("== scan @ --workers 2\nexit 6\n-- stdout\n-- stderr\nfreespace: another scan is running\n");
    expect(start).toMatch(/^== files in <work>\/busy\n\. d \d+\n\.\/\.lock f \d+\n\.\/id f \d+\n/m);
  },
  RUN_MS,
);

// A base that holds its first call, the scan of the fixture tree: it says so in $STUB_HELD, then waits for $STUB_GO
// (10 s at most, so a failed test cannot leave it running). The script is then past making its work dir and its tree.
const HOLD = `#!/bin/sh
: > "$STUB_HELD"
n=0
while [ ! -e "$STUB_GO" ] && [ $n -lt 500 ]; do sleep 0.02; n=$((n + 1)); done
`;

// A poll with a deadline of 10 s for something another process does; nothing to wait on but the file itself.
async function until(done: () => boolean): Promise<boolean> {
  // gstack-timing-ok: poll with a deadline for a marker file a child process writes, the outcome is asserted after it
  for (let i = 0; i < 500 && !done(); i++) await Bun.sleep(20);
  return done();
}

// bash runs a trap when the foreground command ends, so the signal is sent first and the stub released after it.
// Value: protects=a HUP, INT or TERM ends rust-diff.sh with 129, 130 or 143 and removes its work dir, the folder nobody
//   may read in it too; fails_when=a `trap 'exit N' SIG` line is dropped or changed, or the EXIT trap loses its chmod and
//   the work dir stays on the runner; why_new=no test interrupted the script, every run went to the end; seam=none;
//   no_claim=GitHub's job cancellation and a real scan in progress (a stub base holds the first scan, signal sent to the script)
for (const [signal, code] of [
  ["SIGTERM", 143],
  ["SIGINT", 130],
  ["SIGHUP", 129],
] as const) {
  test(`a ${signal} while base scans exits ${code} and removes the work dir with its unreadable folder`, async () => {
    const dir = join(tmp, `signal-${signal}`);
    const scratch = join(dir, "tmp");
    const [held, go] = [join(dir, "held"), join(dir, "go")];
    mkdirSync(scratch, { recursive: true });
    writeFileSync(join(dir, "hold"), HOLD);
    chmodSync(join(dir, "hold"), 0o755);
    // TMPDIR is where the script's mktemp puts the work dir: anything left in it after the exit is a leftover
    const env = { ...process.env, TMPDIR: scratch, STUB_HELD: held, STUB_GO: go, RUST_DIFF_FAN: "1" };
    const proc = Bun.spawn([script, join(dir, "hold"), join(dir, "hold"), join(dir, "out")], { env, stdout: "ignore", stderr: "ignore" });
    try {
      expect(await until(() => existsSync(held)), "the script reached its first scan").toBe(true);
      expect(readdirSync(scratch)).toHaveLength(1);
      // what the EXIT trap has to get rid of: a folder with mode 000, which rm cannot enter
      expect(statSync(join(scratch, readdirSync(scratch)[0] ?? "", "tree/locked")).mode & 0o777).toBe(0);
      proc.kill(signal);
      writeFileSync(go, "");
      expect(await proc.exited).toBe(code);
      expect(proc.signalCode, "the trap ended the script, not the signal").toBeNull();
      expect(readdirSync(scratch)).toEqual([]);
    } finally {
      writeFileSync(go, "");
      proc.kill();
      Bun.spawnSync(["chmod", "-R", "u+rwx", scratch]);
    }
  }, 30_000);
}

// Value: protects=a Ctrl-C (SIGINT to the job's process group, as a terminal sends it) ends the job at once, the call in
//   progress too; fails_when=timeout runs in a process group of its own again (no --foreground): the call then holds the
//   job until it ends; why_new=the signal tests signal the script alone (adversarial review of A0); seam=setsid;
//   no_claim=how GitHub cancels a job
test("a Ctrl-C reaches the call in progress and ends the job at once", async () => {
  const dir = join(tmp, "ctrl-c");
  const scratch = join(dir, "tmp");
  const [held, go] = [join(dir, "held"), join(dir, "go")];
  mkdirSync(scratch, { recursive: true });
  writeFileSync(join(dir, "hold"), HOLD);
  chmodSync(join(dir, "hold"), 0o755);
  const env = { ...process.env, TMPDIR: scratch, STUB_HELD: held, STUB_GO: go, RUST_DIFF_FAN: "1" };
  // its own process group, as a job started from a terminal
  const proc = Bun.spawn(["setsid", script, join(dir, "hold"), join(dir, "hold"), join(dir, "out")], {
    env,
    stdout: "ignore",
    stderr: "ignore",
  });
  try {
    expect(await until(() => existsSync(held)), "the script reached its first scan").toBe(true);
    const sent = Date.now();
    process.kill(-proc.pid, "SIGINT");
    expect(await proc.exited).toBe(130);
    // the stub holds 10 s unless the signal reaches it
    expect(Date.now() - sent).toBeLessThan(5_000);
    expect(readdirSync(scratch)).toEqual([]);
  } finally {
    writeFileSync(go, "");
    proc.kill();
    Bun.spawnSync(["chmod", "-R", "u+rwx", scratch]);
  }
}, 30_000);

// Value: protects=the made-up trees hold what CONTRIBUTING says they hold, after each change too: every folder its own
//   date, the wide part's 340 folders, hard links inside a folder, across folders and into the folder refresh rewalks,
//   a sparse file, a name that is not UTF-8, a folder nobody may read; and the listing that tree_check compares sees a
//   change of mtime, mode, size or link count; fails_when=settle_dirs gives folders one date (`--sort mtime` then
//   compares only tie-breaks), make_wide loses a level or its links, a blob stops being sparse, or tree_listing drops a
//   field; why_new=the guard runs use one folder per level and assert answers, not the tree; seam=none; no_claim=what
//   the CLI makes of the tree (the guard runs compare that)
// The tree at the default width, made, changed and drifted: a few seconds, more under the guard runs beforeAll starts.
test(
  "the trees hold distinct folder dates, hard links, a sparse file, an odd name and a locked folder",
  async () => {
    const tree = join(tmp, "shape");
    const sh = `. "$1/scripts/rust-diff/tree.sh"
dates() { echo "$1 dirs $(find "$tree" -type d 2>/dev/null | wc -l) dates $(find "$tree" -type d -printf '%T@\\n' 2>/dev/null | sort -u | wc -l)"; }
make_tree; dates made
echo "wide $(find "$tree/wide" -mindepth 1 -type d | wc -l)"
echo "iso links $(stat -c %h "$tree/media/iso/debian.iso") $(stat -c %h "$tree/logs/debian.link")"
echo "wide links $(find "$tree/wide" -name 'link*.dat' -links 2 | wc -l)"
echo "sparse $(stat -c '%s %b' "$tree/media/sparse.img")"
[ -e "$tree/odd/$(printf 'bad\\377name.bin')" ] && echo "odd name"
echo "locked $(stat -c %a "$tree/locked")"
tree_listing > "$tree.listing"
touch -d "2020-01-01 00:00 UTC" "$tree/top.bin"; chmod 600 "$tree/logs/app.log"; truncate -s 1 "$tree/media/edge-new.bin"
ln "$tree/projects/app/src/main.rs" "$tree/projects/app/main.link"
tree_listing | diff "$tree.listing" - | grep '^>' || true
change_tree; dates changed
drift_tree; dates drifted
remove_tree; [ -e "$tree" ] || echo removed`;
    // the default width: no RUST_DIFF_FAN
    const { RUST_DIFF_FAN: _fan, ...env } = process.env;
    const p = Bun.spawn(["bash", "-c", `set -euo pipefail; tree=$2 epoch=$(date +%s)\n${sh}`, "shape", repo, tree], {
      env,
      stdout: "pipe",
      stderr: "pipe",
    });
    const [out, err, code] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited]);
    expect(err).toBe("");
    expect(code).toBe(0);
    for (const step of ["made", "changed", "drifted"]) {
      const [, dirs, dates] = out.match(new RegExp(`^${step} dirs (\\d+) dates (\\d+)$`, "m")) ?? [];
      expect(Number(dirs), step).toBeGreaterThan(300);
      expect(dates, `every folder its own date after ${step}`).toBe(dirs);
    }
    expect(out).toContain("\nwide 340\n");
    expect(out).toContain("\niso links 3 3\n");
    expect(Number(out.match(/^wide links (\d+)$/m)?.[1])).toBeGreaterThan(0);
    expect(out).toContain("\nsparse 52428800 0\n");
    expect(out).toContain("\nodd name\n");
    expect(out).toContain("\nlocked 0\n");
    for (const changed of [
      /^> top\.bin f \d+ 102400 1 1577836800\.0+$/m,
      /^> logs\/app\.log f 600 /m,
      /^> media\/edge-new\.bin f \d+ 1 1 /m,
      /^> projects\/app\/src\/main\.rs f \d+ 12288 2 /m,
      /^> projects\/app\/main\.link f \d+ 12288 2 /m,
    ]) {
      expect(out).toMatch(changed);
    }
    expect(out.endsWith("removed\n")).toBe(true);
  },
  RUN_MS,
);
