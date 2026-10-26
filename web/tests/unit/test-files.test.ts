// scripts/test-files.sh, the pre-gate runner: Rust runs once per crate in the repo root, web files use their runners in web/. A failing runner's
// status is the script's, shots and guard shots render into web/pregate-shots (cleared under the lock, gone at exit),
// a signal reaches the running runner before the script exits, and bad input exits 2 before anything runs.
// Stub cargo, bun and bunx stand in for the real runners; they record argv, cwd, $SHOTS_DIR, $E2E_PORT, an open fd 9,
// $GUARD_SHOTS and what web/pregate-shots holds when they start.
import { afterAll, beforeAll, expect, test } from "bun:test";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const repo = realpathSync(join(import.meta.dir, "../../.."));
const web = join(repo, "web");

// `bun -e` (the script's free-port pick) prints PORT. Any other call appends
// "name|cwd|argv|$SHOTS_DIR|$E2E_PORT|fd9|$GUARD_SHOTS|left" to $STUB_LOG (fd9 is "open" when the script's lock reached
// the runner; left lists what web/pregate-shots holds as the runner starts). If $STUB_GO is set, it waits until that
// file exists (10 s at most) and appends a "released" line; a TERM meanwhile appends a "stopped" line after 0.3 s, as a
// runner tearing down, and exits 143. With $STUB_SHOT and $GUARD_SHOTS set it takes a shot: view.png holds $STUB_SHOT,
// or the stub's pid if that is "pid". Exits with $STUB_EXIT (default 0).
const PORT = "41730";
const stub = `#!/bin/sh
if [ "$1" = -e ]; then echo ${PORT}; exit 0; fi
# set before the call is logged: a test signals as soon as it sees the call
[ -z "\${STUB_GO:-}" ] || trap 'trap "" TERM; sleep 0.3; echo "stopped|||||||" >> "$STUB_LOG"; exit 143' TERM
fd9=; [ ! -e /proc/self/fd/9 ] || fd9=open
left=$(find pregate-shots -mindepth 1 2>/dev/null | sort | paste -sd, -)
printf '%s|%s|%s|%s|%s|%s|%s|%s\\n' "\${0##*/}" "$(pwd -P)" "$*" "\${SHOTS_DIR:-}" "\${E2E_PORT:-}" "$fd9" "\${GUARD_SHOTS:-}" "$left" >> "$STUB_LOG"
if [ -n "\${STUB_SHOT:-}" ] && [ -n "\${GUARD_SHOTS:-}" ]; then
  shot=$STUB_SHOT; [ "$shot" != pid ] || shot=$$
  printf '%s' "$shot" > "$GUARD_SHOTS/view.png"
fi
if [ -n "\${STUB_GO:-}" ]; then
  n=0
  while [ ! -e "$STUB_GO" ] && [ $n -lt 500 ]; do sleep 0.02; n=$((n + 1)); done
  echo "released|||||||" >> "$STUB_LOG"
fi
exit "\${STUB_EXIT:-0}"
`;

let tmp = "";
let runs = 0;
// A copy of the script in tmp/repo, so it runs in tmp/repo/web and its lock on web/ never waits for a real e2e run in
// this checkout. The test files it is given exist there, empty.
let script = "";
let stubWeb = "";
// Where the copy's runs shoot: web/pregate-shots of tmp/repo.
let shotsRoot = "";

beforeAll(() => {
  tmp = realpathSync(mkdtempSync(join(tmpdir(), "test-files-")));
  script = join(tmp, "repo/scripts/test-files.sh");
  stubWeb = join(tmp, "repo/web");
  shotsRoot = join(stubWeb, "pregate-shots");
  mkdirSync(join(tmp, "repo/scripts"), { recursive: true });
  mkdirSync(stubWeb);
  copyFileSync(join(repo, "scripts/test-files.sh"), script);
  for (const f of testFiles) {
    mkdirSync(dirname(join(tmp, "repo", f)), { recursive: true });
    writeFileSync(join(tmp, "repo", f), "");
  }
  mkdirSync(join(tmp, "bin"));
  mkdirSync(join(tmp, "bunx-only"));
  for (const path of ["bin/bun", "bin/bunx", "bin/cargo", "bunx-only/bunx"]) {
    writeFileSync(join(tmp, path), stub);
    chmodSync(join(tmp, path), 0o755);
  }
  serveCopy();
});

afterAll(() => {
  rmSync(tmp, { recursive: true, force: true });
});

// The environment of one script run: the stubs first on PATH, a fresh call log, no inherited $SHOTS_DIR, $E2E_PORT,
// $GUARD_SHOTS or $TEST_FILES_HEARTBEAT, then `extra`.
function stubEnv(stubExit = 0, extra: Record<string, string> = {}) {
  const log = join(tmp, `${runs++}.log`);
  const env: Record<string, string | undefined> = {
    ...process.env,
    PATH: `${join(tmp, "bin")}:${process.env.PATH}`,
    STUB_LOG: log,
    STUB_EXIT: String(stubExit),
  };
  for (const name of ["SHOTS_DIR", "E2E_PORT", "GUARD_SHOTS", "TEST_FILES_HEARTBEAT"]) delete env[name];
  return { log, env: { ...env, ...extra } };
}

type StubLine = [string, string, string, string, string, string, string, string];

// The stub's lines in order: its calls, and the "released" and "stopped" lines (empty but for the name).
function readCalls(log: string) {
  const lines = existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(Boolean) : [];
  return lines.map((l) => {
    const fields = l.split("|");
    if (fields.length !== 8) throw new Error(`stub line with ${fields.length} fields: ${l}`);
    const [name, cwd, argv, shotsDir, port, fd9, guardDir, left] = fields as StubLine;
    return { name, cwd, argv, shotsDir, port, fd9, guardDir, left };
  });
}

// Runs the script from outside the repo; returns its status, stderr and the stub calls.
function run(args: string[], stubExit = 0) {
  const { log, env } = stubEnv(stubExit);
  const done = Bun.spawnSync(["sh", script, ...args], { cwd: tmp, env });
  return { code: done.exitCode, stderr: done.stderr.toString(), calls: readCalls(log) };
}

