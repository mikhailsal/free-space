// How scripts/rust-diff.sh starts and stops, on stub builds (rust-diff-stub.ts): bad input, a missing tool, missing
// time zones and root exit 2 before anything runs; a base that cannot make the fixture snapshot stops the job at once
// and shows why; a command of the harness's own that fails stops it with exit 5; the work dir is absolute from a
// relative TMPDIR too; a HUP, INT or TERM ends the job with 128 + the signal number and removes its work dir, a Ctrl-C
// ends the call in progress too, and a server or a gui still running goes with the job.
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
import { gone, guard, HOLD, STUB_ENV, scanFails, script, stub, until, writeStub } from "./rust-diff-stub";

let tmp = "";

// The parent of a process (Linux /proc).
const parent = (pid: number) => {
  const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  return Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
};

beforeAll(() => {
  tmp = realpathSync(mkdtempSync(join(tmpdir(), "rust-diff-process-")));
  writeStub(tmp, "base", stub());
  writeStub(tmp, "noscan1", scanFails(1));
  writeStub(tmp, "noscan2", scanFails(2));
  writeFileSync(join(tmp, "not-executable"), stub());
});

afterAll(() => {
  Bun.spawnSync(["chmod", "-R", "u+rwx", tmp]);
  rmSync(tmp, { recursive: true, force: true });
});

// Value: protects=a base that cannot make the fixture snapshot stops the job at once and says why: exit 1, the base's own
//   output, then the message naming the scan (the first, or the second after the tree changed), and nothing answered (no
//   base/ or head/ in the out dir); fails_when=the `if !` around the scan is dropped (set -e ends the job with the scan's
//   status 7 and no message), the scan's output is not shown, the step number is not the scan's, or the work dir stays
//   in TMPDIR; why_new=every base so far scanned fine; seam=none; no_claim=what the real scan prints when it fails (a
//   stub says it is on fire)
test("a base that cannot make the fixture snapshot stops the job at once and shows why", async () => {
  for (const step of [1, 2]) {
    const r = await guard(tmp, `noscan${step}`, "base");
    expect(r.code, `scan ${step}`).toBe(1);
    expect(r.stderr).toMatch(
      new RegExp(`^disk on fire in scan ${step}\\n.*: base could not make the fixture snapshot \\(scan ${step}\\)\\n$`),
    );
    expect(existsSync(join(r.out, "head"))).toBe(false);
    expect(readdirSync(r.out)).toEqual([]);
    expect(r.left, "the work dir, removed").toEqual([]);
  }
}, 30_000);

// Value: protects=no arguments, a build that is not an executable file, a width of the trees outside 1 to 9 or a bound
//   on a call that is not whole seconds from 1 exit 2 before anything runs (no out dir, no work dir); fails_when=a
//   check goes (a width of 0 then makes no wide part, and the job dies midway; a bound of 0 lets a hang run forever, a
//   word fails every call) or moves past the work dir; why_new=RUST_DIFF_FAN was taken as given (red team of A0's
//   /ship), and RUST_DIFF_TIMEOUT too (testing review of A0b); seam=none; no_claim=none
test("bad input exits 2 before anything runs", async () => {
  const none = Bun.spawnSync([script], { stderr: "pipe" });
  expect(none.exitCode).toBe(2);
  expect(none.stderr.toString()).toContain("usage:");
  for (const bad of ["not-executable", "missing"]) {
    const r = await guard(tmp, "base", bad);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain(`not an executable file: ${join(tmp, bad)}`);
    expect(existsSync(r.out)).toBe(false);
    expect(r.left, "no work dir").toEqual([]);
  }
  for (const fan of ["0", "10", "x"]) {
    const r = await guard(tmp, "base", "base", { RUST_DIFF_FAN: fan });
    expect(r.code, fan).toBe(2);
    expect(r.stderr).toContain(`RUST_DIFF_FAN must be 1 to 9, not ${fan}`);
    expect(existsSync(r.out)).toBe(false);
    expect(r.left, "no work dir").toEqual([]);
  }
  for (const bound of ["0", "1.5", "x"]) {
    const r = await guard(tmp, "base", "base", { RUST_DIFF_TIMEOUT: bound });
    expect(r.code, bound).toBe(2);
    expect(r.stderr).toContain(`RUST_DIFF_TIMEOUT must be whole seconds from 1, not ${bound}`);
    expect(existsSync(r.out)).toBe(false);
    expect(r.left, "no work dir").toEqual([]);
  }
});

