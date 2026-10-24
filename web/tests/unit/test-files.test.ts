// scripts/test-files.sh, the pre-gate runner: each test file goes to its own runner in web/, a failing runner's
// status is the script's, shots render into a temp dir, and bad input exits 2 before anything runs.
// Stub bun and bunx stand in for the real runners; they record argv, cwd, $SHOTS_DIR, $E2E_PORT and an open fd 9.
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

// `bun -e` (the script's free-port pick) prints PORT. Any other call appends "name|cwd|argv|$SHOTS_DIR|$E2E_PORT|fd9"
// to $STUB_LOG (fd9 is "open" when the script's lock reached the runner); if $STUB_GO is set, waits until that file
// exists (10 s at most) and appends "released". Exits with $STUB_EXIT (default 0).
const PORT = "41730";
const stub = `#!/bin/sh
if [ "$1" = -e ]; then echo ${PORT}; exit 0; fi
fd9=; [ ! -e /proc/self/fd/9 ] || fd9=open
printf '%s|%s|%s|%s|%s|%s\\n' "\${0##*/}" "$(pwd -P)" "$*" "\${SHOTS_DIR:-}" "\${E2E_PORT:-}" "$fd9" >> "$STUB_LOG"
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

// The environment of one script run: the stubs first on PATH, a fresh call log, and no inherited $SHOTS_DIR or $E2E_PORT.
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
  return { log, env };
}

// The stub calls in order.
function readCalls(log: string) {
  const lines = existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(Boolean) : [];
  return lines.map((l) => {
    const [name, cwd, argv, shotsDir, port, fd9] = l.split("|") as [string, string, string, string, string, string];
    return { name, cwd, argv, shotsDir, port, fd9 };
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
];
const unit = "web/tests/unit/format.test.ts";
const e2e = "web/tests/e2e/find.spec.ts";
const shots = "web/tests/shots/readme.spec.ts";
const unitCall = ["bun", "test ./tests/unit/format.test.ts"];
const e2eCall = ["bunx", "playwright test tests/e2e/find.spec.ts"];
const shotsCall = ["bunx", "playwright test -c playwright.shots.config.ts tests/shots/readme.spec.ts"];

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

test("e2e and shots get the free port the script picked, unit tests get none", () => {
  const got = run([shots, e2e, unit]);
  expect(got.code).toBe(0);
  expect(got.calls.map((c) => c.port)).toEqual(["", PORT, PORT]);
  expect(run([shots]).calls.map((c) => c.port)).toEqual([PORT]);
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
  "web/tests/e2e/serve.ts",
];

// .gstack/pregate.json lists test paths literally (no globs), so a new e2e spec must be added to every list by hand.
test("every pregate dependency that selects e2e specs selects all of them, and every listed test exists", () => {
  const config = JSON.parse(readFileSync(join(repo, ".gstack/pregate.json"), "utf8")) as { dependencies: Record<string, string[]> };
  const specs = readdirSync(join(web, "tests/e2e"))
    .filter((f) => f.endsWith(".spec.ts"))
    .map((f) => `web/tests/e2e/${f}`)
    .sort();
  for (const [glob, tests] of Object.entries(config.dependencies)) {
    for (const t of tests) expect(existsSync(join(repo, t)), `${glob} lists ${t}`).toBe(true);
    if (tests.some((t) => t.startsWith("web/tests/e2e/"))) expect([...tests].sort(), glob).toEqual(specs);
  }
  for (const input of e2eInputs) expect([...(config.dependencies[input] ?? [])].sort(), input).toEqual(specs);
});