// Starts the script on `args` in the background, from outside the repo, with its stderr piped; `sh` is the command
// before the script.
function start(args: string[], env: Record<string, string | undefined>, sh = ["sh"]) {
  return Bun.spawn([...sh, script, ...args], { cwd: tmp, env, stderr: "pipe" });
}

// Polls `cond` every 20 ms; throws when `ms` pass without it.
async function waitFor(cond: () => boolean, what: string, ms = 5000) {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await Bun.sleep(20);
  }
}

// Reads `stream` until what it has read holds `text`, or it ends; returns what it read.
async function readUntil(stream: ReadableStream<Uint8Array>, text: string) {
  const reader = stream.getReader();
  let said = "";
  while (!said.includes(text)) {
    const chunk = await reader.read();
    if (chunk.done) break;
    said += new TextDecoder().decode(chunk.value);
  }
  reader.releaseLock();
  return said;
}

const testFiles = [
  "core/src/lib.rs",
  "core/src/scan/mod.rs",
  "core/tests/scan.rs",
  "cli/src/serve/gui.rs",
  "cli/tests/cli.rs",
  "cli/tests/common/mod.rs",
  "web/tests/unit/format.test.ts",
  "web/tests/unit/contrast.test.ts",
  "web/tests/unit/helper.ts",
  "web/tests/e2e/find.spec.ts",
  "web/tests/e2e/browse.spec.ts",
  "web/tests/shots/readme.spec.ts",
  "web/tests/shots/b.spec.ts",
  "web/tests/shots/home.ts",
  "web/tests/guard/views.spec.ts",
];
const unit = "web/tests/unit/format.test.ts";
const e2e = "web/tests/e2e/find.spec.ts";
const shots = "web/tests/shots/readme.spec.ts";
const unitCall = ["bun", "test ./tests/unit/format.test.ts"];
const e2eCall = ["bunx", "playwright test tests/e2e/find.spec.ts"];
const shotsCall = ["bunx", "playwright test -c playwright.shots.config.ts tests/shots/readme.spec.ts"];
const guard = "web/tests/guard/views.spec.ts";
const guardCall = ["bunx", "playwright test -c playwright.guard.config.ts --update-snapshots=all tests/guard/views.spec.ts"];
const guardCalls = [guardCall, guardCall];

type Row = { name: string; args: string[]; calls: string[][]; code?: number; stubExit?: number; stderr?: string };

const rustCore = "core/src/lib.rs";
const rustCli = "cli/src/serve/gui.rs";
const cargoCall = (crate: string) => ["cargo", `test -j 2 --manifest-path ${join(tmp, "repo", crate, "Cargo.toml")} -- --test-threads=1`];

// Value: protects=Rust files selected by pregate reach cargo once per crate alongside web runners;
//   fails_when=the custom runner rejects cfg(test) modules/helpers, repeats a crate, loses failures or partially runs bad input;
//   why_new=existing runner cases cover web paths only; seam=none; no_claim=compiling or executing Rust tests (cargo is stubbed)
const rustRows: Row[] = [
  { name: "Rust core source runs its crate", args: [rustCore], calls: [["core"]] },
  { name: "Rust CLI source runs its crate", args: [rustCli], calls: [["cli"]] },
  {
    name: "Rust source, integration tests and common helpers run each selected crate once",
    args: [rustCli, rustCore, "cli/tests/cli.rs", "core/src/scan/mod.rs", "core/tests/scan.rs", "cli/tests/common/mod.rs", rustCli],
    calls: [["core"], ["cli"]],
  },
  { name: "Rust and web files retain all runners", args: [unit, rustCli, rustCore], calls: [["core"], ["cli"], unitCall] },
  {
    name: "Rust cargo failure stops later crates and web runners",
    args: [unit, rustCli, rustCore],
    calls: [["core"]],
    stubExit: 7,
    code: 7,
  },
  ...[
    ["core/src/gone.rs", "no such test file"],
    ["other/src/lib.rs", "no runner for"],
    ["core/Cargo.toml", "no runner for"],
    ["cli/src/../main.rs", "unsafe test path"],
    ["cli/src/bad;name.rs", "unsafe test path"],
  ].map(([file, error]) => ({
    name: `Rust selection rejects ${file} before any runner starts`,
    args: [rustCore, unit, file],
    calls: [],
    code: 2,
    stderr: `${error} ${file}`,
  })),
];

for (const row of rustRows) {
  test(row.name, () => {
    const got = run(row.args, row.stubExit);
    const calls = row.calls.map((call) => (call.length === 1 ? cargoCall(call[0]) : call));
    expect(got.code, got.stderr).toBe(row.code ?? 0);
    expect(got.calls.map((c) => [c.name, c.argv])).toEqual(calls);
    expect(got.calls.map((c) => c.cwd)).toEqual(calls.map(([runner]) => (runner === "cargo" ? dirname(stubWeb) : stubWeb)));
    if (row.stderr) expect(got.stderr).toContain(row.stderr);
  });
}

// Value: protects=signals stop Cargo and its test child before the runner returns, including a TERM-resistant Cargo;
//   fails_when=only Cargo receives TERM, group escalation is absent or a signal during session startup leaks the run;
//   why_new=existing signal cases select web runners, and the routing Cargo stub spawns no subprocess;
//   seam=none; no_claim=processes that deliberately escape their Cargo session
function cargoTreeFixture(escalate: boolean, early: boolean) {
  const dir = join(tmp, `cargo-tree-${runs++}`);
  mkdirSync(dir);
  const pids = join(dir, "pids");
  const launched = join(dir, "launched");
  const body = `#!/bin/sh
/bin/sleep 60 &
spawned=$!
if [ "$CARGO_ESCALATE" = yes ]; then trap '' TERM; else trap 'wait "$spawned" 2>/dev/null || :; exit 143' TERM; fi
echo "$$ $spawned" > "$CARGO_PIDS"
wait "$spawned" || :
if [ "$CARGO_ESCALATE" = yes ]; then while :; do :; done; fi
`;
  writeFileSync(join(dir, "cargo"), body, { mode: 0o755 });
  if (early) {
    writeFileSync(join(dir, "setsid"), '#!/bin/sh\necho "$$" > "$CARGO_LAUNCHED"\nkill -TERM "$PPID"\nexec "$REAL_SETSID" "$@"\n', {
      mode: 0o755,
    });
  }
  const { env } = stubEnv(0, {
    PATH: `${dir}:${join(tmp, "bin")}:${process.env.PATH}`,
    CARGO_PIDS: pids,
    CARGO_LAUNCHED: launched,
    CARGO_ESCALATE: escalate ? "yes" : "no",
    REAL_SETSID: Bun.which("setsid") ?? "/usr/bin/setsid",
  });
  return { env, pids, launched };
}

