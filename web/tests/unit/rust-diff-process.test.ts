// How scripts/rust-diff.sh starts and stops, on stub builds (rust-diff-stub.ts): bad input, a missing tool and root
// exit 2 before anything runs; a base that cannot make the fixture snapshot stops the job at once and shows why; a HUP,
// INT or TERM ends the job with 128 + the signal number and removes its work dir, a Ctrl-C ends the call in progress
// too, and a server still running goes with the job.
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
import { guard, HOLD, STUB_ENV, scanFails, script, stub, until, writeStub } from "./rust-diff-stub";

let tmp = "";

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
//   status 7 and no message), the scan's output is not shown, or the step number is not the scan's; why_new=every base so
//   far scanned fine; seam=none; no_claim=what the real scan prints when it fails (a stub says it is on fire)
test("a base that cannot make the fixture snapshot stops the job at once and shows why", async () => {
  for (const step of [1, 2]) {
    const r = await guard(tmp, `noscan${step}`, "base");
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
    const r = await guard(tmp, "base", bad);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain(`not an executable file: ${join(tmp, bad)}`);
    expect(existsSync(r.out)).toBe(false);
  }
});

// Value: protects=a missing tool, any of the six, or a run as root stops the job before anything runs: exit 2, no out
//   dir; fails_when=a tool leaves the check (the job then dies midway at a command not found) or the root check goes
//   (install-timer would install the build on this host); why_new=nothing ran these checks, then only one tool was left
//   out (testing review of A0); seam=PATH; no_claim=a real run as root (a stub `id` says 0)
test("a missing tool or a run as root stops the job before anything runs", () => {
  const tools = ["jq", "curl", "script", "flock", "perl", "sqlite3"];
  const cases = tools.map((tool) => {
    const dir = join(tmp, `path-without-${tool}`);
    mkdirSync(dir, { recursive: true });
    for (const other of tools.filter((t) => t !== tool)) symlinkSync(Bun.which(other) ?? other, join(dir, other));
    return [dir, `${tool} is required`] as const;
  });
  const root = join(tmp, "path-root");
  mkdirSync(root, { recursive: true });
  writeStub(root, "id", "#!/bin/sh\necho 0\n");
  for (const [PATH, said] of [...cases, [`${root}:${process.env.PATH}`, "run it as a user, not root"] as const]) {
    const out = join(tmp, `checked-${said.split(" ")[0]}`);
    const r = Bun.spawnSync([Bun.which("bash") ?? "bash", script, join(tmp, "base"), join(tmp, "base"), out], {
      env: { ...process.env, PATH },
      stderr: "pipe",
    });
    expect(r.exitCode, said).toBe(2);
    expect(r.stderr.toString()).toContain(said);
    expect(existsSync(out)).toBe(false);
  }
});

// bash runs a trap when the foreground command ends, so the signal is sent first and the stub released after it.
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

// Value: protects=a server still running when the job ends goes with it: the EXIT trap TERMs the serve call, and timeout
//   passes that on to the build; fails_when=the trap's kill of $serving is dropped, or serve_answers stops setting it (the
//   server then outlives the job, holding a port and a token); why_new=no test ended the job while serve ran (testing
//   and state-machine reviews of A0); seam=none; no_claim=GitHub's job cancellation, and a TERM during any other call
//   (bash runs the trap once that call ends)
test("a TERM while serve waits to start ends the job and stops the server too", async () => {
  const dir = join(tmp, "serve-signal");
  mkdirSync(dir, { recursive: true });
  const [held, served] = [join(dir, "held"), join(dir, "serve.pid")];
  // a server that never prints its address: the job is in its wait for it (serve --port 0 only: `serve --help`, asked
  // before, is a call the job waits out, and its sleep would end by itself)
  const serving = '[ "$*" != "serve --port 0" ] || { echo $$ > "$STUB_SERVE_PID"; : > "$STUB_HELD"; exec sleep 60; }';
  writeStub(dir, "serving", stub({ early: serving }));
  const env = { ...process.env, ...STUB_ENV, STUB_HELD: held, STUB_SERVE_PID: served };
  const proc = Bun.spawn([script, join(dir, "serving"), join(dir, "serving"), join(dir, "out")], {
    env,
    stdout: "ignore",
    stderr: "ignore",
  });
  let pid = 0;
  const gone = () => {
    try {
      process.kill(pid, 0);
      return false;
    } catch {
      return true;
    }
  };
  try {
    expect(await until(() => existsSync(held), 120_000), "the script reached serve").toBe(true);
    pid = Number(readFileSync(served, "utf8"));
    expect(gone(), "the server runs").toBe(false);
    proc.kill("SIGTERM");
    expect(await proc.exited).toBe(143);
    expect(await until(gone), "the server stopped with the job").toBe(true);
  } finally {
    proc.kill();
    if (pid && !gone()) process.kill(pid, "SIGKILL");
  }
}, 150_000);
