// Local checks never call the real installer. Root/systemd integration belongs to rust-install-diff on GitHub.
import { afterAll, beforeAll, expect, test } from "bun:test";
import { chmodSync, existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const repo = resolve(import.meta.dir, "../../..");
const helper = join(repo, "scripts/rust-diff/install-kept.sh");
let tmp = "";
beforeAll(() => {
  tmp = mkdtempSync(join(tmpdir(), "install-diff-test-"));
});
afterAll(() => {
  rmSync(tmp, { recursive: true, force: true });
});
function shell(code: string, ...args: string[]) {
  return Bun.spawnSync(["bash", "-c", `set -euo pipefail; . "$1"; shift; ${code}`, "test", helper, ...args]);
}

// Value: masks only the real scheduled time and exact input path; schema/value regressions remain visible.
test("report normalization preserves null, weekday, fields, foreign paths and invalid clock shapes", () => {
  const file = join(tmp, "report.json");
  writeFileSync(file, JSON.stringify({ copied_from: "/base", next_run: "Mon 2026-10-26 23:12:00 UTC", scan_started: false, extra: 17 }));
  const got = shell('install_report "$1" /base', file);
  expect(got.exitCode).toBe(0);
  expect(JSON.parse(got.stdout.toString())).toEqual({
    copied_from: "<source-binary>",
    next_run: "Mon <scheduled-date-time> UTC",
    scan_started: false,
    extra: 17,
  });
  writeFileSync(file, JSON.stringify({ copied_from: "/unexpected", next_run: null, scan_started: true }));
  expect(JSON.parse(shell('install_report "$1" /base', file).stdout.toString())).toEqual({
    copied_from: "/unexpected",
    next_run: null,
    scan_started: true,
  });
  for (const next_run of [
    42,
    "unknown",
    "Mon 2026-99-26 23:12:00 UTC",
    "Tue 2026-10-26 23:12:00 UTC",
    "Mon 2026-10-26 23:12:00 PDT",
    "prefix Mon 2026-10-26 23:12:00 UTC",
  ]) {
    writeFileSync(file, JSON.stringify({ next_run }));
    expect(shell('install_report "$1" /base', file).exitCode).not.toBe(0);
  }
});

// Value: observes real filesystem metadata/ACLs, links without following, changed bytes and missing paths.
test("manifest records actual modes, numeric owners, link targets, ACLs and changed content", () => {
  const file = join(tmp, "manifest-file");
  const link = join(tmp, "manifest-link");
  writeFileSync(file, "original\n");
  chmodSync(file, 0o640);
  symlinkSync(file, link);
  const acl = Bun.spawnSync(["setfacl", "-m", "u:65534:r--", file]);
  expect(acl.exitCode).toBe(0);
  const first = shell('install_kept "$1" /unused 1.2.3', file);
  expect(first.exitCode).toBe(0);
  expect(first.stdout.toString()).toContain(`uid=${process.getuid?.()}`);
  expect(first.stdout.toString()).toContain("mode=640");
  expect(first.stdout.toString()).toContain("user:65534:r--");
  expect(first.stdout.toString()).toContain("original\n");
  chmodSync(file, 0o600);
  const changedMode = shell('install_kept "$1" /unused 1.2.3', file);
  expect(changedMode.stdout.toString()).toContain("mode=600");
  expect(changedMode.stdout.toString()).not.toBe(first.stdout.toString());
  writeFileSync(file, "changed\n");
  expect(shell('install_kept "$1" /unused 1.2.3', file).stdout.toString()).not.toBe(first.stdout.toString());
  const linked = shell('install_kept "$1" /unused 1.2.3', link);
  expect(linked.exitCode).toBe(0);
  expect(linked.stdout.toString()).toContain("type=symbolic link");
  expect(linked.stdout.toString()).toContain(`-> '${file}'`);
  expect(linked.stdout.toString()).not.toContain("changed\n");
  expect(shell('install_kept "$1" /unused 1.2.3', join(tmp, "absent")).stdout.toString()).toContain("absent ");
});

// Value: changes to a different version stamp, or to ordinary skill prose, are not erased.
test("only the exact own-version skill stamp is normalized", () => {
  const dir = join(tmp, "freespace");
  expect(Bun.spawnSync(["mkdir", dir]).exitCode).toBe(0);
  const file = join(dir, "SKILL.md");
  writeFileSync(file, "<!-- freespace-version: 1.2.3 -->\n<!-- freespace-version: 9.9.9 -->\nordinary version 1.2.3\n");
  const result = shell('install_kept "$1" /unused 1.2.3', file);
  expect(result.exitCode).toBe(0);
  expect(result.stdout.toString()).toContain("<!-- freespace-version: <own-version> -->");
  expect(result.stdout.toString()).toContain("<!-- freespace-version: 9.9.9 -->");
  expect(result.stdout.toString()).toContain("ordinary version 1.2.3");
});

// Value: the real content helper preserves final-newline, CRLF, embedded bytes and foreign stamps;
// removing a final newline is an actual negative control in scratch, never an installed host edit.
test("skill normalization preserves all other bytes including final newline state", () => {
  const file = join(tmp, "skill-bytes");
  const stamp = "<!-- freespace-version: 1.2.3 -->";
  const normalized = "<!-- freespace-version: <own-version> -->";
  for (const ending of ["", "\n", "\n\n"]) {
    const rest = Buffer.from("\n<!-- freespace-version: 9.9.9 -->\r\nordinary 1.2.3\0\xff", "latin1");
    writeFileSync(file, Buffer.concat([Buffer.from(stamp), rest, Buffer.from(ending)]));
    const got = shell('install_skill_content "$1" 1.2.3', file);
    expect(got.exitCode).toBe(0);
    expect(got.stdout.equals(Buffer.concat([Buffer.from(normalized), rest, Buffer.from(ending)]))).toBe(true);
  }
  // Exercise dispatch through a real freespace/SKILL.md path as well as the content helper.
  const dir = join(tmp, "newline-control", "freespace");
  mkdirSync(dir, { recursive: true });
  const skill = join(dir, "SKILL.md");
  writeFileSync(skill, `${stamp}\n`);
  const withNewline = shell('install_kept "$1" /unused 1.2.3', skill);
  writeFileSync(skill, stamp);
  const withoutNewline = shell('install_kept "$1" /unused 1.2.3', skill);
  expect(withNewline.exitCode).toBe(0);
  expect(withoutNewline.exitCode).toBe(0);
  expect(withoutNewline.stdout.equals(withNewline.stdout)).toBe(false);
});

test("text normalization preserves foreign headers, unknown times, shape and every unrelated value", () => {
  const file = join(tmp, "text-report");
  const line = "  timer      freespace.timer, next run Mon 2026-10-26 23:12:00 UTC";
  const text = `freespace 1.2.3 timer installed\nordinary 1.2.3 /base null\n${line}\n`;
  writeFileSync(file, text);
  const got = shell('install_text_report "$1" 1.2.3', file);
  expect(got.exitCode).toBe(0);
  expect(got.stdout.toString()).toBe(
    "freespace <own-version> timer installed\nordinary 1.2.3 /base null\n  timer      freespace.timer, next run Mon <scheduled-date-time> UTC\n",
  );
  for (const preserved of [
    "freespace 9.9.9 timer installed\n",
    "prefix freespace 1.2.3 timer installed\n",
    "  timer      freespace.timer, next run unknown\n",
    "  timer      freespace.timer, next run null\n",
    "  timer      freespace.timer, next run Mon 2026-10-26 23:12:00 PDT\n",
    `prefix ${line}\n`,
    "ordinary 1.2.3 without final newline",
  ]) {
    writeFileSync(file, preserved);
    expect(shell('install_text_report "$1" 1.2.3', file).stdout.toString()).toBe(preserved);
  }
  writeFileSync(file, line.replace("Mon", "Tue"));
  expect(shell('install_text_report "$1" 1.2.3', file).exitCode).not.toBe(0);
});

test("actual text and JSON reports must prove ACL, scheduled time, expected copy behavior and no scan", () => {
  const file = join(tmp, "no-scan-report");
  const text =
    "freespace 1.2.3 timer installed\n  config     /etc/freespace/config.toml (roots /scratch, daily at 23:12, keep_previous true)\n  low_memory false (auto: estimate fits budget)\n  snapshots  /scratch/snap (read access for human (ACL))\n  timer      freespace.timer, next run Mon 2026-10-26 23:12:00 UTC\n";
  for (const [report, accepted] of [
    [text, true],
    [text.replace("1.2.3 timer", "9.9.9 timer"), false],
    [text.replace("freespace 1.2.3", "prefix freespace 1.2.3"), false],
    [text.replace("/etc/freespace/config.toml", "/foreign/config.toml"), false],
    [text.replace("roots /scratch,", "roots /foreign,"), false],
    [text.replace("daily at", "hourly at"), false],
    [text.replace("daily at 23:12", "daily at 01:23"), false],
    [text.replace("keep_previous true", "keep_previous false"), false],
    [text.replace("(ACL)", "via group freespace"), false],
    [text.replace("low_memory false", "low_memory unknown"), false],
    [text.replace(" (auto: estimate fits budget)", ""), false],
    [text.replace("Mon 2026-10-26 23:12:00 UTC", "unknown"), false],
    [`${text}First scan started (no snapshot)\n`, false],
    [text.replace("human", "foreign"), false],
  ] as const) {
    writeFileSync(file, report);
    expect(
      shell('install_text_check "$1" 1.2.3 /etc/freespace/config.toml /scratch daily 23:12 true /scratch/snap human', file).exitCode === 0,
    ).toBe(accepted);
  }
  const report = { access: "acl", next_run: "Mon 2026-10-26 23:12:00 UTC", scan_started: false, scan_reason: null, copied_from: null };
  for (const change of [
    {},
    { copied_from: "/source" },
    { access: "group" },
    { scan_started: true },
    { scan_reason: "started" },
    { next_run: null },
  ]) {
    writeFileSync(file, JSON.stringify({ ...report, ...change }));
    expect(shell('install_no_scan_report "$1" true', file).exitCode === 0).toBe(Object.keys(change).length === 0);
  }
  writeFileSync(file, JSON.stringify({ ...report, copied_from: "/source" }));
  expect(shell('install_no_scan_report "$1" false /source', file).exitCode).toBe(0);
  expect(shell('install_no_scan_report "$1" false /foreign', file).exitCode).not.toBe(0);
});

// Value: protects=usage and local-context refusals occur before writes; fails_when=bad count/flag reaches a later guard or writes;
// why_new=the original case used only valid-shaped arguments; seam=none (actual script entry);
// non-testcaller=CI invokes this same entry; no_claim=privileged/pristine-host safety or lifecycle.
test("real comparator refuses outside the disposable GitHub runner", () => {
  const { GITHUB_ACTIONS: _github, RUNNER_ENVIRONMENT: _runner, ...env } = process.env;
  const result = Bun.spawnSync(
    [
      "bash",
      join(repo, "scripts/rust-diff-install.sh"),
      "--github-disposable-runner",
      "/bin/true",
      "/bin/true",
      join(tmp, "never-output"),
      join(tmp, "never-scratch"),
      "nobody",
    ],
    { env },
  );
  expect(result.exitCode).toBe(2);
  expect(result.stderr.toString()).toContain("disposable GitHub-hosted runner");
  expect(() => readFileSync(join(tmp, "never-output"))).toThrow();
  expect(() => readFileSync(join(tmp, "never-scratch"))).toThrow();
  const valid = ["--github-disposable-runner", "/bin/true", "/bin/true", join(tmp, "never-output"), join(tmp, "never-scratch"), "nobody"];
  for (const args of [[], valid.slice(0, -1), [...valid, "extra"], ["--wrong", ...valid.slice(1)]]) {
    const rejected = Bun.spawnSync(["bash", join(repo, "scripts/rust-diff-install.sh"), ...args], { env });
    expect(rejected.exitCode).toBe(2);
    expect(rejected.stdout.toString()).toBe("");
    expect(rejected.stderr.toString()).toContain("expected --github-disposable-runner BASE HEAD OUT SCRATCH USER");
    expect(rejected.stderr.toString()).not.toContain("unbound variable");
    expect(existsSync(join(tmp, "never-output"))).toBe(false);
    expect(existsSync(join(tmp, "never-scratch"))).toBe(false);
  }
});

// Value: cleanup only removes directories it created, in reverse order; unexpected residual content fails safely.
test("parent cleanup removes empty chains and propagates failure without deleting residue", () => {
  const parent = join(tmp, "cleanup-parent");
  const child = join(parent, "child");
  expect(Bun.spawnSync(["mkdir", "-p", child]).exitCode).toBe(0);
  writeFileSync(join(child, "residue"), "keep this evidence");
  const failed = shell('install_remove_empty_parents "$1" "$2"', parent, child);
  expect(failed.exitCode).not.toBe(0);
  expect(readFileSync(join(child, "residue"), "utf8")).toBe("keep this evidence");
  rmSync(join(child, "residue"));
  expect(shell('install_remove_empty_parents "$1" "$2"', parent, child).exitCode).toBe(0);
  expect(() => readFileSync(child)).toThrow();
  expect(() => readFileSync(parent)).toThrow();
});

// Value: a failed ACL read cannot become an apparently equal, partial manifest through a pipeline.
test("manifest propagates tool failures", () => {
  const file = join(tmp, "acl-failure");
  writeFileSync(file, "data");
  const failed = shell('getfacl() { return 37; }; install_kept "$1" /unused 1.2.3', file);
  expect(failed.exitCode).toBe(37);
});

// Value: protects=stable recursive evidence for nested content, hardlink counts and unusual bytes; fails_when=walk omits/reorders
// entries, follows directory links or loses names/content; why_new=existing manifest tests pass individual leaves only;
// seam=none (real scratch files and tools); non-testcaller=capture() calls install_kept for manifest roots;
// no_claim=arbitrary hardlink-group identity, every possible filename or installed host artifacts.
test("recursive manifest orders real entries and preserves hardlinks and unusual names", () => {
  const root = join(tmp, "recursive");
  mkdirSync(join(root, "a"), { recursive: true });
  writeFileSync(join(root, "z"), "shared-payload\n");
  linkSync(join(root, "z"), join(root, "a", "hard"));
  writeFileSync(join(root, "a", "nested"), "nested-payload\n");
  const newline = join(root, "m\nname");
  const odd = Buffer.concat([Buffer.from(`${root}/`), Buffer.from([0xff])]);
  writeFileSync(newline, "newline-payload\n");
  writeFileSync(odd, "odd-payload\n");
  const outside = join(tmp, "outside");
  mkdirSync(outside);
  writeFileSync(join(outside, "secret"), "must-not-follow\n");
  symlinkSync(outside, join(root, "linked"));
  const first = shell('install_kept "$1" /unused 1.2.3', root);
  const again = shell('install_kept "$1" /unused 1.2.3', root);
  expect(first.exitCode).toBe(0);
  expect(first.stderr.toString()).toBe("");
  expect(again.stdout.equals(first.stdout)).toBe(true);
  const ordered = [
    root,
    join(root, "a"),
    join(root, "a", "hard"),
    join(root, "a", "nested"),
    join(root, "linked"),
    newline,
    join(root, "z"),
    odd,
  ];
  const positions = ordered.map((path) =>
    first.stdout.indexOf(Buffer.concat([Buffer.from("path "), Buffer.from(path), Buffer.from("\n")])),
  );
  expect(positions.every((position) => position >= 0)).toBe(true);
  expect(positions).toEqual([...positions].sort((a, b) => a - b));
  expect(first.stdout.toString().match(/type=regular file .*links=2 /g)).toHaveLength(2);
  for (const payload of ["shared-payload\n", "nested-payload\n", "newline-payload\n", "odd-payload\n"]) {
    expect(first.stdout.toString()).toContain(payload);
  }
  expect(first.stdout.toString()).not.toContain("must-not-follow");
  expect(first.stdout.toString()).toContain(`-> '${outside}'`);
});

// Value: protects=only exit-1/not-found is tolerated and actual property output/status retained; fails_when=systemctl errors
// pass or requested unit/properties are lost; why_new=no prior unit_state execution; seam=systemctl tool boundary stub;
// non-testcaller=capture() calls unit_state for timer/service; no_claim=real systemd behavior or distro compatibility.
test("unit state preserves loaded and missing units while propagating other systemctl failures", () => {
  const cases = [
    { code: 0, text: "LoadState=loaded\nActiveState=active\n", wanted: 0 },
    { code: 1, text: "LoadState=not-found\nActiveState=inactive\n", wanted: 0 },
    { code: 1, text: "LoadState=error\n", wanted: 1 },
    { code: 37, text: "LoadState=not-found\n", wanted: 37 },
    { code: 1, text: "prefix LoadState=not-found\n", wanted: 1 },
  ];
  for (const [index, row] of cases.entries()) {
    const scratch = join(tmp, `unit-${index}`);
    mkdirSync(scratch);
    const result = shell(
      `scratch=$1; response=$2; status=$3
systemctl() { printf '%s\\n' "$*" >&2; printf '%s' "$response"; return "$status"; }
unit_state freespace.timer LoadState ActiveState; echo accepted`,
      scratch,
      row.text,
      String(row.code),
    );
    expect(result.exitCode).toBe(row.wanted);
    expect(result.stderr.toString()).toBe("show freespace.timer -p LoadState -p ActiveState\n");
    expect(result.stdout.toString()).toBe(`${row.text}systemctl-show-exit=${row.code}\n${row.wanted === 0 ? "accepted\n" : ""}`);
  }
});

const comparisonFiles = [
  "installed.txt",
  "installed-rerun.txt",
  "residual.txt",
  "install.txt",
  "uninstall.txt",
  "rerun.json",
  "rerun.stderr",
  "install.json",
  "uninstall.json",
  "install.stderr",
  "uninstall.stderr",
  "skill.stdout",
  "skill.stderr",
];
function comparisonFixture(name: string) {
  const dir = join(tmp, name);
  const out = join(dir, "out");
  const scratch = join(dir, "scratch");
  mkdirSync(scratch, { recursive: true });
  const base = join(dir, "base[1]");
  const head = join(dir, "head[2]");
  for (const [bin, version] of [
    [base, "1.2.3"],
    [head, "2.3.4"],
  ]) {
    writeFileSync(bin, `#!/bin/sh\nprintf 'freespace ${version}\\n'\n`, { mode: 0o755 });
  }
  for (const label of ["base", "base2", "head", "base-text", "base2-text", "head-text"]) {
    mkdirSync(join(out, label), { recursive: true });
    for (const file of comparisonFiles) writeFileSync(join(out, label, file), `unchanged ${file}\n`);
    const bin = label.startsWith("head") ? head : base;
    const version = label.startsWith("head") ? "2.3.4" : "1.2.3";
    writeFileSync(join(out, label, "install.stderr"), `installed /usr/local/bin/freespace (copied from ${bin})\n`);
    writeFileSync(join(out, label, "skill.stderr"), `freespace skill ${version} installed in /owned/home/.claude/skills/freespace\n`);
  }
  return { out, scratch, base, head };
}

// Value: protects=distinct comparison outcomes and exact diagnostic masking; fails_when=mismatch/error passes, equality fails,
// or foreign/partial/ordinary text is erased; why_new=no prior compare execution; seam=diff tool stub only for its error branch;
// non-testcaller=main invokes compare(base,base2,3) and compare(base,head,1); run() executes real owned version binaries;
// no_claim=main orchestration stopping head after failed base, real root lifecycle, or every diagnostic language/format.
test("comparison distinguishes equality, base drift, head drift, tool errors and exact diagnostic masks", () => {
  const cases = [
    { name: "equal", label: "head", code: 0 },
    { name: "equal-text", label: "head-text", code: 0 },
    { name: "text-drift", label: "head-text", code: 1, file: "install.txt", content: "changed text\n" },
    { name: "text-uninstall-drift", label: "head-text", code: 1, file: "uninstall.txt", content: "left text residue\n" },
    { name: "text-stderr-drift", label: "head-text", code: 1, file: "uninstall.stderr", content: "unexpected error\n" },
    { name: "text-base-drift", label: "base2-text", code: 3, file: "installed.txt", content: "changed metadata\n" },
    { name: "rerun-report-drift", label: "head", code: 1, file: "rerun.json", content: '{"copied_from":"/foreign"}\n' },
    { name: "rerun-manifest-drift", label: "head-text", code: 1, file: "installed-rerun.txt", content: "changed ACL\n" },
    { name: "base-drift", label: "base2", code: 3, file: "installed.txt", content: "changed metadata\n" },
    { name: "head-drift", label: "head", code: 1, file: "residual.txt", content: "leftover\n" },
    { name: "diff-error", label: "head", code: 5, stub: "diff() { echo tool-failed >&2; return 2; };" },
    {
      name: "foreign-path",
      label: "head",
      code: 1,
      file: "install.stderr",
      content: "installed /usr/local/bin/freespace (copied from /foreign)\n",
    },
    {
      name: "partial-line",
      label: "head",
      code: 1,
      file: "install.stderr",
      content: "prefix installed /usr/local/bin/freespace (copied from @BIN@)\n",
      baseContent: "prefix installed /usr/local/bin/freespace (copied from @BIN@)\n",
    },
    {
      name: "foreign-version",
      label: "head",
      code: 1,
      file: "skill.stderr",
      content: "freespace skill 9.9.9 installed in /owned/home/.claude/skills/freespace\n",
    },
    {
      name: "ordinary-version",
      label: "head",
      code: 1,
      file: "skill.stderr",
      content: "ordinary version 2.3.4\n",
      baseContent: "ordinary version 1.2.3\n",
    },
  ];
  for (const row of cases) {
    const fixture = comparisonFixture(row.name);
    const left = row.label.endsWith("-text") ? "base-text" : "base";
    const content = row.content?.replaceAll("@BIN@", fixture.head);
    if (row.file) writeFileSync(join(fixture.out, row.label, row.file), content ?? "");
    if (row.baseContent && row.file) writeFileSync(join(fixture.out, left, row.file), row.baseContent.replaceAll("@BIN@", fixture.base));
    const result = shell(
      `out=$1 scratch=$2 base=$3 head=$4 home=$5 human=owned-human
${row.stub ?? ""}
compare "$8" "$6" "$7"; echo compared`,
      fixture.out,
      fixture.scratch,
      fixture.base,
      fixture.head,
      tmp,
      row.label,
      row.label.startsWith("base2") ? "3" : "1",
      left,
    );
    expect(result.exitCode).toBe(row.code);
    if (row.code === 0) {
      expect(result.stdout.toString()).toBe("compared\n");
      expect(result.stderr.toString()).toBe("");
    } else if (row.code === 5) {
      expect(result.stderr.toString()).toContain("tool-failed");
      expect(result.stderr.toString()).toContain("diff failed (2)");
    } else {
      expect(result.stdout.toString()).toContain(content?.trim() ?? "");
      expect(result.stderr.toString()).toContain(`${row.label} differs from ${left} (${row.file})`);
      expect(result.stdout.toString()).not.toContain("compared\n");
    }
  }
});

// Value: protects=fixed clean environment, bounded timeout argv, closed stdin and real command failure; fails_when=caller env leaks,
// timeout policy changes or failure becomes success; why_new=earlier tests bypassed run(); seam=none (real env/timeout/owned command);
// non-testcaller=take() uses run for install/completions/skill/uninstall and compare() for version; no_claim=waiting 120s,
// timeout kill behavior, uninterruptible processes, signal cleanup or root commands.
test("run isolates environment and invokes the fixed timeout policy while preserving failure", () => {
  const clean = shell(
    "home=$1 human=owned-human; export LEAK_ME=secret FREESPACE_CONFIG=/foreign XDG_CONFIG_HOME=/foreign; run /usr/bin/env -0",
    tmp,
  );
  expect(clean.exitCode).toBe(0);
  const entries = clean.stdout.toString().split("\0").filter(Boolean).sort();
  expect(entries).toEqual([`HOME=${tmp}`, "LC_ALL=C", "PATH=/usr/sbin:/usr/bin:/sbin:/bin", "SUDO_USER=owned-human", "TZ=UTC"].sort());
  const command = join(tmp, "owned-command");
  writeFileSync(
    command,
    '#!/bin/bash\nif read -r input; then exit 99; fi\ntr "\\0" "\\n" < "/proc/$PPID/cmdline"\nprintf "owned failure\\n" >&2\nexit 37\n',
    { mode: 0o755 },
  );
  const failed = shell('home=$1 human=owned-human; run "$2" <<< caller-input; echo must-not-reach', tmp, command);
  expect(failed.exitCode).toBe(37);
  expect(failed.stderr.toString()).toBe("owned failure\n");
  expect(failed.stdout.toString().trim().split("\n")).toEqual(["timeout", "-k", "10", "120", command]);
});

// Value: protects=prerequisite/executable/confinement refusals and runner/user context contracts before writes;
// fails_when=missing tool, unsafe input or identity is accepted; why_new=entry tests stop before these operational checks;
// seam=id/getent read-tool stubs only; real PATH/file predicates; non-testcaller=main calls these preflight validators before mkdir;
// no_claim=absent-systemd/non-UTC-host guards, real root identity, canonicalization races or full host installation.
test("preflight validates tools, executable inputs, confined fresh paths and runner identity", () => {
  const root = join(tmp, "inputs");
  mkdirSync(root);
  const blocked = join(root, "not-executable");
  writeFileSync(blocked, "sentinel");
  const existing = join(root, "existing");
  mkdirSync(existing);
  const dangling = join(root, "dangling");
  symlinkSync(join(root, "missing-target"), dangling);
  const defaults = { base: "/bin/true", out: join(root, "out"), scratch: join(root, "scratch") };
  const cases = [
    { ...defaults, code: 0, says: "" },
    { ...defaults, code: 2, says: "jq required", prefix: 'PATH="";' },
    { ...defaults, base: blocked, code: 2, says: "both builds must be executable" },
    { ...defaults, base: blocked, code: 2, says: "both builds must be executable", prefix: "head=$base; base=/bin/true;" },
    { ...defaults, base: join(root, "missing-bin"), code: 2, says: "both builds must be executable" },
    { ...defaults, scratch: join(tmp, "outside"), code: 2, says: "scratch must be directly inside" },
    { ...defaults, scratch: join(root, "with space"), code: 2, says: "no whitespace" },
    { ...defaults, scratch: existing, code: 2, says: "scratch must not exist" },
    { ...defaults, scratch: dangling, code: 2, says: "scratch must not exist" },
    { ...defaults, out: existing, code: 2, says: "OUT must not exist" },
    { ...defaults, out: dangling, code: 2, says: "OUT must not exist" },
    { ...defaults, out: join(tmp, "outside-out"), code: 2, says: "OUT must be directly inside" },
    { ...defaults, out: defaults.scratch, code: 2, says: "OUT must be outside scratch" },
    { ...defaults, out: `${root}/./scratch`, code: 2, says: "OUT must be outside scratch" },
  ];
  for (const row of cases) {
    const result = shell(
      `base=$1 head=/bin/true out=$2 scratch=$3 RUNNER_TEMP=$4; ${"prefix" in row ? row.prefix : ""} install_check_inputs; echo accepted`,
      row.base,
      row.out,
      row.scratch,
      root,
    );
    expect(result.exitCode).toBe(row.code);
    expect(result.stdout.toString()).toBe(row.code === 0 ? "accepted\n" : "");
    expect(result.stderr.toString()).toContain(row.says);
    expect(existsSync(defaults.out)).toBe(false);
    expect(existsSync(defaults.scratch)).toBe(false);
  }
  for (const [uid, github, runner, code] of [
    ["0", "true", "github-hosted", 0],
    ["1000", "true", "github-hosted", 2],
    ["0", "false", "github-hosted", 2],
    ["0", "true", "self-hosted", 2],
  ] as const) {
    const result = shell(
      'uid=$1; id() { echo "$uid"; }; GITHUB_ACTIONS=$2 RUNNER_ENVIRONMENT=$3 install_require_runner',
      uid,
      github,
      runner,
    );
    expect(result.exitCode).toBe(code);
    if (code) expect(result.stderr.toString()).toContain("disposable GitHub-hosted runner");
  }
  for (const [user, home, status, code] of [
    ["human", root, "0", 0],
    ["root", root, "0", 2],
    ["unknown", root, "2", 2],
    ["human", "/", "0", 2],
    ["human", join(root, "missing-home"), "0", 2],
  ] as const) {
    const result = shell(
      'record_home=$2 status=$3; getent() { printf "human:x:1000:1000::%s:/bin/sh\\n" "$record_home"; return "$status"; }; install_user_home "$1"',
      user,
      home,
      status,
    );
    expect(result.exitCode).toBe(code);
    expect(result.stdout.toString()).toBe(code === 0 ? `${home}\n` : "");
    if (code) expect(result.stderr.toString()).toContain(status === "2" ? "USER must exist" : "existing non-root home");
  }
  expect(readFileSync(blocked, "utf8")).toBe("sentinel");
  expect(existsSync(existing)).toBe(true);
});

// Value: protects=preexisting files/links/groups/units and unsafe ancestor metadata refuse before mutation;
// fails_when=sentinel/group/loaded state or symlink/foreign-owner/write bits pass; why_new=no prior pristine/ancestor execution;
// seam=getent/systemctl read stubs; stat UID only is synthetic for owned-mode cases (actual predicates/modes preserved);
// non-testcaller=main calls install_pristine and install_root_ancestors, which calls install_root_directory;
// no_claim=real privileged ownership changes, every host manifest entry, TOCTOU or root lifecycle.
test("pristine and ancestor validators reject real sentinels and unsafe observed metadata", () => {
  const root = join(tmp, "pristine");
  mkdirSync(root);
  const sentinel = join(root, "sentinel");
  writeFileSync(sentinel, "preserve-me");
  const dangling = join(root, "dangling");
  symlinkSync(join(root, "missing"), dangling);
  for (const row of [
    { path: join(root, "absent"), group: "1", unit: "not-found", code: 0, says: "" },
    { path: sentinel, group: "1", unit: "not-found", code: 2, says: sentinel },
    { path: dangling, group: "1", unit: "not-found", code: 2, says: dangling },
    { path: join(root, "absent"), group: "0", unit: "not-found", code: 2, says: "group exists" },
    { path: join(root, "absent"), group: "1", unit: "loaded", code: 2, says: "known to systemd" },
    {
      path: join(root, "absent"),
      group: "1",
      unit: "not-found",
      badUnit: "freespace.service",
      code: 2,
      says: "freespace.service is known",
    },
  ]) {
    const result = shell(
      'group=$2 response=$3 bad_unit=$4; getent() { return "$group"; }; systemctl() { if [ "$2" = "$bad_unit" ]; then echo loaded; else printf "%s\\n" "$response"; fi; return 1; }; install_pristine "$1"; echo accepted',
      row.path,
      row.group,
      row.unit,
      ("badUnit" in row ? row.badUnit : "") ?? "",
    );
    expect(result.exitCode).toBe(row.code);
    expect(result.stdout.toString()).toBe(row.code === 0 ? "accepted\n" : "");
    expect(result.stderr.toString()).toContain(row.says);
  }
  const safe = join(root, "mode-case");
  mkdirSync(safe);
  for (const mode of [0o755, 0o775, 0o757]) {
    chmodSync(safe, mode);
    const result = shell(
      'stat() { if [ "$2" = %u ]; then echo 0; else command stat "$@"; fi; }; install_root_directory "$1"; echo accepted',
      safe,
    );
    expect(result.exitCode).toBe(mode === 0o755 ? 0 : 2);
    expect(result.stdout.toString()).toBe(mode === 0o755 ? "accepted\n" : "");
    if (mode !== 0o755) expect(result.stderr.toString()).toContain(`unsafe manifest ancestor: ${safe}`);
  }
  const foreign = shell('install_root_directory "$1"', safe);
  expect(foreign.exitCode).toBe(2);
  expect(foreign.stderr.toString()).toContain(`unsafe manifest ancestor: ${safe}`);
  const linked = shell('install_root_ancestors "$1"', join(dangling, "leaf"));
  expect(linked.exitCode).toBe(2);
  expect(linked.stderr.toString()).toContain(`unsafe symlink ancestor: ${dangling}`);
  expect(readFileSync(sentinel, "utf8")).toBe("preserve-me");
});

// Value: protects=installed-copy content is verified against that take's input, and corruption/missing input fails;
// fails_when=cmp is dropped/ignored or success marker precedes verification; why_new=exact binary branch was unreachable locally;
// seam=none (actual cmp and owned bytes); non-testcaller=install_kept exact /usr/local/bin/freespace branch calls this helper;
// no_claim=exact-path dispatch, root installation, executable provenance or independent build correctness.
test("binary verification accepts exact input bytes and propagates corruption or missing input", () => {
  const input = join(tmp, "binary-input");
  const installed = join(tmp, "binary-copy");
  const bytes = Buffer.from([0, 255, 1, 2, 128]);
  writeFileSync(input, bytes);
  writeFileSync(installed, bytes);
  const equal = shell('install_verify_binary "$1" "$2"', installed, input);
  expect(equal.exitCode).toBe(0);
  expect(equal.stdout.toString()).toBe("content=identical-to-input-binary\n");
  expect(equal.stderr.toString()).toBe("");
  writeFileSync(installed, Buffer.from([0, 255, 1, 9, 128]));
  const corrupt = shell('install_verify_binary "$1" "$2"', installed, input);
  expect(corrupt.exitCode).toBe(1);
  expect(corrupt.stdout.toString()).toContain("differ: byte 4");
  expect(corrupt.stdout.toString()).not.toContain("content=identical");
  const missing = shell('install_verify_binary "$1" "$2"', installed, join(tmp, "missing-input"));
  expect(missing.exitCode).toBe(2);
  expect(missing.stderr.toString()).toContain("No such file or directory");
  expect(missing.stdout.toString()).not.toContain("content=identical");
});

type GuardStep = { name?: string; uses?: string; run?: string; if?: string; with?: Record<string, unknown> };
type GuardJob = { if: string; "runs-on": string; "timeout-minutes": number; steps: GuardStep[] };
function jobRun(job: GuardJob, name: string) {
  const step = job.steps.find((candidate) => candidate.name === name);
  expect(step?.run, name).toBeString();
  return step?.run ?? "";
}

// Value: execute the actual preparation loop against scratch directories. Missing parents remain absent;
// existing completion ancestors receive both ownership/mode commands; a symlink or file refuses safely.
test("workflow prepares every existing completion ancestor and refuses unsafe ones", () => {
  const workflow = Bun.YAML.parse(readFileSync(join(repo, ".github/workflows/refactor-guard.yml"), "utf8")) as {
    jobs: Record<string, GuardJob>;
  };
  const original = jobRun(workflow.jobs["rust-install-diff"], "prepare disposable runner prerequisites");
  for (const kind of ["safe", "symlink", "file"]) {
    const local = join(tmp, `prerequisites-${kind}`, "local");
    const completion = join(local, "share/bash-completion/completions");
    mkdirSync(completion, { recursive: true });
    mkdirSync(join(local, "bin"));
    if (kind === "symlink") symlinkSync(join(local, "bin"), join(local, "share/zsh"));
    if (kind === "file") writeFileSync(join(local, "share/zsh"), "sentinel");
    const result = shell(`sudo() { printf '%s\\n' "$*"; }
${original.replaceAll("/usr/local", local)}`);
    expect(result.exitCode).toBe(kind === "safe" ? 0 : 1);
    const commands = result.stdout.toString().trim().split("\n");
    expect(commands).toEqual(
      expect.arrayContaining([
        "timeout 180 apt-get update -q",
        "env DEBIAN_FRONTEND=noninteractive timeout 180 apt-get install -yq acl",
        `chown root:root ${local} ${local}/bin`,
        `chmod go-w ${local} ${local}/bin`,
      ]),
    );
    for (const path of [join(local, "share"), join(local, "share/bash-completion"), completion]) {
      expect(commands).toContain(`chown root:root -- ${path}`);
      expect(commands).toContain(`chmod go-w -- ${path}`);
    }
    expect(commands).not.toContain(`chown root:root -- ${local}/share/fish`);
    expect(existsSync(join(local, "share/fish"))).toBe(false);
    if (kind !== "safe") expect(result.stderr.toString()).toContain(`unsafe completion ancestor: ${local}/share/zsh`);
    if (kind === "file") expect(readFileSync(join(local, "share/zsh"), "utf8")).toBe("sentinel");
  }
});

// Value: protects=public job routing, trusted checkout inputs, separate build sources/targets, bounded prerequisites,
// disposable sudo protocol and retained evidence; fails_when=workflow compares wrong input or drops safety/evidence wiring;
// why_new=YAML syntax alone checked no protocol; seam=git/cargo/cp/sudo external tools stubbed inside isolated shell;
// non-testcaller=GitHub Actions consumes this actual workflow; no_claim=GitHub execution, real builds/install/apt or root lifecycle.
test("workflow routes the refactor guard to independent builds and the disposable sudo evidence protocol", () => {
  const workflow = Bun.YAML.parse(readFileSync(join(repo, ".github/workflows/refactor-guard.yml"), "utf8")) as {
    jobs: Record<string, GuardJob>;
  };
  const job = workflow.jobs["rust-install-diff"];
  expect(job.if).toBe("contains(github.event.pull_request.labels.*.name, 'refactor')");
  expect(job["runs-on"]).toBe("ubuntu-latest");
  expect(job["timeout-minutes"]).toBeGreaterThan(0);
  const checkout = job.steps.find((step) => step.uses?.startsWith("actions/checkout@"));
  expect(checkout?.with).toMatchObject({
    ref: "$" + "{{ github.event.pull_request.head.sha }}",
    "fetch-depth": 0,
    "persist-credentials": false,
  });
  const root = join(tmp, "workflow");
  mkdirSync(join(root, "base"), { recursive: true });
  mkdirSync(join(root, "head"));
  const baseCheckout = shell(
    `RUNNER_TEMP=$1 BASE_SHA=base-sha HEAD_SHA=head-sha
git() { if [ "$1" = merge-base ]; then printf '%s\\n' "$*" >&2; printf merged-base; else printf '%s\\n' "$*"; fi; }
${jobRun(job, "base checkout")}`,
    root,
  );
  expect(baseCheckout.exitCode).toBe(0);
  expect(baseCheckout.stdout.toString()).toBe(`worktree add --detach ${root}/base merged-base\n`);
  expect(baseCheckout.stderr.toString()).toBe("merge-base base-sha head-sha\n");
  const builds = shell(
    `RUNNER_TEMP=$1; cd "$1/head"
cargo() { printf 'cargo|%s|%s\\n' "$PWD" "$*"; }; cp() { printf 'cp|%s\\n' "$*"; }
${jobRun(job, "build each checkout into its own target")}`,
    root,
  );
  expect(builds.exitCode).toBe(0);
  expect(builds.stdout.toString().trim().split("\n")).toEqual([
    `cargo|${root}/head|build --release --locked --bin freespace`,
    `cp|target/release/freespace ${root}/bin/head`,
    `cargo|${root}/base|build --release --locked --bin freespace --target-dir ${root}/base-target`,
    `cp|${root}/base-target/release/freespace ${root}/bin/base`,
  ]);
  const invocation = shell(
    `RUNNER_TEMP=$1 GITHUB_ACTIONS=true RUNNER_ENVIRONMENT=github-hosted USER=runner
sudo() { printf '%s\\0' "$@"; }
${jobRun(job, "compare real installation and uninstall, base self-check first")}`,
    root,
  );
  expect(invocation.exitCode).toBe(0);
  expect(invocation.stdout.toString().split("\0").filter(Boolean)).toEqual([
    "env",
    "GITHUB_ACTIONS=true",
    "RUNNER_ENVIRONMENT=github-hosted",
    `RUNNER_TEMP=${root}`,
    "scripts/rust-diff-install.sh",
    "--github-disposable-runner",
    `${root}/bin/base`,
    `${root}/bin/head`,
    `${root}/rust-install-diff`,
    `${root}/install-scratch`,
    "runner",
  ]);
  const upload = job.steps.find((step) => step.uses?.startsWith("actions/upload-artifact@"));
  expect(upload?.if).toBe("always()");
  expect(upload?.with).toMatchObject({ name: "rust-install-diff", path: "$" + "{{ runner.temp }}/rust-install-diff", overwrite: true });
  expect(Number(upload?.with?.["retention-days"])).toBeGreaterThan(0);
});
