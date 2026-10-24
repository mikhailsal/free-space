// scripts/test-files.sh, the pre-gate runner: each test file goes to its own runner in web/, a failing runner's
// status is the script's, shots and guard shots render into a temp dir, and bad input exits 2 before anything runs.
// Stub bun and bunx stand in for the real runners; they record argv, cwd, $SHOTS_DIR, $E2E_PORT, an open fd 9 and
// $GUARD_SHOTS.
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
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const repo = realpathSync(join(import.meta.dir, "../../.."));
const web = join(repo, "web");

// `bun -e` (the script's free-port pick) prints PORT. Any other call appends "name|cwd|argv|$SHOTS_DIR|$E2E_PORT|fd9|$GUARD_SHOTS"
// to $STUB_LOG (fd9 is "open" when the script's lock reached the runner); if $STUB_GO is set, waits until that file
// exists (10 s at most) and appends "released". With $STUB_SHOT and $GUARD_SHOTS set it takes a shot: view.png holds
// $STUB_SHOT, or the stub's pid if that is "pid". Exits with $STUB_EXIT (default 0).
const PORT = "41730";
const stub = `#!/bin/sh
if [ "$1" = -e ]; then echo ${PORT}; exit 0; fi
fd9=; [ ! -e /proc/self/fd/9 ] || fd9=open
printf '%s|%s|%s|%s|%s|%s|%s\\n' "\${0##*/}" "$(pwd -P)" "$*" "\${SHOTS_DIR:-}" "\${E2E_PORT:-}" "$fd9" "\${GUARD_SHOTS:-}" >> "$STUB_LOG"
if [ -n "\${STUB_SHOT:-}" ] && [ -n "\${GUARD_SHOTS:-}" ]; then
  shot=$STUB_SHOT; [ "$shot" != pid ] || shot=$$
  printf '%s' "$shot" > "$GUARD_SHOTS/view.png"
fi
n=0
[ -z "\${STUB_GO:-}" ] || { while [ ! -e "$STUB_GO" ] && [ $n -lt 500 ]; do sleep 0.02; n=$((n + 1)); done; echo released >> "$STUB_LOG"; }
exit "\${STUB_EXIT:-0}"
`;

let tmp = "";
let runs = 0;
// A copy of the script in tmp/repo, so it runs in tmp/repo/web and its lock on web/ never waits for a real e2e run in
// this checkout. The test files it is given exist there, empty.
let script = "";
let stubWeb = "";

beforeAll(() => {
  tmp = realpathSync(mkdtempSync(join(tmpdir(), "test-files-")));
  script = join(tmp, "repo/scripts/test-files.sh");
  stubWeb = join(tmp, "repo/web");
  mkdirSync(join(tmp, "repo/scripts"), { recursive: true });
  mkdirSync(stubWeb);
  copyFileSync(join(repo, "scripts/test-files.sh"), script);
  for (const f of testFiles) {
    mkdirSync(dirname(join(tmp, "repo", f)), { recursive: true });
    writeFileSync(join(tmp, "repo", f), "");
  }
  mkdirSync(join(tmp, "bin"));
  mkdirSync(join(tmp, "bunx-only"));
  for (const path of ["bin/bun", "bin/bunx", "bunx-only/bunx"]) {
    writeFileSync(join(tmp, path), stub);
    chmodSync(join(tmp, path), 0o755);
  }
});

afterAll(() => {
  rmSync(tmp, { recursive: true, force: true });
});

// The environment of one script run: the stubs first on PATH, a fresh call log, and no inherited $SHOTS_DIR, $E2E_PORT
// or $GUARD_SHOTS.
function stubEnv(stubExit = 0, extra: Record<string, string> = {}) {
  const log = join(tmp, `${runs++}.log`);
  const env: Record<string, string | undefined> = {
    ...process.env,
    PATH: `${join(tmp, "bin")}:${process.env.PATH}`,
    STUB_LOG: log,
    STUB_EXIT: String(stubExit),
    ...extra,
  };
  delete env.SHOTS_DIR;
  delete env.E2E_PORT;
  delete env.GUARD_SHOTS;
  return { log, env };
}

// The stub calls in order.
function readCalls(log: string) {
  const lines = existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(Boolean) : [];
  return lines.map((l) => {
    const [name, cwd, argv, shotsDir, port, fd9, guardDir] = l.split("|") as [string, string, string, string, string, string, string];
    return { name, cwd, argv, shotsDir, port, fd9, guardDir };
  });
}

// Runs the script from outside the repo; returns its status, stderr and the stub calls.
function run(args: string[], stubExit = 0) {
  const { log, env } = stubEnv(stubExit);
  const done = Bun.spawnSync(["sh", script, ...args], { cwd: tmp, env });
  return { code: done.exitCode, stderr: done.stderr.toString(), calls: readCalls(log) };
}