function recordedPids(path: string) {
  return existsSync(path)
    ? readFileSync(path, "utf8")
        .trim()
        .split(/\s+/)
        .map(Number)
        .filter((pid) => pid > 1)
    : [];
}

function signalOwned(pid: number, signal: NodeJS.Signals) {
  try {
    process.kill(pid, signal);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
  }
}

async function stopCargoFixture(proc: ReturnType<typeof start>, pids: string, launched: string) {
  // Kill the test child first so its Cargo parent can wait/reap it, including on the old broken runner.
  const [cargo, child] = recordedPids(pids);
  if (child) signalOwned(child, "SIGTERM");
  proc.kill("SIGTERM");
  await Promise.race([proc.exited, Bun.sleep(1000)]);
  for (const pid of [child, cargo, ...recordedPids(launched)]) if (pid) signalOwned(pid, "SIGKILL");
  proc.kill("SIGKILL");
  await proc.exited;
}

for (const [signal, code, escalate, early] of [
  ["SIGHUP", 129, false, false],
  ["SIGINT", 130, false, false],
  ["SIGTERM", 143, false, false],
  ["SIGTERM", 143, true, false],
  ["SIGTERM", 143, false, true],
] as const) {
  test(`Cargo tree stops on ${signal}${escalate ? " with escalation" : ""}${early ? " during session startup" : ""}`, async () => {
    const fixture = cargoTreeFixture(escalate, early);
    const proc = start([rustCore], fixture.env);
    try {
      await waitFor(() => recordedPids(early ? fixture.launched : fixture.pids).length > 0, "Cargo startup");
      if (!early) proc.kill(signal);
      // gstack-timing-ok: cleanup has a 0.5 s TERM grace; the unchanged runner blocks on its 60 s child.
      expect(await Promise.race([proc.exited, Bun.sleep(2000).then(() => "still running")])).toBe(code);
      for (const pid of [...recordedPids(fixture.pids), ...recordedPids(fixture.launched)]) {
        expect(existsSync(`/proc/${pid}`), `Cargo/test process ${pid} remains`).toBe(false);
      }
    } finally {
      await stopCargoFixture(proc, fixture.pids, fixture.launched);
    }
  }, 10000);
}

// Value: protects=missing setsid rejects all selected work before the first runner with an install hint;
//   fails_when=setsid is unchecked or only checked after another runner; why_new=web selections did not require sessions;
//   seam=none; no_claim=util-linux installation on other hosts
test("Rust selection without setsid exits 2 naming util-linux before any runner", () => {
  const bin = join(tmp, "no-setsid");
  mkdirSync(bin);
  for (const tool of ["dirname", "nice", "find", "sort", "paste", "sleep"]) symlinkSync(Bun.which(tool) ?? tool, join(bin, tool));
  const { log, env } = stubEnv(0, { PATH: `${join(tmp, "bin")}:${bin}` });
  const done = Bun.spawnSync(["/bin/sh", script, unit, rustCore], { cwd: tmp, env });
  expect(done.exitCode, done.stderr.toString()).toBe(2);
  expect(done.stderr.toString()).toContain("setsid not found (util-linux)");
  expect(readCalls(log)).toEqual([]);
});

const rows: Row[] = [
  { name: "a unit file goes to bun test, with the path relative to web/", args: [unit], calls: [unitCall] },
  { name: "an e2e file goes to the default Playwright config", args: [e2e], calls: [e2eCall] },
  { name: "a shots file goes to Playwright with the shots config", args: [shots], calls: [shotsCall] },
  {
    name: "one call with all three kinds runs unit, e2e, then shots, whatever the argument order",
    args: [shots, e2e, unit],
    calls: [unitCall, e2eCall, shotsCall],
  },
  {
    name: "a guard file takes the shots twice with the guard config, each time into a dir of its own",
    args: [guard],
    calls: guardCalls,
  },
  {
    name: "all four kinds run unit, e2e, shots, then the guard, whatever the argument order",
    args: [guard, shots, e2e, unit],
    calls: [unitCall, e2eCall, shotsCall, ...guardCalls],
  },
  {
    name: "two files of each kind: one call per runner, the files in argument order",
    args: [unit, e2e, shots, "web/tests/unit/contrast.test.ts", "web/tests/e2e/browse.spec.ts", "web/tests/shots/b.spec.ts"],
    calls: [
      ["bun", "test ./tests/unit/format.test.ts ./tests/unit/contrast.test.ts"],
      ["bunx", "playwright test tests/e2e/find.spec.ts tests/e2e/browse.spec.ts"],
      ["bunx", "playwright test -c playwright.shots.config.ts tests/shots/readme.spec.ts tests/shots/b.spec.ts"],
    ],
  },
  {
    name: "a failing unit runner's status is the script's status and later runners do not start",
    args: [shots, e2e, unit],
    stubExit: 3,
    code: 3,
    calls: [unitCall],
  },
  { name: "a failing e2e runner stops the shots runner", args: [shots, e2e], stubExit: 3, code: 3, calls: [e2eCall] },
  { name: "a failing shots runner's status is the script's status", args: [shots], stubExit: 3, code: 3, calls: [shotsCall] },
  {
    name: "a guard run that fails taking the shots is the script's status, and the second does not start",
    args: [guard],
    stubExit: 3,
    code: 3,
    calls: [guardCall],
  },
  { name: "no arguments: usage on stderr, exit 2, nothing runs", args: [], code: 2, stderr: "usage:", calls: [] },
  {
    name: "a path no runner owns: exit 2, nothing runs",
    args: ["scripts/foo.sh"],
    code: 2,
    stderr: "no runner for scripts/foo.sh",
    calls: [],
  },
  {
    name: "a path no runner owns after a valid one: exit 2, and the valid one does not run either",
    args: [unit, "web/tests/integration/x.spec.ts"],
    code: 2,
    stderr: "no runner for web/tests/integration/x.spec.ts",
    calls: [],
  },
  {
    name: "a test file that does not exist, after a valid one: exit 2, nothing runs",
    args: [unit, "web/tests/unit/gone.test.ts"],
    code: 2,
    stderr: "no such test file web/tests/unit/gone.test.ts",
    calls: [],
  },
  // A runner given a helper finds no test in it and passes ("0 pass").
  ...["web/tests/unit/helper.ts", "web/tests/shots/home.ts"].map((f) => ({
    name: `a helper that exists, ${f}, after a valid file: exit 2, nothing runs`,
    args: [unit, f],
    code: 2,
    stderr: `not a test file ${f}`,
    calls: [],
  })),
  // The runner lists are split on spaces and glob-expanded, so a path with any of these would reach a runner changed.
  ...[" ", "*", "?", "[", ";", "$", "`"].map((c) => ({
    name: `a path with ${JSON.stringify(c)} after a valid one: exit 2, nothing runs`,
    args: [unit, `web/tests/e2e/a${c}b.spec.ts`],
    code: 2,
    stderr: `unsafe test path web/tests/e2e/a${c}b.spec.ts`,
    calls: [],
  })),
  // `echo` would turn the backslash escape into an ESC byte, which clears a terminal showing the log.
  {
    name: "a rejected path is printed as given, its backslash escapes left as text",
    args: [unit, "web/tests/e2e/a\\033[2Jb.spec.ts"],
    code: 2,
    stderr: "unsafe test path web/tests/e2e/a\\033[2Jb.spec.ts",
    calls: [],
  },
  {
    name: "a path that climbs out of web/tests: exit 2, nothing runs",
    args: ["web/tests/unit/../../../x.test.ts"],
    code: 2,
    stderr: "unsafe test path",
    calls: [],
  },
];

