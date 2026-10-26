// Exercise the real harness traps without root or real mounts. Mount stubs
// stop early or drive a paused child; cleanup commands record their effects.
// Value: protects=cleanup preserves mounted directories and reaps paused scans before unmounting;
// fails_when=failed unmounts are ignored, a target is omitted or the child is not killed and reaped;
// why_new=the sudo happy-path harnesses never exercise failed cleanup or controlled signal ordering;
// seam=none; no_claim=real mount/namespace semantics (privileged commands are stubbed)
import { expect, test } from "bun:test";
import { existsSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";

const repo = join(import.meta.dir, "../../..");
const scripts = ["mount-fs.sh", "fault-fs.sh"];

function executable(path: string, body: string) {
  writeFileSync(path, `#!/bin/sh\n${body}\n`, { mode: 0o755 });
}

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), "fs-harness-"));
  const bin = join(dir, "bin");
  const scratch = join(dir, "scratch");
  mkdirSync(bin);
  mkdirSync(scratch);
  executable(
    join(bin, "readlink"),
    `case "$1" in
      /proc/self/ns/mnt) echo "mnt:[\${STUB_PRIVATE_MNT_NS:-\${STUB_CALLER_MNT_NS:-parent}}]";;
      /proc/1/ns/mnt) echo "mnt:[init]";;
      /proc/[0-9]*/ns/mnt) echo "mnt:[\${STUB_CALLER_MNT_NS:-parent}]";;
      *) exit 92;;
    esac`,
  );
  executable(
    join(bin, "unshare"),
    `
    [ "$1" = --mount ] && [ "$2" = --propagation ] && [ "$3" = private ] &&
      [ "$4" = -- ] || exit 91
    shift 4
    export STUB_PRIVATE_MNT_NS=child
    exec "$@"`,
  );
  return {
    dir,
    bin,
    scratch,
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, TMPDIR: scratch, CLEANUP_LOG: join(dir, "cleanup.log") },
  };
}

function run(script: string, unmount: boolean, mounted: boolean, signal?: string, survivor?: string) {
  const { dir, bin, scratch, env } = fixture();
  executable(join(bin, "mount"), signal ? `kill -${signal} "$PPID"; exit 0` : "exit 42");
  executable(
    join(bin, "umount"),
    `echo "$1" >> "$CLEANUP_LOG"; ${survivor ? `case "$1" in */${survivor}) exit 1;; *) exit 0;; esac` : `exit ${unmount ? 0 : 1}`}`,
  );
  executable(join(bin, "mountpoint"), survivor ? `case "$2" in */${survivor}) exit 0;; *) exit 1;; esac` : `exit ${mounted ? 0 : 1}`);
  try {
    const done = Bun.spawnSync(["timeout", "--kill-after=1", "10", "sh", join(repo, "scripts", script), "/bin/false"], {
      env,
    });
    const targets = readFileSync(join(dir, "cleanup.log"), "utf8")
      .trim()
      .split("\n")
      .map((target) => relative(scratch, target).split("/").slice(1).join("/"));
    const retained = readdirSync(scratch);
    return { code: done.exitCode, stderr: done.stderr.toString(), targets, retained };
  } finally {
    // Every mount command was a stub; these directories contain only test files.
    rmSync(dir, { recursive: true, force: true });
  }
}