const testFiles = [
  "web/tests/unit/format.test.ts",
  "web/tests/unit/contrast.test.ts",
  "web/tests/e2e/find.spec.ts",
  "web/tests/e2e/browse.spec.ts",
  "web/tests/shots/readme.spec.ts",
  "web/tests/shots/b.spec.ts",
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
  // The runner lists are split on spaces and glob-expanded, so a path with any of these would reach a runner changed.
  ...[" ", "*", "?", "[", ";", "$", "`"].map((c) => ({
    name: `a path with ${JSON.stringify(c)} after a valid one: exit 2, nothing runs`,
    args: [unit, `web/tests/e2e/a${c}b.spec.ts`],
    code: 2,
    stderr: `unsafe test path web/tests/e2e/a${c}b.spec.ts`,
    calls: [],
  })),
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

test("shots render into a fresh temp dir, never docs/img, and the dir is gone afterwards", () => {
  const got = run([unit, shots]);
  expect(got.code).toBe(0);
  expect(got.calls.map((c) => c.shotsDir)).toEqual(["", expect.stringMatching(/^\//)]);
  const dir = got.calls[1]?.shotsDir ?? "";
  expect(dir.startsWith(repo)).toBe(false);
  expect(existsSync(dir)).toBe(false);
});

test("the two guard runs shoot into two dirs of one fresh temp dir, gone afterwards, with the port and without the lock", () => {
  const got = run([unit, shots, guard]);
  expect(got.code).toBe(0);
  const [first, second] = got.calls.slice(2);
  expect(first?.guardDir).toMatch(/^\/.*\/guard-a$/);
  expect(second?.guardDir).toBe(`${dirname(first?.guardDir ?? "")}/guard-b`);
  expect(got.calls.slice(0, 2).map((c) => c.guardDir)).toEqual(["", ""]);
  expect(first?.guardDir.startsWith(repo)).toBe(false);
  expect(existsSync(dirname(first?.guardDir ?? ""))).toBe(false);
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

// Both configs build their URLs and the server command from $E2E_PORT and never reuse a server already on that port
// (it belongs to someone else); unset or empty, they fall back to 4173 and, outside CI, reuse a running server.
test("the Playwright configs serve and browse on $E2E_PORT", () => {
  const read = `const base = (await import("./playwright.config.ts")).default;
const shots = (await import("./playwright.shots.config.ts")).default;
const show = (c) => [c.use.baseURL, c.webServer.url, c.webServer.command, c.webServer.reuseExistingServer];
console.log(JSON.stringify([base, shots].map(show)));`;
  for (const [port, want, reuse] of [
    ["41731", "41731", false],
    [undefined, "4173", true],
    ["", "4173", true],
  ] as const) {
    const env: Record<string, string | undefined> = { ...process.env, E2E_PORT: port };
    if (port === undefined) delete env.E2E_PORT;
    delete env.CI;
    const done = Bun.spawnSync([process.execPath, "-e", read], { cwd: web, env });
    expect(done.exitCode, done.stderr.toString()).toBe(0);
    const config = [`http://127.0.0.1:${want}`, `http://127.0.0.1:${want}/`, `bun run build && PORT=${want} bun tests/e2e/serve.ts`, reuse];
    expect(JSON.parse(done.stdout.toString()), `E2E_PORT=${port}`).toEqual([config, config]);
  }
});

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

// A copy of tests/e2e/serve.ts sits in tmp/serve/tests/e2e/, so its default build folder (dist/ two levels up) is
// tmp/serve/dist, never the real web/dist. Starts it on a free port with `env` added, answers one GET, stops it.
async function served(env: Record<string, string>, path: string) {
  const probe = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => new Response() });
  const port = probe.port;
  await probe.stop(true);
  const inherited: Record<string, string | undefined> = { ...process.env, PORT: String(port) };
  delete inherited.GUARD_DIST;
  const p = Bun.spawn([process.execPath, join(tmp, "serve/tests/e2e/serve.ts")], {
    cwd: join(tmp, "serve"),
    env: { ...inherited, ...env },
    stdout: "pipe",
    stderr: "pipe",
  });
  try {
    const out = p.stdout.getReader();
    let said = "";
    while (!said.includes("ready")) {
      const chunk = await out.read();
      if (chunk.done) throw new Error(`serve.ts did not start: ${await new Response(p.stderr).text()}`);
      said += new TextDecoder().decode(chunk.value);
    }
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
  mkdirSync(join(tmp, "serve/tests/e2e"), { recursive: true });
  copyFileSync(join(web, "tests/e2e/serve.ts"), join(tmp, "serve/tests/e2e/serve.ts"));
  mkdirSync(join(tmp, "serve/dist"));
  writeFileSync(join(tmp, "serve/dist/index.html"), "default build");
  mkdirSync(join(tmp, "other"));
  writeFileSync(join(tmp, "other/only.js"), "only in other");

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

// Two runs in one checkout share web/dist and test-results, so the second waits for the first and says so. While it
// blocks in `flock`, its runner cannot have started; both log to one file, so the order shows it started after the
// first one was released.
test("a second e2e run in the same checkout waits until the first one ends", async () => {
  const go = join(tmp, "go-first");
  const first = stubEnv(0, { STUB_GO: go });
  const p1 = Bun.spawn(["sh", script, e2e], { cwd: tmp, env: first.env, stderr: "pipe" });
  const secondEnv = { ...first.env };
  delete secondEnv.STUB_GO;
  let p2: ReturnType<typeof Bun.spawn> | undefined;
  try {
    for (let i = 0; i < 250 && readCalls(first.log).length === 0; i++) await Bun.sleep(20);
    const second = Bun.spawn(["sh", script, shots], { cwd: tmp, env: secondEnv, stderr: "pipe" });
    p2 = second;
    const stderr = second.stderr.getReader();
    let said = "";
    while (!said.includes("waiting")) {
      const chunk = await stderr.read();
      if (chunk.done) break;
      said += new TextDecoder().decode(chunk.value);
    }
    expect(said).toContain(`waiting for another e2e run in ${stubWeb}`);
    for (let i = 0; i < 250 && children(second.pid, "flock").length === 0; i++) await Bun.sleep(20);
    expect(children(second.pid, "flock")).toHaveLength(1);
    expect(readCalls(first.log)).toHaveLength(1);
    writeFileSync(go, "");
    expect(await p1.exited).toBe(0);
    expect(await second.exited).toBe(0);
    expect(readCalls(first.log).map((c) => c.argv ?? c.name)).toEqual([e2eCall[1], "released", shotsCall[1]]);
  } finally {
    // a failed expectation must not leave the first stub waiting and the second run blocked on its lock
    writeFileSync(go, "");
    p1.kill();
    p2?.kill();
  }
});

// A Playwright web server outlives a killed run (it is spawned detached); holding fd 9 it would block every later run.
test("the runners never inherit the lock", () => {
  const got = run([unit, e2e, shots]);
  expect(got.code).toBe(0);
  expect(got.calls.map((c) => c.fd9)).toEqual(["", "", ""]);
});

test("a failing shots runner leaves no temp dir either", () => {
  const got = run([shots], 3);
  expect(got.code).toBe(3);
  const dir = got.calls[0]?.shotsDir ?? "";
  expect(dir).toMatch(/^\//);
  expect(existsSync(dir)).toBe(false);
});

// gstack-pregate stops a stalled lane with TERM; INT and HUP come from a terminal. The stub holds until the signal
// is sent, so it lands while shots render, and the script exits 128 + the signal number.
for (const [signal, code] of [
  ["SIGTERM", 143],
  ["SIGINT", 130],
  ["SIGHUP", 129],
] as const) {
  test(`a ${signal} while shots render exits ${code} and removes the temp dir`, async () => {
    const go = join(tmp, `go-${signal}`);
    const { log, env } = stubEnv(0, { STUB_GO: go });
    const proc = Bun.spawn(["sh", script, shots], { cwd: tmp, env, stderr: "pipe" });
    for (let i = 0; i < 250 && readCalls(log).length === 0; i++) await Bun.sleep(20);
    proc.kill(signal);
    writeFileSync(go, "");
    expect(await proc.exited).toBe(code);
    const dir = readCalls(log)[0]?.shotsDir ?? "";
    expect(dir).toMatch(/^\//);
    expect(existsSync(dir)).toBe(false);
  });
}

// The stubs only show that the script exports SHOTS_DIR; the spec must write every screenshot under it, or pre-gate
// shots overwrite docs/img.
test("readme.spec.ts writes every screenshot into $SHOTS_DIR", () => {
  const src = readFileSync(join(web, "tests/shots/readme.spec.ts"), "utf8");
  expect(src).toMatch(/^const out = process\.env\.SHOTS_DIR \|\| "\.\.\/docs\/img";$/m);
  const shotCalls = src.match(/\.screenshot\(/g) ?? [];
  expect(shotCalls.length).toBeGreaterThan(0);
  expect(src.match(/\.screenshot\(\{ path: `\$\{out\}\/[^`/]+\.png`/g) ?? []).toHaveLength(shotCalls.length);
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
});