for (const row of rows) {
  test(row.name, () => {
    const got = run(row.args, row.stubExit);
    expect(got.code).toBe(row.code ?? 0);
    expect(got.calls.map((c) => [c.name, c.argv])).toEqual(row.calls);
    expect(got.calls.map((c) => c.cwd)).toEqual(row.calls.map(() => stubWeb));
    if (row.stderr) expect(got.stderr).toContain(row.stderr);
  });
}

// The copy's own web/pregate-shots, not merely some dir outside the real checkout: a script that shot into
// $root/docs/img would fail here (and its EXIT trap would have deleted that dir).
test("shots render into web/pregate-shots/shots, never docs/img, and the dir is gone afterwards", () => {
  const got = run([unit, shots]);
  expect(got.code).toBe(0);
  expect(got.calls.map((c) => c.shotsDir)).toEqual(["", join(shotsRoot, "shots")]);
  expect(existsSync(shotsRoot)).toBe(false);
});

test("the two guard runs shoot into pregate-shots/guard-a and guard-b, gone afterwards, with the port and without the lock", () => {
  const got = run([unit, shots, guard]);
  expect(got.code).toBe(0);
  expect(got.calls.map((c) => c.guardDir)).toEqual(["", "", join(shotsRoot, "guard-a"), join(shotsRoot, "guard-b")]);
  expect(existsSync(shotsRoot)).toBe(false);
  expect(got.calls.map((c) => c.port)).toEqual(["", PORT, PORT, PORT]);
  expect(got.calls.map((c) => c.fd9)).toEqual(["", "", "", ""]);
});

// Value: protects=the guard's verdict is the byte comparison of the two runs' shots, as refactor-guard's on GitHub;
//   fails_when=the diff is dropped or its status lost (two different takes pass), or it compares one dir with itself;
//   why_new=Playwright's own comparison skips pixels it takes for anti-aliasing, so the runs now only take shots;
//   seam=none; no_claim=real PNGs (the stub writes text)
test("the guard passes when both runs take the same shots, and fails naming the shot when they differ", () => {
  const same = stubEnv(0, { STUB_SHOT: "same" });
  const ok = Bun.spawnSync(["sh", script, guard], { cwd: tmp, env: same.env });
  expect(ok.exitCode, ok.stderr.toString()).toBe(0);
  const differ = stubEnv(0, { STUB_SHOT: "pid" });
  const bad = Bun.spawnSync(["sh", script, guard], { cwd: tmp, env: differ.env });
  expect(bad.exitCode).toBe(1);
  expect(bad.stderr.toString()).toMatch(/guard-a\/view\.png.*guard-b\/view\.png.*differ/);
  expect(bad.stderr.toString()).toContain("two runs of one build took different shots");
  expect(readCalls(differ.log)).toHaveLength(2);
});

// Value: protects=guard files run alone get the free port and no inherited lock, as e2e and shots files do (one block);
//   fails_when=$guard leaves the lock-and-port condition: the guard then builds web/dist unlocked and browses on 4174;
//   why_new=the other port checks pass a shots file beside the guard, and that file's own entry masks a missing one;
//   seam=none; no_claim=the lock itself (the wait test below uses e2e and shots files)
test("e2e, shots and guard get the free port the script picked, unit tests get none", () => {
  const got = run([shots, e2e, unit]);
  expect(got.code).toBe(0);
  expect(got.calls.map((c) => c.port)).toEqual(["", PORT, PORT]);
  expect(run([shots]).calls.map((c) => c.port)).toEqual([PORT]);
  expect(run([guard]).calls.map((c) => [c.port, c.fd9])).toEqual([
    [PORT, ""],
    [PORT, ""],
  ]);
});

// Only bunx is stubbed here, so `bun -e` is the real free-port pick.
test("the real free-port pick hands Playwright a port number", () => {
  const { log, env } = stubEnv(0, { PATH: `${join(tmp, "bunx-only")}:${dirname(process.execPath)}:${process.env.PATH}` });
  const done = Bun.spawnSync(["sh", script, e2e], { cwd: tmp, env });
  expect(done.exitCode).toBe(0);
  const port = readCalls(log)[0]?.port ?? "";
  expect(port).toMatch(/^[1-9][0-9]*$/);
  expect(Number(port)).toBeGreaterThanOrEqual(1024);
  expect(Number(port)).toBeLessThanOrEqual(65535);
});