for (const script of scripts) {
  for (const fakeCaller of [false, true]) {
    test(`${script}: direct internal arguments${fakeCaller ? " with fake caller ID" : ""} cannot bypass nested caller isolation`, () => {
      const { dir, bin, scratch, env } = fixture();
      executable(join(bin, "mount"), 'echo unsafe-mount >&2; exit 99');
      try {
        const args = ["sh", join(repo, "scripts", script), "--private-mount-namespace"];
        if (fakeCaller) args.push("fake-id");
        args.push("/bin/false");
        const result = Bun.spawnSync(args, { env: { ...env, STUB_CALLER_MNT_NS: "nested-caller" } });
        expect(result.exitCode).toBe(fakeCaller ? 2 : 1);
        if (!fakeCaller) expect(result.stderr.toString()).toContain("caller's mount namespace");
        expect(result.stderr.toString()).not.toContain("unsafe-mount");
        expect(readdirSync(scratch)).toEqual([]);
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    });
  }

  for (const [label, launcher] of [
    ["failed namespace launcher", "exit 73"],
    ["unchanged namespace", 'shift 4; exec "$@"'],
  ]) {
    test(`${script}: ${label} refuses to create fixtures`, () => {
      const { dir, bin, scratch, env } = fixture();
      executable(join(bin, "unshare"), launcher);
      executable(join(bin, "mount"), 'echo unsafe-mount >&2; exit 99');
      try {
        const result = Bun.spawnSync(["sh", join(repo, "scripts", script), "/bin/false"], { env });
        expect(result.exitCode).toBe(launcher === "exit 73" ? 73 : 1);
        expect(result.stderr.toString()).not.toContain("unsafe-mount");
        if (launcher !== "exit 73") expect(result.stderr.toString()).toContain("caller's mount namespace");
        expect(readdirSync(scratch)).toEqual([]);
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    });
  }

  test(`${script}: a mount left behind fails cleanup and preserves its directory`, () => {
    const result = run(script, false, true);
    expect(result.code).toBe(1);
    expect(result.stderr).toContain("still mounted");
    expect(result.retained).toHaveLength(1);
    expect(result.targets).toEqual(script === "mount-fs.sh" ? ["root/a/b", "root/a/m"] : ["snap"]);
  });

  for (const unmount of [false, true]) {
    test(`${script}: ${unmount ? "successful unmount" : "an already absent mount"} removes its directory`, () => {
      const result = run(script, unmount, false);
      expect(result.code).toBe(42);
      expect(result.retained).toEqual([]);
      expect(result.stderr).not.toContain("still mounted");
    });
  }

  for (const [signal, code] of [
    ["HUP", 129],
    ["INT", 130],
    ["QUIT", 131],
    ["PIPE", 141],
    ["TERM", 143],
  ] as const) {
    test(`${script}: ${signal} runs cleanup and retains the signal exit code`, () => {
      const result = run(script, true, false, signal);
      expect(result.code).toBe(code);
      expect(result.retained).toEqual([]);
    });
  }
}

for (const survivor of ["b", "m"]) {
  test(`mount-fs.sh: keeps the directory when only ${survivor} survives cleanup`, () => {
    const result = run("mount-fs.sh", false, true, undefined, survivor);
    expect(result.code).toBe(1);
    expect(result.stderr).toContain("still mounted");
    expect(result.retained).toHaveLength(1);
    expect(result.targets).toEqual(["root/a/b", "root/a/m"]);
  });
}

async function waitForFiles(paths: string[]) {
  const deadline = Date.now() + 3000;
  while (!paths.every((path) => existsSync(path) && readFileSync(path, "utf8").trim())) {
    if (Date.now() > deadline) throw new Error("harness did not start its paused child");
    await Bun.sleep(10);
  }
}

function pausedFixture() {
  const fixtureData = fixture();
  const { dir, bin } = fixtureData;
  const childLog = join(dir, "child.pid");
  const readyLog = join(dir, "ready.pid");
  const cli = join(bin, "cli");
  const env = { ...fixtureData.env, CHILD_LOG: childLog, READY_LOG: readyLog, COUNT_LOG: join(dir, "calls") };
  executable(join(bin, "mount"), 'touch "$6/snapshot.db" "$6/snapshot.prev.db"');
  executable(join(bin, "fallocate"), 'touch "$3"');
  executable(join(bin, "jq"), "exit 0");
  executable(join(bin, "sleep"), '[ -e "$READY_LOG" ] || echo "$PPID" > "$READY_LOG"; exec /bin/sleep "$@"');
  executable(join(bin, "mountpoint"), "exit 1");
  executable(
    join(bin, "umount"),
    `
    child=$(cat "$CHILD_LOG")
    if kill -0 "$child" 2>/dev/null; then echo child-alive; else echo child-reaped; fi >> "$CLEANUP_LOG"
    exit 0`,
  );
  executable(
    cli,
    `
    if [ -n "\${FREESPACE_PAUSE:-}" ]; then
      echo "$$" > "$CHILD_LOG"
      exec /bin/sleep 60
    fi
    [ "$1" = scan ] || exit 0
    echo scan >> "$COUNT_LOG"
    [ "$(wc -l < "$COUNT_LOG")" -lt 3 ] || exit 1`,
  );
  return { ...fixtureData, env, cli, childLog, readyLog };
}

function signalIfRunning(pid: number, signal: NodeJS.Signals) {
  try {
    process.kill(pid, signal);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
  }
}

test("fault-fs.sh: a signal kills and reaps the paused scan before unmounting", async () => {
  const { dir, scratch, env, cli, childLog, readyLog } = pausedFixture();
  const runner = Bun.spawn(["timeout", "--kill-after=1", "10", "sh", join(repo, "scripts/fault-fs.sh"), cli], {
    env,
    stdout: "ignore",
    stderr: "pipe",
  });
  try {
    await waitForFiles([childLog, readyLog]);
    const child = Number(readFileSync(childLog, "utf8").trim());
    expect(() => process.kill(child, 0)).not.toThrow();
    process.kill(Number(readFileSync(readyLog, "utf8").trim()), "SIGTERM");
    expect(await runner.exited).toBe(143);
    expect(readFileSync(env.CLEANUP_LOG, "utf8").trim()).toBe("child-reaped");
    expect(() => process.kill(child, 0)).toThrow();
    expect(readdirSync(scratch)).toEqual([]);
  } finally {
    // timeout owns this process group. Give the trap time to reap its child,
    // then stop any remaining stub even when a readiness check or assertion fails.
    signalIfRunning(-runner.pid, "SIGTERM");
    await Promise.race([runner.exited, Bun.sleep(1000)]);
    signalIfRunning(-runner.pid, "SIGKILL");
    await runner.exited;
    rmSync(dir, { recursive: true, force: true });
  }
}, 15000);