// Value: protects=the work dir is absolute when TMPDIR is relative: the CLI gets absolute folders (the cwd questions run
//   from inside the tree, where a relative one would point elsewhere); fails_when=the realpath of the work dir goes;
//   why_new=QA of A0's /ship ran the job with a relative TMPDIR and it died midway; seam=TMPDIR; no_claim=none
test("from a relative TMPDIR the work dir is absolute", async () => {
  const dir = join(tmp, "relative");
  mkdirSync(join(dir, "scratch"), { recursive: true });
  // a base that records the folders it is given, and fails its first scan: the job ends there
  writeStub(dir, "records", `#!/bin/sh\necho "$FREESPACE_SNAPSHOT_DIR $HOME" > "$STUB_SEEN"\nexit 7\n`);
  const seen = join(dir, "seen");
  const p = Bun.spawn([script, "records", "records", "out"], {
    cwd: dir,
    env: { ...process.env, ...STUB_ENV, TMPDIR: "scratch", STUB_SEEN: seen },
    stdout: "ignore",
    stderr: "ignore",
  });
  expect(await p.exited).toBe(1);
  const [snapshots, home] = readFileSync(seen, "utf8").trim().split(" ");
  for (const path of [snapshots, home]) expect(path).toStartWith(`${join(dir, "scratch")}/`);
  expect(readdirSync(join(dir, "scratch"))).toEqual([]);
});

// Value: protects=a command of the harness's own that fails (here a copy of the fixture snapshot that is gone) stops the
//   job with exit 5 and names the command and its line, apart from exit 1 (head differs), and the work dir goes;
//   fails_when=the ERR trap goes (set -e ends the job with the command's own status, 1, the status of a difference), is
//   not inherited by functions (set -E), or the EXIT trap no longer removes the work dir after it; why_new=a harness
//   error read as a difference (red team of A0's /ship); seam=none; no_claim=which commands of the harness can fail on
//   a runner
test("a command of the harness that fails stops the job with exit 5, naming it", async () => {
  // a base that removes the fixture snapshot the harness copies for each run, at its first question
  const gone = 'case "$1 $dir" in "status "*/snap) rm -rf "$' + "{dir%/snap}/pristine" + '" ;; esac';
  writeStub(tmp, "unpristine", stub({ early: gone }));
  const r = await guard(tmp, "unpristine", "base");
  expect(r.code).toBe(5);
  expect(r.stderr).toMatch(/: harness error: rust-diff\.sh line \d+: cp -a "\$work\/pristine" "\$work\/refreshed" exited 1\n/);
  expect(existsSync(join(r.out, "head"))).toBe(false);
  expect(r.left, "the work dir, removed").toEqual([]);
}, 180_000);

// Value: protects=a diff that cannot compare (exit 2, as on a file it cannot read) is a harness error, exit 5 naming what
//   it compared, both for two runs' answers and for the tree's listings, and not a difference (exit 1) or an unreliable
//   base (exit 3); fails_when=same() or tree_check takes any status of diff for a difference again; why_new=both run
//   diff under `if !`, where the ERR trap is off, so its exit 2 read as exit 3 or 1 (state-machine review of A0b);
//   seam=PATH (a diff that fails on one comparison); no_claim=why a real diff fails
test("a diff that cannot compare is a harness error, exit 5", async () => {
  const real = Bun.which("diff") ?? "diff";
  for (const [name, args, said] of [
    ["listing", '*"--label before --label after"*', "the diff of the tree's listing exited 2"],
    ["answers", '*"/fixture.txt "*', "the diff of base/fixture.txt and base2/fixture.txt exited 2"],
  ]) {
    const dir = join(tmp, `no-diff-${name}`);
    mkdirSync(dir, { recursive: true });
    writeStub(dir, "diff", `#!/bin/sh\ncase "$*" in ${args}) echo "diff: cannot read" >&2; exit 2 ;; esac\nexec ${real} "$@"\n`);
    const r = await guard(tmp, "base", "base", { PATH: `${dir}:${process.env.PATH}` });
    expect(r.code, name).toBe(5);
    expect(r.stderr, name).toContain(`: harness error: ${said}\n`);
    expect(existsSync(join(r.out, "head")), name).toBe(false);
    expect(r.left, `${name}: the work dir, removed`).toEqual([]);
  }
}, 240_000);