// Value: protects=the script finds the repo from its own path whatever the caller's CDPATH; fails_when=`cd` follows a
//   CDPATH entry that holds scripts/ (root becomes two lines and every file "no such test file"); seam=none;
//   no_claim=shells other than sh
test("a CDPATH entry holding a scripts/ dir does not move the repo root", () => {
  mkdirSync(join(tmp, "cdpath/scripts"), { recursive: true });
  const { log, env } = stubEnv(0, { CDPATH: join(tmp, "cdpath") });
  const done = Bun.spawnSync(["sh", "scripts/test-files.sh", unit], { cwd: join(tmp, "repo"), env });
  expect(done.exitCode, done.stderr.toString()).toBe(0);
  expect(readCalls(log).map((c) => [c.name, c.argv, c.cwd])).toEqual([[...unitCall, stubWeb]]);
});

// Value: protects=without flock (util-linux) a run that needs the lock exits 2 naming it before anything runs, where it
//   used to say "waiting" and run unlocked; so does a flock that fails (status 2 and up); a unit run takes no lock;
//   fails_when=the `command -v flock` check goes or covers unit runs too, or any flock failure counts as "busy";
//   seam=PATH (the stubs and the tools they use; no flock, or one that exits 2); no_claim=the real flock's statuses
test("without flock, or with one that fails, an e2e run exits 2 naming it; a unit run still runs", () => {
  const bin = join(tmp, "no-flock");
  mkdirSync(bin);
  for (const tool of ["dirname", "find", "sort", "paste", "sleep"]) symlinkSync(Bun.which(tool) ?? tool, join(bin, tool));
  mkdirSync(join(tmp, "bad-flock"));
  writeFileSync(join(tmp, "bad-flock/flock"), "#!/bin/sh\nexit 2\n");
  chmodSync(join(tmp, "bad-flock/flock"), 0o755);
  const noFlock = `${join(tmp, "bin")}:${bin}`;
  for (const [path, args, code, calls, said] of [
    [noFlock, [e2e], 2, [], "flock not found"],
    [`${join(tmp, "bad-flock")}:${noFlock}`, [e2e], 2, [], "flock failed with status 2"],
    [noFlock, [unit], 0, [unitCall], ""],
  ] as const) {
    const { log, env } = stubEnv(0, { PATH: path });
    const done = Bun.spawnSync(["/bin/sh", script, ...args], { cwd: tmp, env });
    expect(done.exitCode, done.stderr.toString()).toBe(code);
    expect(readCalls(log).map((c) => [c.name, c.argv])).toEqual([...calls]);
    expect(done.stderr.toString()).toContain(said);
  }
});

// Value: protects=a heartbeat the wait cannot count in stops a run that needs the lock before any runner, naming it;
//   fails_when=the check goes (0 spins printing "still waiting", 0.5 and 08 break the sh arithmetic mid-wait) or comes
//   after the unit runner; seam=TEST_FILES_HEARTBEAT; no_claim=a heartbeat in a run that needs no lock
test("a heartbeat that is not whole seconds from 1 stops an e2e run before any runner", () => {
  for (const beat of ["0", "0.5", "08", "x"]) {
    const { log, env } = stubEnv(0, { TEST_FILES_HEARTBEAT: beat });
    const done = Bun.spawnSync(["sh", script, unit, e2e], { cwd: tmp, env });
    expect(done.exitCode, beat).toBe(2);
    expect(done.stderr.toString()).toContain(`TEST_FILES_HEARTBEAT is whole seconds from 1, not ${beat}`);
    expect(readCalls(log)).toEqual([]);
  }
});

// Both configs build their URLs and the server command from $E2E_PORT and never reuse a server already on that port
// (it belongs to someone else); unset or empty, they fall back to 4173 and, outside CI, reuse a running server.
// Four fresh Bun processes import Playwright here; allow for slow startup under concurrent CI load.
test("the Playwright configs serve and browse on $E2E_PORT", () => {
  const read = `const base = (await import("./playwright.config.ts")).default;
const shots = (await import("./playwright.shots.config.ts")).default;
const show = (c) => [c.use.baseURL, c.webServer.url, c.webServer.command, c.webServer.reuseExistingServer];
console.log(JSON.stringify([base, shots].map(show)));`;
  for (const [port, ci, want, reuse] of [
    ["41731", undefined, "41731", false],
    [undefined, undefined, "4173", true],
    ["", undefined, "4173", true],
    [undefined, "1", "4173", false],
  ] as const) {
    const env: Record<string, string | undefined> = { ...process.env, E2E_PORT: port, CI: ci };
    if (port === undefined) delete env.E2E_PORT;
    if (ci === undefined) delete env.CI;
    const done = Bun.spawnSync([process.execPath, "-e", read], { cwd: web, env });
    expect(done.exitCode, done.stderr.toString()).toBe(0);
    const config = [`http://127.0.0.1:${want}`, `http://127.0.0.1:${want}/`, `bun run build && PORT=${want} bun tests/e2e/serve.ts`, reuse];
    expect(JSON.parse(done.stdout.toString()), `E2E_PORT=${port} CI=${ci}`).toEqual([config, config]);
  }
}, 10_000);

// The guard serves the build in $GUARD_DIST, or builds web/dist first, on $E2E_PORT (else 4174) and never on a server
// already there. Its shots are stable: no retry, animations off, and partial raster off (with it on, one build repainted
// a form field's border one level apart on 3 of 6 runs). Its own comparison only draws the differences (the verdict
// compares the PNG files), as strictly as Playwright allows: no differing pixel, no colour tolerance.
test("the guard config serves the build it is given, shoots stably and compares as strictly as Playwright allows", () => {
  const read = `const c = (await import("./playwright.guard.config.ts")).default;
const { baseURL, launchOptions } = c.use;
const { url, command, reuseExistingServer } = c.webServer;
console.log(JSON.stringify([baseURL, url, command, reuseExistingServer, c.snapshotPathTemplate, c.testDir, c.retries,
  c.expect.toHaveScreenshot, launchOptions.args]));`;
  const strict = [0, { maxDiffPixels: 0, threshold: 0, animations: "disabled", caret: "hide" }, ["--disable-partial-raster"]];
  for (const [vars, want] of [
    [
      { E2E_PORT: "41732", GUARD_DIST: "/tmp/base/web/dist", GUARD_SHOTS: "/tmp/guard" },
      ["41732", "PORT=41732 bun tests/e2e/serve.ts", "/tmp/guard/{arg}{ext}"],
    ],
    [{}, ["4174", "bun run build && PORT=4174 bun tests/e2e/serve.ts", "guard-shots/{arg}{ext}"]],
  ] as const) {
    const env: Record<string, string | undefined> = { ...process.env, ...vars };
    for (const k of ["E2E_PORT", "GUARD_DIST", "GUARD_SHOTS"]) if (!(k in vars)) delete env[k];
    const done = Bun.spawnSync([process.execPath, "-e", read], { cwd: web, env });
    expect(done.exitCode, done.stderr.toString()).toBe(0);
    const [port, command, template] = want;
    const url = `http://127.0.0.1:${port}`;
    expect(JSON.parse(done.stdout.toString())).toEqual([url, `${url}/`, command, false, template, "tests/guard", ...strict]);
  }
});

// A copy of tests/e2e/serve.ts in tmp/serve/tests/e2e/, so its default build folder (dist/ two levels up) is
// tmp/serve/dist, never the real web/dist; tmp/other is another build.
function serveCopy() {
  mkdirSync(join(tmp, "serve/tests/e2e"), { recursive: true });
  copyFileSync(join(web, "tests/e2e/serve.ts"), join(tmp, "serve/tests/e2e/serve.ts"));
  mkdirSync(join(tmp, "serve/dist"));
  writeFileSync(join(tmp, "serve/dist/index.html"), "default build");
  mkdirSync(join(tmp, "other"));
  writeFileSync(join(tmp, "other/only.js"), "only in other");
}

// Starts the copy of serve.ts on a free port with `env` added and `stdin` ("ignore" is /dev/null); resolves once it
// says it is ready.
async function startServe<In extends "pipe" | "ignore">(env: Record<string, string>, stdin: In) {
  const probe = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response() });
  const port = probe.port;
  await probe.stop(true);
  const inherited: Record<string, string | undefined> = { ...process.env, PORT: String(port) };
  delete inherited.GUARD_DIST;
  const p = Bun.spawn([process.execPath, join(tmp, "serve/tests/e2e/serve.ts")], {
    cwd: join(tmp, "serve"),
    env: { ...inherited, ...env },
    stdin,
    stdout: "pipe",
    stderr: "pipe",
  });
  const said = await readUntil(p.stdout, "ready");
  if (!said.includes("ready")) throw new Error(`serve.ts did not start: ${await new Response(p.stderr).text()}`);
  return { p, port };
}

// Starts the copy of serve.ts with `env` added, answers one GET, stops it.
async function served(env: Record<string, string>, path: string) {
  const { p, port } = await startServe(env, "ignore");
  try {
    const res = await fetch(`http://127.0.0.1:${port}${path}`);
    return { status: res.status, csp: res.headers.get("content-security-policy") ?? "", body: await res.text() };
  } finally {
    p.kill();
    await p.exited;
  }
}

// Value: protects=serve.ts serves the build in $GUARD_DIST (absolute or relative), else the dist/ beside tests/; the guard's
//   base shots are taken from it; fails_when=$GUARD_DIST is ignored or loses its trailing slash, or an empty value beats
//   the default: pixel-diff then compares head with itself; why_new=the config test reads only the server command, no
//   test starts serve.ts with $GUARD_DIST set; seam=none; no_claim=Playwright driving it; a copy of serve.ts stands in
test("serve.ts serves the build in $GUARD_DIST, absolute or relative, else the dist/ beside tests/", async () => {
  const fallback = await served({}, "/");
  expect([fallback.status, fallback.body]).toEqual([200, "default build"]);
  expect(fallback.csp).toContain("default-src 'self'");
  expect((await served({}, "/only.js")).status).toBe(404);
  for (const dist of [join(tmp, "other"), "../other", "../other/"]) {
    const got = await served({ GUARD_DIST: dist }, "/only.js");
    expect([dist, got.status, got.body]).toEqual([dist, 200, "only in other"]);
    expect(got.csp).toBe(fallback.csp);
  }
  expect((await served({ GUARD_DIST: "" }, "/")).body).toBe("default build");
});

// Value: protects=a Playwright run killed outright (SIGKILL, the pre-gate's escalation) leaves no server behind: the
//   server, started detached, stops at the end of its stdin pipe; fails_when=serve.ts stops watching stdin, or watches
//   /dev/null too (started by hand in the background it would stop at once); seam=none; no_claim=Playwright's own pipe
//   (a socket pair, as Bun's here), a terminal on stdin
test("serve.ts stops when its stdin pipe closes, and stays up on /dev/null", async () => {
  const piped = await startServe({}, "pipe");
  try {
    piped.p.stdin.end();
    expect(await Promise.race([piped.p.exited, Bun.sleep(3000).then(() => "still up")])).toBe(0);
  } finally {
    // a server that missed the end of its input must not outlive the test
    piped.p.kill();
  }
  const { p, port } = await startServe({}, "ignore");
  try {
    expect(await Promise.race([p.exited, Bun.sleep(500).then(() => "still up")])).toBe("still up");
    expect((await fetch(`http://127.0.0.1:${port}/`)).status).toBe(200);
  } finally {
    p.kill();
    await p.exited;
  }
}, 10_000);

// The children of `pid` that run `name` (Linux /proc).
function children(pid: number, name: string) {
  return readdirSync("/proc").filter((d) => {
    try {
      const stat = readFileSync(`/proc/${d}/stat`, "utf8");
      const ppid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
      return ppid === pid && stat.slice(stat.indexOf("(") + 1, stat.lastIndexOf(")")) === name;
    } catch {
      return false;
    }
  });
}