// Value: protects=a missing tool, any of the six, missing time zones, or a run as root stops the job before anything
//   runs: exit 2, no out dir; fails_when=a tool leaves the check (the job then dies midway at a command not found), the
//   zone check goes (without tzdata the C library takes UTC and TZ_QUERIES test nothing), or the root check goes
//   (install-timer would install the build on this host); why_new=nothing ran these checks, then only one tool was left
//   out (testing review of A0), and no zone was checked (red team of A0); seam=PATH, TZDIR; no_claim=a real run as root
//   (a stub `id` says 0) or a host without tzdata (an empty TZDIR stands in)
test("a missing tool, missing time zones or a run as root stop the job before anything runs", () => {
  const tools = ["jq", "curl", "script", "flock", "perl", "sqlite3"];
  // each case links the other five: one missing here would make the case fail for the wrong tool
  for (const tool of tools) expect(Bun.which(tool), `${tool} on this host`).not.toBeNull();
  const cases: [Record<string, string>, string][] = tools.map((tool) => {
    const dir = join(tmp, `path-without-${tool}`);
    mkdirSync(dir, { recursive: true });
    for (const other of tools.filter((t) => t !== tool)) symlinkSync(Bun.which(other) ?? other, join(dir, other));
    return [{ PATH: dir }, `${tool} is required`];
  });
  const root = join(tmp, "path-root");
  mkdirSync(root, { recursive: true });
  writeStub(root, "id", "#!/bin/sh\necho 0\n");
  const zones = join(tmp, "no-zones");
  mkdirSync(zones, { recursive: true });
  cases.push([{ TZDIR: zones }, "tzdata is required"], [{ PATH: `${root}:${process.env.PATH}` }, "run it as a user, not root"]);
  for (const [extra, said] of cases) {
    const out = join(tmp, `checked-${said.split(" ")[0]}`);
    const r = Bun.spawnSync([Bun.which("bash") ?? "bash", script, join(tmp, "base"), join(tmp, "base"), out], {
      env: { ...process.env, ...extra },
      stderr: "pipe",
    });
    expect(r.exitCode, said).toBe(2);
    expect(r.stderr.toString()).toContain(said);
    expect(existsSync(out)).toBe(false);
  }
});

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
    process.kill(-proc.pid, "SIGINT");
    expect(await proc.exited).toBe(130);
    // the call died of the signal: one the signal misses holds its 10 s, the job waits for it, and it marks its end
    expect(existsSync(`${held}.end`)).toBe(false);
    expect(readdirSync(scratch)).toEqual([]);
  } finally {
    writeFileSync(go, "");
    proc.kill();
    Bun.spawnSync(["chmod", "-R", "u+rwx", scratch]);
  }
}, 30_000);

// Value: protects=a TERM while serve waits to start ends the job at once, and the server goes with it: the EXIT trap
//   TERMs the serve call, and timeout passes that on to the build; fails_when=the trap's kill of $serving is dropped,
//   serve_answers stops setting it (the server then outlives the job, holding a port and a token), or the address wait
//   runs in a $(...) again (bash then runs the trap when the wait is over, a minute on); why_new=no test ended the job
//   while serve ran (testing and state-machine reviews of A0), then a held server that ended by itself within the wait
//   passed with $serving unset (QA of A0); seam=none; no_claim=GitHub's job cancellation, and a TERM during any other
//   call (bash runs the trap once that call ends)
test("a TERM while serve waits to start ends the job at once and stops the server too", async () => {
  const dir = join(tmp, "serve-signal");
  mkdirSync(dir, { recursive: true });
  const [held, served] = [join(dir, "held"), join(dir, "serve.pid")];
  // a server that never prints its address, and outlasts the job's wait for it (serve --port 0 only: `serve --help`,
  // asked before, is a call the job waits out)
  const serving = '[ "$*" != "serve --port 0" ] || { echo $$ > "$STUB_SERVE_PID"; : > "$STUB_HELD"; exec sleep 300; }';
  writeStub(dir, "serving", stub({ early: serving }));
  const env = { ...process.env, ...STUB_ENV, STUB_HELD: held, STUB_SERVE_PID: served };
  const proc = Bun.spawn([script, join(dir, "serving"), join(dir, "serving"), join(dir, "out")], {
    env,
    stdout: "ignore",
    stderr: "ignore",
  });
  let pid = 0;
  try {
    expect(await until(() => existsSync(held), 120_000), "the script reached serve").toBe(true);
    pid = Number(readFileSync(served, "utf8"));
    expect(gone(pid), "the server runs").toBe(false);
    proc.kill("SIGTERM");
    // the wait for the address is a minute
    expect(await until(() => proc.exitCode !== null, 20_000), "the job ended at the TERM").toBe(true);
    expect(await proc.exited).toBe(143);
    expect(await until(() => gone(pid)), "the server stopped with the job").toBe(true);
  } finally {
    proc.kill();
    if (pid && !gone(pid)) process.kill(pid, "SIGKILL");
  }
}, 180_000);