// The process group of `pid` (Linux /proc).
function group(pid: number) {
  const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  return Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[2]);
}

// Two runs in one checkout share web/dist and test-results, so the second waits for the first, says so, and says so
// again every $TEST_FILES_HEARTBEAT seconds (a silence-based stall timeout must not take it for hung). While it blocks
// in `flock`, its runner cannot have started; both log to one file, so the order shows it started after the first one
// was released.
test("a second e2e run in the same checkout waits until the first one ends, and keeps saying so", async () => {
  const go = join(tmp, "go-first");
  const first = stubEnv(0, { STUB_GO: go });
  const p1 = start([e2e], first.env);
  const secondEnv: Record<string, string | undefined> = { ...first.env, TEST_FILES_HEARTBEAT: "1" };
  delete secondEnv.STUB_GO;
  let p2: ReturnType<typeof start> | undefined;
  try {
    await waitFor(() => readCalls(first.log).length > 0, "the first runner");
    const second = start([shots], secondEnv);
    p2 = second;
    const said = await readUntil(second.stderr, "still waiting");
    expect(said).toContain(`waiting for another e2e run in ${stubWeb}\n`);
    expect(said).toContain(`still waiting for another e2e run in ${stubWeb} (1 s)`);
    await waitFor(() => children(second.pid, "flock").length === 1, "the second run's flock");
    expect(readCalls(first.log)).toHaveLength(1);
    writeFileSync(go, "");
    expect(await p1.exited).toBe(0);
    expect(await second.exited).toBe(0);
    expect(readCalls(first.log).map((c) => [c.name, c.argv])).toEqual([e2eCall, ["released", ""], shotsCall]);
  } finally {
    // a failed expectation must not leave the first stub waiting and the second run blocked on its lock
    writeFileSync(go, "");
    p1.kill();
    p2?.kill();
  }
}, 20_000);

// Value: protects=a run queued behind another one's lock stops at once on a TERM (the pre-gate's stall stop) and takes
//   its flock with it, its runner never started; fails_when=the wait runs flock in the foreground (the trap then waits
//   for the heartbeat, 30 s by default) or leaves flock running; why_new=the signal tests above hit a running runner;
//   seam=none; no_claim=the pre-gate's own stall timer
test("a run waiting for the lock stops at once on a TERM, its flock gone and its runner never started", async () => {
  const go = join(tmp, "go-queued");
  const first = stubEnv(0, { STUB_GO: go });
  const p1 = start([e2e], first.env);
  const queuedEnv: Record<string, string | undefined> = { ...first.env, TEST_FILES_HEARTBEAT: "30" };
  delete queuedEnv.STUB_GO;
  let p2: ReturnType<typeof start> | undefined;
  try {
    await waitFor(() => readCalls(first.log).length > 0, "the first runner");
    const queued = start([shots], queuedEnv);
    p2 = queued;
    await readUntil(queued.stderr, "waiting");
    await waitFor(() => children(queued.pid, "flock").length === 1, "the queued run's flock");
    const flock = children(queued.pid, "flock")[0];
    const sent = Date.now();
    queued.kill("SIGTERM");
    expect(await queued.exited).toBe(143);
    // gstack-timing-ok: the regression waits out the 30 s heartbeat; 2 s is 15 times below it, and a trap and a reap take ms
    expect(Date.now() - sent).toBeLessThan(2000);
    await waitFor(() => !existsSync(`/proc/${flock}`), "the queued run's flock to end", 1000);
    writeFileSync(go, "");
    expect(await p1.exited).toBe(0);
    expect(readCalls(first.log).map((c) => [c.name, c.argv])).toEqual([e2eCall, ["released", ""]]);
  } finally {
    writeFileSync(go, "");
    p1.kill();
    p2?.kill();
  }
}, 20_000);

// A Playwright web server outlives a killed run (it is spawned detached); holding fd 9 it would block every later run.
test("the runners never inherit the lock", () => {
  const got = run([unit, e2e, shots]);
  expect(got.code).toBe(0);
  expect(got.calls.map((c) => c.fd9)).toEqual(["", "", ""]);
});

test("a failing shots runner leaves no pregate-shots either", () => {
  const got = run([shots], 3);
  expect(got.code).toBe(3);
  expect(got.calls[0]?.shotsDir).toBe(join(shotsRoot, "shots"));
  expect(existsSync(shotsRoot)).toBe(false);
});

// gstack-pregate stops a stalled lane with TERM; INT and HUP come from a terminal, and any of them from a manual kill.
// Sent to the script alone while its runner holds, it reaches the runner as TERM (the stub then tears down for
// 0.3 s); the script exits 128 + the signal number only once the runner is gone, and removes pregate-shots.
for (const [kind, file, call] of [
  ["e2e", e2e, e2eCall],
  ["shots", shots, shotsCall],
] as const) {
  for (const [signal, code] of [
    ["SIGTERM", 143],
    ["SIGINT", 130],
    ["SIGHUP", 129],
  ] as const) {
    test(`a ${signal} to the script alone while ${kind} run stops the runner, then exits ${code}`, async () => {
      const go = join(tmp, `go-${kind}-${signal}`);
      const { log, env } = stubEnv(0, { STUB_GO: go });
      const proc = start([file], env);
      try {
        await waitFor(() => readCalls(log).length > 0, "the runner");
        proc.kill(signal);
        expect(await proc.exited).toBe(code);
        expect(readCalls(log).map((c) => [c.name, c.argv])).toEqual([call, ["stopped", ""]]);
        expect(existsSync(shotsRoot)).toBe(false);
      } finally {
        writeFileSync(go, "");
        proc.kill();
      }
    }, 10_000);
  }
}

// Value: protects=on a signal to the whole group (a terminal's Ctrl-C, a group kill) the script keeps the lock until its
//   runner has stopped, so a queued run never starts beside a runner still tearing down; fails_when=a path has no trap,
//   or the trap exits without reaping the runner; why_new=the tests above signal the script alone; seam=setsid (the
//   script leads its own group); no_claim=the real Playwright's teardown
for (const signal of ["SIGTERM", "SIGINT"] as const) {
  test(`on a ${signal} to the whole group a queued run starts only after the runner has stopped`, async () => {
    const go = join(tmp, `go-group-${signal}`);
    const first = stubEnv(0, { STUB_GO: go });
    const p1 = start([e2e], first.env, ["setsid", "sh"]);
    const queuedEnv: Record<string, string | undefined> = { ...first.env };
    delete queuedEnv.STUB_GO;
    let p2: ReturnType<typeof start> | undefined;
    try {
      await waitFor(() => readCalls(first.log).length > 0, "the first runner");
      const queued = start([shots], queuedEnv);
      p2 = queued;
      expect(await readUntil(queued.stderr, "waiting")).toContain("waiting for another e2e run");
      // never signal a group the script does not lead: it would be this test runner's
      expect(group(p1.pid)).toBe(p1.pid);
      process.kill(-p1.pid, signal);
      expect(await p1.exited).toBe(signal === "SIGTERM" ? 143 : 130);
      expect(await queued.exited).toBe(0);
      expect(readCalls(first.log).map((c) => [c.name, c.argv])).toEqual([e2eCall, ["stopped", ""], shotsCall]);
    } finally {
      writeFileSync(go, "");
      p1.kill();
      p2?.kill();
    }
  }, 20_000);
}

// Value: protects=a run killed outright (SIGKILL skips the EXIT trap) leaves web/pregate-shots behind, and the next run
//   clears it under the lock before its runner starts, so stale shots never pass for fresh ones; fails_when=the clearing
//   goes, or moves after the runner starts; seam=none; no_claim=the real Playwright
test("a run killed outright leaves pregate-shots behind, and the next run clears it before its runner starts", async () => {
  const go = join(tmp, "go-killed");
  const killed = stubEnv(0, { STUB_GO: go });
  const proc = start([shots], killed.env);
  try {
    await waitFor(() => readCalls(killed.log).length > 0, "the runner");
    proc.kill("SIGKILL");
    await proc.exited;
    writeFileSync(go, "");
    await waitFor(() => readCalls(killed.log).some((c) => c.name === "released"), "the orphaned runner to end");
    expect(existsSync(join(shotsRoot, "shots"))).toBe(true);
    writeFileSync(join(shotsRoot, "shots/old.png"), "stale");
    mkdirSync(join(shotsRoot, "guard-a"));
    writeFileSync(join(shotsRoot, "guard-a/view.png"), "stale");
    const next = run([shots]);
    expect(next.code).toBe(0);
    expect(next.calls.map((c) => c.left)).toEqual(["pregate-shots/shots"]);
    expect(existsSync(shotsRoot)).toBe(false);
  } finally {
    writeFileSync(go, "");
    proc.kill();
  }
}, 10_000);

// The stubs only show that the script exports SHOTS_DIR; every shots spec must write each screenshot under it, or
// pre-gate shots overwrite docs/img. Comments aside, docs/img may appear only as the fallback for an unset SHOTS_DIR.
test("every shots spec writes each screenshot into $SHOTS_DIR and names docs/img only as its fallback", () => {
  const fallback = /^const out\s*=\s*process\.env\.SHOTS_DIR\s*\|\|\s*"\.\.\/docs\/img";$/m;
  const specs = readdirSync(join(web, "tests/shots")).filter((f) => f.endsWith(".spec.ts"));
  let total = 0;
  for (const spec of specs) {
    const src = readFileSync(join(web, "tests/shots", spec), "utf8").replace(/^\s*\/\/.*$/gm, "");
    const shotCalls = src.match(/\.screenshot\(/g) ?? [];
    total += shotCalls.length;
    expect(src.match(/\.screenshot\(\s*\{\s*path:\s*`\$\{out\}\/[^`/]+\.png`/g) ?? [], spec).toHaveLength(shotCalls.length);
    if (shotCalls.length > 0) expect(src, spec).toMatch(fallback);
    expect(src.replace(fallback, ""), spec).not.toContain("docs/img");
  }
  expect(total).toBeGreaterThan(0);
});

// What the e2e server builds and serves: no spec imports these, so only .gstack/pregate.json selects e2e for them.
const e2eInputs = [
  "web/src/**",
  "web/index.html",
  "web/favicon.svg",
  "web/fonts/**",
  "web/package.json",
  "web/bun.lock",
  "web/tsconfig.json",
  "web/playwright.config.ts",
  "web/tests/e2e/mock.ts",
  "web/tests/e2e/serve.ts",
];

// Value: protects=both install comparator inputs select their focused test through the public dependency mapping;
// fails_when=either mapping drops/misnames that test; why_new=existence alone allowed an unmapped script;
// seam=none (actual checked-in config); non-testcaller=pre-gate consumes this dependencies contract;
// no_claim=external selector implementation, selected test outcomes or CI execution.
// .gstack/pregate.json lists test paths literally (no globs), so a new e2e spec must be added to every list by hand.
// A list may name other tests besides (serve.ts also selects the test of its $GUARD_DIST).
test("every pregate dependency that selects e2e specs selects all of them, and every listed test exists", () => {
  const config = JSON.parse(readFileSync(join(repo, ".gstack/pregate.json"), "utf8")) as { dependencies: Record<string, string[]> };
  const specs = readdirSync(join(web, "tests/e2e"))
    .filter((f) => f.endsWith(".spec.ts"))
    .map((f) => `web/tests/e2e/${f}`)
    .sort();
  const e2e = (tests: string[]) => tests.filter((t) => t.startsWith("web/tests/e2e/")).sort();
  for (const [glob, tests] of Object.entries(config.dependencies)) {
    for (const t of tests) expect(existsSync(join(repo, t)), `${glob} lists ${t}`).toBe(true);
    if (tests.some((t) => t.startsWith("web/tests/e2e/"))) expect(e2e(tests), glob).toEqual(specs);
  }
  for (const input of e2eInputs) expect(e2e(config.dependencies[input] ?? []), input).toEqual(specs);
  for (const input of ["scripts/rust-diff-install.sh", "scripts/rust-diff/install-kept.sh"]) {
    expect(config.dependencies[input], input).toContain("web/tests/unit/rust-diff-install.test.ts");
  }
});