// Value: protects=a TERM while gui waits to start ends the job at once, and gui goes with it, so does the script(1) it
//   runs under: the EXIT trap TERMs script(1) ($serving), which passes the TERM on to the build, through timeout;
//   fails_when=gui_run stops setting $serving, the trap TERMs a process that does not pass the TERM on (the gui then
//   outlives the job with its port and its token), or the address wait runs in a $(...) again; why_new=only serve was
//   ended with the job (state-machine review of A0); seam=none; no_claim=GitHub's job cancellation
test("a TERM while gui waits to start ends the job at once and stops gui and its script(1) too", async () => {
  const dir = join(tmp, "gui-signal");
  mkdirSync(dir, { recursive: true });
  const [held, started] = [join(dir, "held"), join(dir, "gui.pid")];
  // a gui that never prints its address, and outlasts the job's wait for it (`gui --port 0` only: the bare `gui` the job
  // asks before must still refuse)
  const holding = '[ "$1 $2" != "gui --port" ] || { echo $$ > "$STUB_GUI_PID"; : > "$STUB_HELD"; exec sleep 300; }';
  writeStub(dir, "holding", stub({ early: holding }));
  const env = { ...process.env, ...STUB_ENV, STUB_HELD: held, STUB_GUI_PID: started };
  const proc = Bun.spawn([script, join(dir, "holding"), join(dir, "holding"), join(dir, "out")], {
    env,
    stdout: "ignore",
    stderr: "ignore",
  });
  let [pid, terminal] = [0, 0];
  try {
    expect(await until(() => existsSync(held), 120_000), "the script reached gui").toBe(true);
    pid = Number(readFileSync(started, "utf8"));
    // gui runs under timeout, under script(1)
    terminal = parent(parent(pid));
    expect(readFileSync(`/proc/${terminal}/comm`, "utf8")).toBe("script\n");
    proc.kill("SIGTERM");
    expect(await until(() => proc.exitCode !== null, 20_000), "the job ended at the TERM").toBe(true);
    expect(await proc.exited).toBe(143);
    expect(await until(() => gone(pid)), "gui stopped with the job").toBe(true);
    expect(await until(() => gone(terminal)), "its script(1) too").toBe(true);
  } finally {
    proc.kill();
    for (const p of [pid, terminal]) if (p && !gone(p)) process.kill(p, "SIGKILL");
  }
}, 180_000);

// Value: protects=a script(1) that runs with no child listed (a kernel without CONFIG_PROC_CHILDREN) is a harness error:
//   exit 5 naming script(1), not a wait for a gui nobody can stop, and the work dir goes though gui still runs;
//   fails_when=the empty-children check in gui_run goes (the job then hangs at the TERM that reaches nothing, or reports
//   a difference), or reports exit 1; why_new=gui_run TERMed `${child%% *}` unchecked, so an empty list became
//   `kill -TERM ""` and a wait without end (A0b); seam=PATH (a script(1) that prints an address and sleeps, with no
//   child); no_claim=a real kernel without /proc children (the stub has none)
test("a script(1) with no child listed is a harness error, exit 5", async () => {
  const dir = join(tmp, "no-child");
  mkdirSync(dir, { recursive: true });
  const real = Bun.which("script") ?? "script";
  // the gui call prints an address and a token, then is the sleep itself (no child); every other call is the real script(1)
  const token = "ab".repeat(32);
  writeStub(
    dir,
    "script",
    `#!/bin/sh\ncase "$*" in *" gui --port"*) echo "http://127.0.0.1:1/#t=${token}"; exec sleep 300 ;; esac\nexec ${real} "$@"\n`,
  );
  const r = await guard(tmp, "base", "base", { PATH: `${dir}:${process.env.PATH}` });
  expect(r.code).toBe(5);
  expect(r.stderr).toMatch(/: harness error: no child of script\(1\) in \/proc\/\d+\/task\/\d+\/children: gui cannot be stopped\n/);
  expect(existsSync(join(r.out, "head"))).toBe(false);
  expect(r.left, "the work dir, removed").toEqual([]);
}, 180_000);
