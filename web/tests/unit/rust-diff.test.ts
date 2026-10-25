// scripts/rust-diff.sh, the rust-diff job of refactor-guard, on stub builds (rust-diff-stub.ts): answers that differ
// only in time fields, live numbers, the snapshot id and the build's own version match after normalization (D20); one
// changed data byte in head fails with the difference in every phase; a base that answers two runs differently stops the
// job as unreliable before head runs; a base that no longer answers as the harness expects, or whose server does not
// start or answer, stops it as stale. A question the CLI refuses is part of the answer too: its exit status and its
// stderr are compared like its stdout, and so are the files each phase leaves (names, types, modes, databases); a call
// that hangs answers exit 124; a build that changes the tree it reads stops the job. How the job starts and stops (bad
// input, a missing tool, root, signals) is in rust-diff-process.test.ts; the made-up trees in rust-diff-tree.test.ts.
import { afterAll, beforeAll, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync } from "node:fs";
import { availableParallelism, tmpdir } from "node:os";
import { join } from "node:path";
import { guard, type Run, stub, writeStub } from "./rust-diff-stub";

// One pass of the questions is about 310 stub calls and 29 requests, each normalized, and a full run makes three (base,
// base again, head). The fourteen runs start together in beforeAll, four of them full, the others stopped after one or
// two passes: about 2.5 min in all at nice 19 on 8 threads, 3 min beside another job, and longer in proportion on fewer
// threads (GitHub's runner has 4), so the bound grows as they shrink. afterAll waits for them all, so it gets the same
// bound: a filtered run (-t) must not leave tmp behind.
const RUN_MS = Math.max(300_000, Math.ceil(2_400_000 / availableParallelism()));
let tmp = "";
type RunName =
  | "same"
  | "byte"
  | "flaky"
  | "other"
  | "unlike"
  | "stale"
  | "noserve"
  | "deadserve"
  | "tamper"
  | "retouch"
  | "shrink"
  | "midstart"
  | "midscan"
  | "midrefresh";
const pending = {} as Record<RunName, Promise<Run>>;
const read = (r: Run, path: string) => readFileSync(join(r.out, path), "utf8");
// the part of TEXT from the line FROM to the line TO (or to its end)
const section = (text: string, from: string, to?: string) => {
  const start = text.indexOf(`${from}\n`);
  if (start < 0) return "";
  const end = to === undefined ? -1 : text.indexOf(`${to}\n`, start);
  return end < 0 ? text.slice(start) : text.slice(start, end);
};

beforeAll(() => {
  tmp = realpathSync(mkdtempSync(join(tmpdir(), "rust-diff-")));
  const write = (name: string, body: string) => writeStub(tmp, name, body);
  write("base", stub());
  write("same", stub({ version: "0.2.0" })); // a release bump alone is no difference
  write("byte", stub({ entries: 4 }));
  // its own pid in every --json answer
  write("flaky", stub({ sh: 'bound="$bound,\\"pid\\":$$"' }));
  // a head that refuses /nope with another status and other words, whose server lets other pages frame it, and whose
  // gui writes its redirect file for everyone to read and exits 3 on TERM: five differences in other lines of the
  // answers, one run for all
  const others = "FRAME=SAMEORIGIN GUI_MODE=644 GUI_TERM_EXIT=3; export FRAME GUI_MODE GUI_TERM_EXIT";
  write("other", stub({ refusal: { exit: 5, message: "nothing at" }, early: others }));
  // differences no mask may hide, in one head so the suite pays for one more run, not seven: the space doctor says the
  // next scan needs, a count in state.json, a wider mode and a stray file in the snapshot folder, a file in $HOME and
  // one in the unit dir, an index more in snapshot.db, and the first `diff @` (text, of the fixture snapshot) hanging
  const need = '[ "$1" != doctor ] || need="1.0 G"';
  const leak = '[ ! -f "$dir/id" ] || { chmod 666 "$dir/id"; : > "$dir/leftover.tmp"; }';
  const home = '[ "$1" != status ] || { : > "$HOME/.leak"; : > "$FREESPACE_UNIT_DIR/x.service"; }';
  const hang = '[ "$1 $#" != "diff 2" ] || case "$dir $2" in */snap" "*/tree) exec sleep 30 ;; esac';
  const index = 'SCHEMA_EXTRA="CREATE INDEX IF NOT EXISTS calls_command ON calls(command);"';
  write("unlike", stub({ early: index, sh: [need, "fails=1", leak, home, hang].join("\n") }));
  // a base whose CLI has moved on from the harness: a flag it no longer takes, a refusal it no longer makes, one it
  // makes for another reason, a --json answer that is not JSON, and a server that takes a POST
  const moved = [
    `case " $* " in *" --min-size "*) echo "error: unexpected argument '--min-size'" >&2; exit 2 ;; *" install-timer --no-start"*) exit 0 ;; esac`,
    `case "$*" in "ls "*/nope*) echo "error: unexpected argument" >&2; exit 2 ;; "doctor --json") exit 0 ;; esac`,
    "ALLOW_POST=1; export ALLOW_POST",
  ];
  write("stale", stub({ early: moved.join("\n") }));
  write("noserve", stub({ early: '[ "$1" != serve ] || exit 1' }));
  // a server that prints an address nobody answers on (port 9, closed), and stays up until the TERM
  const dead = `echo "freespace serve: http://127.0.0.1:9/#t=${"ab".repeat(32)}" >&2; exec sleep 60`;
  write("deadserve", stub({ early: `[ "$1" != serve ] || { ${dead}; }` }));
  // builds that change the tree they read, each in another phase: a head links a file of the fixture tree; a base dates
  // back and narrows a file of its own tree after its second scan; a base cuts a file in the config phase; a base
  // narrows a file of the fixture tree during a held scan, dates back one of its own tree in its first scan, and leaves
  // a file in its own tree as it refreshes
  write("tamper", stub({ early: '[ "$1" != ls ] || [ ! -f "$2/top.bin" ] || ln -f "$2/top.bin" "$2/.top.link"' }));
  write(
    "retouch",
    stub({ early: 'case "$1 $2" in "ls "*/ftree) touch -d "2020-01-01 00:00 UTC" "$2/top.bin"; chmod 600 "$2/top.bin" ;; esac' }),
  );
  write(
    "shrink",
    stub({ early: '[ ! -f "$FREESPACE_CONFIG" ] || case "$1 $2" in "ls "*/projects) truncate -s 1 "$2/app/src/main.rs" ;; esac' }),
  );
  write("midstart", stub({ early: '[ "$1" != ls ] || case "$dir" in */busy) chmod 640 "$2/top.bin" ;; esac' }));
  write("midscan", stub({ early: 'case "$1 $2" in "scan "*/ftree) touch -d "2020-01-01 00:00 UTC" "$2/top.bin" ;; esac' }));
  write("midrefresh", stub({ early: `case "$1 $2" in "refresh "*/ftree/logs) : > "\${2%/logs}/stray" ;; esac` }));
  const runs: [RunName, string, string, Record<string, string>?, { tty?: boolean }?][] = [
    ["same", "base", "same"],
    ["byte", "base", "byte"],
    ["flaky", "flaky", "same"],
    ["other", "base", "other"],
    // a hang waits out this bound, not the stub's 30 s
    ["unlike", "base", "unlike", { RUST_DIFF_TIMEOUT: "15" }],
    // on a terminal, as a run from a shell is: no call may take it (gui would start, not refuse: 15 s, then exit 124)
    ["stale", "stale", "base", { RUST_DIFF_TIMEOUT: "15" }, { tty: true }],
    ["noserve", "noserve", "base"],
    ["deadserve", "deadserve", "base"],
    ["tamper", "base", "tamper"],
    ["retouch", "retouch", "base"],
    ["shrink", "shrink", "base"],
    ["midstart", "midstart", "base"],
    ["midscan", "midscan", "base"],
    ["midrefresh", "midrefresh", "base"],
  ];
  for (const [name, base, head, env, how] of runs) pending[name] = guard(tmp, base, head, env, how);
});

afterAll(async () => {
  await Promise.allSettled(Object.values(pending));
  rmSync(tmp, { recursive: true, force: true });
}, RUN_MS);

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
    for (const key of live) expect(fixture, key).toMatch(new RegExp(`"${key}": "<(number|string)>"`));
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
  },
  RUN_MS,
);

// Value: protects=a question the CLI refuses is answered as it is: exit status and plain-text stderr recorded, in the
//   --json form too, and the run goes on; fails_when=`|| code=$?` or the `|| cat` fallback of normalize_json is dropped
//   (the job aborts at ls @/nope) or the status is not written; why_new=the stub never refused anything, so no test
//   ran those paths; seam=none; no_claim=the real CLI's wording and codes (a stub refuses /nope as the CLI does)
test(
  "a question the CLI refuses is answered with its status and stderr, also under --json, and the run goes on",
  async () => {
    const r = await pending.same;
    expect(r.code).toBe(0);
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
      "gui",
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
    // gui without a terminal is refused; on one, rust-diff/gui.sh asks it
    expect(fixture).toContain("== gui\nexit 2\n-- stdout\n-- stderr\nfreespace: gui needs a terminal and a browser\n");
  },
  RUN_MS,
);

// Value: protects=refresh is exercised: each build refreshes its own copy of the fixture snapshot (one path, so the
//   answers read the same), nine REFRESHES in a row, and the questions after them come from the copy, with the new
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
    // QUERIES ask before any refresh, AFTER_REFRESH after the four that work (five are refused)
    for (const ask of ["ls @", "status"]) expect(counts(ask, "\\nrefreshed"), ask).toEqual(["0", "4"]);
    expect(counts("ls @ --json", '"refreshed":'), "json").toEqual(["0", "4"]);
    expect(fixture).toContain("== refresh @/logs --workers 2\nexit 0\n-- stdout\nargs refresh <work>/tree/logs --workers 2\n");
    expect(fixture).toContain("== refresh @/cache --workers 2 --json\nexit 0\n");
    expect(fixture).toContain("== refresh @/nope --workers 2\nexit 2\n");
    expect(fixture).toContain("== refresh @/top.bin --workers 2 --json\nexit 2\n");
    expect(fixture).toContain("== refresh @/wide/d1 --workers 4\nexit 0\n");
    // a folder that is a file on disk now, one under a symlink now, a symlink: refused; then a changed folder
    expect(fixture).toContain(
      "== refresh @/empty --workers 2\nexit 2\n-- stdout\n-- stderr\nfreespace: <work>/tree/empty is no longer a folder on disk\n",
    );
    expect(fixture).toContain("== refresh @/projects/app/src --workers 2 --json\nexit 2\n");
    expect(fixture).toContain("== refresh @/projects/media-link --workers 2\nexit 2\n");
    expect(fixture).toContain("== refresh @/projects --workers 2 --json\nexit 0\n");
    // an id of a snapshot is 19 digits here: none is left in any answer (a path in hex, as --path-raw and gui's raw=
    // take it, may hold 19 digits in a row, among other hex digits)
    for (const text of [fixture, fresh]) expect(text).not.toMatch(/(?<![0-9a-f])\d{19}(?![0-9a-f])/);
    // the fresh phase refreshes the snapshot the build made itself, of its own tree
    expect(fresh).toContain("== refresh @/logs --workers 2\nexit 0\n-- stdout\nargs refresh <work>/ftree/logs --workers 2\n");
    // each scan and refresh walks the tree as the phase left it: the first scan before change_tree, the second after
    // it (cache/ came), the refreshes after drift_tree (old.log and cache/ went)
    const sees = (text: string) =>
      [...text.matchAll(/^== ((?:scan|refresh) \S+)[^\n]*\n(?:(?!== )[^\n]*\n)*?sees (.*)$/gm)].map((m) => `${m[1]}: ${m[2]}`);
    const refreshed = [
      "refresh @/logs: app.log debian.link new.log ",
      "refresh @/cache: ",
      "refresh @/wide/d1: d1 ",
      "refresh @/projects: app app.real media-link ",
    ];
    const root = "empty locked logs media odd projects top.bin wide ";
    expect(sees(fresh)).toEqual([`scan @: ${root}`, `scan @: cache ${root}`, ...refreshed]);
    expect(sees(fixture)).toEqual(refreshed);
  },
  RUN_MS,
);

// Value: protects=a difference in any phase fails the job and shows under that phase's file; fails_when=same() stops
//   at the first phase that differs, or a phase leaves PHASES; why_new=no test tied a difference to the start, fresh or
//   config phase (state-machine review of A0); seam=none; no_claim=which data a real head changes
test(
  "one changed data byte in head fails with the difference on stdout, in every phase",
  async () => {
    const r = await pending.byte;
    expect(r.code).toBe(1);
    for (const phase of ["fixture", "start", "fresh", "config"]) expect(r.stdout, phase).toContain(`\n+++ head/${phase}.txt\n`);
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
//   file, another count in state.json or another schema in snapshot.db fails the job; fails_when=files() is dropped,
//   lists names only, skips state.json or the databases, or kept() lists only the snapshot folder; why_new=only the
//   answers were compared, and a head that opened the folder to everyone passed (adversarial review), and a refactor
//   of the writer could change the schema while every answer stays (Red Team of A0); seam=none; no_claim=the real
//   CLI's files, modes and schema
test(
  "a head that leaves a stray file, a wider mode or another schema in the snapshot folder, $HOME or the unit dir fails",
  async () => {
    const r = await pending.unlike;
    expect(r.code).toBe(1);
    expect(r.stdout).toMatch(/^\+\.\/id f 666$/m);
    expect(r.stdout).toMatch(/^\+\.\/leftover\.tmp f \d+$/m);
    expect(r.stdout).toMatch(/^\+\.\/\.leak f \d+$/m);
    expect(r.stdout).toMatch(/^\+\.\/x\.service f \d+$/m);
    expect(r.stdout).toContain('\n-  "consecutive_failures": 0\n+  "consecutive_failures": 1\n');
    expect(r.stdout).toContain("\n+CREATE INDEX calls_command ON calls(command);\n");
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

// Value: protects=the floor: base must give the exit status the harness expects for every question (0 unless EXPECTED
//   lists another), a refusal with the words EXPECTED lists for it, a --json answer of 0 in JSON, and each request to
//   serve its HTTP status, or the job stops as stale (exit 4) after base's first run, naming each question, before base
//   again or head run; fails_when=floor() is dropped or not called from ask or http, the EXPECTED lookup takes the wrong
//   phase, the words or the JSON go unchecked, the check runs after head, or a call takes the job's stdin (this run has
//   a terminal there: gui would start instead of refusing, then answer 124); why_new=a flag both builds refuse alike
//   compared equal and tested nothing (adversarial review of PR 2: 10 exit 2 blocks, job green), exit 2 stands for a
//   usage error as much as a refusal (adversarial review of A0), and every run had no terminal (red team of A0);
//   seam=script(1) for the terminal; no_claim=which flags a future CLI renames
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
    // the gui question refused, though the job ran on a terminal
    expect(r.stderr).not.toContain("once: gui:");
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

// Value: protects=no command may change the tree it reads: a build that links, dates back, narrows, cuts or adds a
//   file of the tree stops the job at once, with the listing's difference and the phase, at each of the six checks (the
//   fixture phase, the start phase, the fresh phase's first scan, its questions, its refreshes, the config phase);
//   fails_when=tree_check is dropped from one of them or the listing leaves out names, modes, sizes, links or mtimes;
//   why_new=only the snapshot folder was listed (freespace never changes what it scans, CONTRIBUTING), and three of the
//   six checks were tried (testing review of A0); seam=none; no_claim=changes the listing does not see (access and
//   change times)
test(
  "a build that changes the tree it reads stops the job with the change and the phase",
  async () => {
    for (const [name, bin, phase, change] of [
      ["tamper", "head", "the fixture phase", /^\+\.top\.link f \d+ 102400 2 [\d.]+$/m],
      ["retouch", "base", "its second scan and the questions", /^\+top\.bin f 600 102400 1 1577836800\.0+$/m],
      ["shrink", "base", "the config phase", /^\+projects\/app\.real\/src\/main\.rs f \d+ 1 1 [\d.]+$/m],
      ["midstart", "base", "the start phase", /^\+top\.bin f 640 102400 1 [\d.]+$/m],
      ["midscan", "base", "its first scan", /^\+top\.bin f \d+ 102400 1 1577836800\.0+$/m],
      ["midrefresh", "base", "its refreshes", /^\+stray f \d+ 0 1 [\d.]+$/m],
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
//   headers only, a HEAD no body, the addresses it listens on are listed, and the server stops on TERM; port and token
//   are masked; the refusals take a token one character longer, shorter or changed, a foreign or cross-site caller,
//   another host name (for the page too) and a POST, and localhost and the server's own origin pass; fails_when=the
//   serve phase is dropped, the token is not sent, a header leaves the comparison, the page's bytes enter it (they move
//   with every web build), or the listening check goes; why_new=the HTTP API was not compared at all (D11), and the
//   rules that pass, Sec-Fetch-Site, HEAD and the bind address were not asked (Red Team of A0); seam=none; no_claim=the
//   real server's rules (a stub serves them)
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
      ["a longer token", 403],
      ["a token one character short", 403],
      ["a token with its last character changed", 403],
      ["a foreign origin", 403],
      ["a cross-site fetch", 403],
      ["a same-site fetch", 403],
      ["another host name", 403],
      ["another host name for the page", 403],
      ["a POST", 405],
      ["the host name localhost", 200],
      ["its own origin", 200],
      ["the origin localhost", 200],
      ["a HEAD", 200],
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
    expect(fixture).toContain("== serve: a HEAD\nhttp 200\n-- headers\ncache-control: no-store\ncontent-type: application/json\n");
    expect(fixture).toContain("-- body\n(none: a HEAD)\n== serve: the page\n");
    expect(fixture).toContain("== serve listens on\ntcp 127.0.0.1\n== serve stopped by TERM\n");
    expect(fixture).toContain(
      "== serve stopped by TERM\nexit 0\n-- stdout\n-- stderr\nfreespace serve: http://127.0.0.1:<port>/#t=<token>\npress Ctrl-C to stop\n",
    );
    expect(fixture).not.toMatch(/^(date|content-length):/m);
  },
  RUN_MS,
);

// Value: protects=a header the server answers differently fails the job, and so does a gui whose redirect file others
//   may read, or that stops on TERM with another exit status; fails_when=headers leave the serve answers, the runtime
//   folder is not listed, or TERM goes to script(1), which then exits 0 whatever gui does; why_new=the HTTP API was not
//   compared, and gui was not asked at all; seam=none; no_claim=the real server's headers, file modes and exit status
test(
  "a head whose server answers with another header, or whose gui writes its token for everyone or stops otherwise, fails",
  async () => {
    const r = await pending.other;
    expect(r.code).toBe(1);
    expect(r.stdout).toContain("-x-frame-options: DENY\n+x-frame-options: SAMEORIGIN\n");
    // past the 200 lines of difference the job prints for fixture.txt: the answers themselves
    const file = (side: string) => section(read(r, `${side}/fixture.txt`), "== gui here, a browser opens", "-- xdg-open got");
    expect(file("base")).toContain("./freespace-gui-<pid>.html f 600\n");
    expect(file("head")).toContain("./freespace-gui-<pid>.html f 644\n");
    for (const mode of ["over ssh", "here, a browser opens", "here, no browser"]) {
      const stop = (side: string) => section(read(r, `${side}/fixture.txt`), `== gui ${mode} stopped by TERM`, "-- stdout");
      expect(stop("base"), mode).toBe(`== gui ${mode} stopped by TERM\nexit 0\n`);
      expect(stop("head"), mode).toBe(`== gui ${mode} stopped by TERM\nexit 3\n`);
    }
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
//   and for diff without a previous snapshot, a refresh of a root; once a root is gone from the disk, its refresh and
//   the scan refused and status asked; then a config with an unknown key refused; and each part lists what the CLI
//   keeps in $HOME and the unit dir; fails_when=$config does not reach FREESPACE_CONFIG, the phase or its gone part is
//   dropped, or kept() lists only the snapshot folder; why_new=no answer read a config file, nothing looked outside the
//   snapshot folder (D11), and no refresh met a root or a root that is gone (Red Team of A0); seam=none; no_claim=the
//   real config's keys and rules (a stub refuses as the CLI does)
test(
  "a config file's roots are scanned, refreshed and asked, a root that goes refused, an unknown key refused",
  async () => {
    const r = await pending.same;
    const config = readFileSync(join(r.out, "head/config.txt"), "utf8");
    expect(config).toContain("== scan --json\nexit 0\n");
    expect(config).toContain("== diff @/media\nexit 3\n");
    expect(config).toContain("== find @ --name *.iso\nexit 2\n-- stdout\n-- stderr\nfreespace: outside the scanned roots\n");
    expect(config).toContain("== find @/media --name *.iso\nexit 0\n");
    expect(config).toContain("== status\nexit 2\n-- stdout\n-- stderr\nfreespace: unknown field keep_old\n");
    expect(config).toContain("== refresh @/projects --workers 2 --json\nexit 0\n");
    const gone = section(config, "== refresh @/media --workers 2", "== files in <work>/conf");
    expect(gone).toContain("exit 2\n-- stdout\n-- stderr\nfreespace: root <work>/ftree/media no longer exists; run a full scan\n");
    expect(gone).toContain("== scan\nexit 2\n-- stdout\n-- stderr\nfreespace: scan failed: No such file or directory\n");
    expect(gone).toContain("== status\nexit 0\n");
    for (const dir of ["home", "units"]) expect(config).toMatch(new RegExp(`^== files in <work>/${dir}\\n\\. d \\d+\\n`, "m"));
  },
  RUN_MS,
);

// Value: protects=the start phase: with no snapshot folder each question is refused as the CLI refuses it, refresh too,
//   and the folder stays missing; while a scan holds the lock, the first scan's questions say so, a scan and a refresh
//   are refused and the lock file is all the folder holds; over the fixture snapshot status answers while scan and
//   refresh are refused; fails_when=the phase is dropped, the lock is not held while the questions run, or files()
//   fails on a folder that is not there; why_new=no answer came before the first snapshot or during a scan
//   (adversarial review of A0), and no scan or refresh was asked then (state-machine review of A0); seam=none;
//   no_claim=the real CLI's lock rules (a stub probes the lock)
test(
  "before the first snapshot and while a scan holds the lock, each question is asked",
  async () => {
    const r = await pending.same;
    const start = readFileSync(join(r.out, "head/start.txt"), "utf8");
    const refused = (q: string, why: string) => `== ${q}\nexit 8\n-- stdout\n-- stderr\nfreespace: no snapshot in <work>/${why}\n`;
    expect(start).toContain(refused("status", "empty"));
    expect(start).toContain(refused("refresh @/logs --workers 2", "empty"));
    expect(start).toContain("== files in <work>/empty\n(missing)\n");
    const running = "exit 6\n-- stdout\n-- stderr\nfreespace: another scan is running\n";
    const first = section(start, "== files in <work>/empty", "== files in <work>/first");
    for (const q of ["scan @ --workers 2", "refresh @/logs --workers 2 --json"]) expect(first, q).toContain(`== ${q}\n${running}`);
    // the refused scan and refresh write nothing: the lock the harness holds, and the visits the stub's status counts
    expect(start).toMatch(/^== files in <work>\/first\n\. d \d+\n\.\/\.lock f \d+\n\.\/visits f \d+\n== files in <work>\/home\n/m);
    const busy = section(start, "== files in <work>/first");
    for (const q of ["scan @ --workers 2", "refresh @/logs --workers 2"]) expect(busy, q).toContain(`== ${q}\n${running}`);
    expect(start).toContain(refused("ls @", "first yet: a scan running since 00:00, wait"));
    expect(start).toMatch(/^== status\nexit 0\n-- stdout\nargs status\n[^=]*\nheld true\n/m);
    expect(start).toMatch(/^== files in <work>\/busy\n\. d \d+\n\.\/\.lock f \d+\n\.\/id f \d+\n/m);
  },
  RUN_MS,
);

// Value: protects=a base whose server prints an address nobody answers on stops the job as stale, with curl's own
//   words in the answer; fails_when=the `|| code="curl failed..."` fallback goes (set -e then ends the job at the first
//   request, with no message), or curl's error leaves the answer; why_new=every stub's server answered (testing review
//   of A0); seam=none; no_claim=why a real server stops answering
test(
  "a base whose server prints an address nobody answers on stops the job as stale",
  async () => {
    const r = await pending.deadserve;
    expect(r.code).toBe(4);
    expect(r.stderr).toMatch(/^serve: status: answered curl failed: curl: \(7\) .+, the harness expects 200$/m);
    expect(read(r, "base/fixture.txt")).toContain("== serve: status\nhttp curl failed: curl: (7) ");
    expect(readdirSync(r.out)).toEqual(["base"]);
  },
  RUN_MS,
);

// Value: protects=gui is asked on a terminal: over ssh it prints the tunnel lines and serves; here it writes its token
//   into a private redirect file in XDG_RUNTIME_DIR, hands that file to xdg-open, says so when xdg-open fails, and the
//   first request with the token consumes the file; each stops on TERM with its own exit status; the folder's fragment
//   is in the address in its three forms (none, a path, raw hex); fails_when=the gui phase is dropped, it runs without
//   a terminal, the runtime folder or the redirect file goes unlisted, or the token is not found (the redirect file
//   holds the address twice, as the real one does); why_new=gui was not compared at all, and A4 splits cli/src/serve/gui.rs (Red Team of A0);
//   seam=xdg-open on PATH; no_claim=a real browser, and gui's 10 s and 60 s timers (no answer waits for them)
test(
  "gui is asked on a terminal: over ssh, with a browser that opens and with one that fails",
  async () => {
    const r = await pending.same;
    const fixture = read(r, "head/fixture.txt");
    const ssh = section(fixture, "== gui over ssh", "== gui here, a browser opens");
    expect(ssh).toContain("== gui over ssh: status\nhttp 200\n");
    expect(ssh).toContain("== gui over ssh listens on\ntcp 127.0.0.1\n");
    expect(ssh).toMatch(
      /== gui over ssh stopped by TERM\nexit 0\n-- stdout\nfreespace gui is running on .+, port <port>\. Open:\n {2}http:\/\/127\.0\.0\.1:<port>\/#t=<token>\n/,
    );
    const opens = section(fixture, "== gui here, a browser opens", "== gui here, no browser");
    expect(opens).toContain(
      "== files in <work>/runtime\n. d 700\n./freespace-gui-<pid>.html f 600\n-- xdg-open got\n<work>/runtime/freespace-gui-<pid>.html\n" +
        '-- the redirect file\n<meta http-equiv="refresh" content="0;url=http://127.0.0.1:<port>/#<work>/tree/media?t=<token>">' +
        '<a href="http://127.0.0.1:<port>/#<work>/tree/media?t=<token>">Open freespace</a>\n',
    );
    // the request with the token consumed the redirect file
    expect(opens).toMatch(/== gui here, a browser opens: status\nhttp 200\n[\s\S]*\n== files in <work>\/runtime\n\. d 700\n== gui here/);
    expect(opens).toContain("stopped by TERM\nexit 0\n-- stdout\nfreespace gui on http://127.0.0.1:<port>/ · press Ctrl-C to stop\n");
    const fails = section(fixture, "== gui here, no browser", "== files in <work>/snap");
    expect(fails).toMatch(/\nno browser opened; open http:\/\/127\.0\.0\.1:<port>\/#\/\?raw=[0-9a-f]+&t=<token>\n/);
    expect(fails).toContain("== gui here, no browser: status\nhttp 200\n");
  },
  RUN_MS,
);

// Value: protects=name globs are asked (`?`, classes, a negated class, an escape, icase, a lone `[`, a lone `\`
//   refused), and paths as a person types them: from inside the tree (`.`, `..`, a symlink, a name not in the snapshot),
//   as given (a symlink, `..`, a name that is not UTF-8, --path-raw bad and outside the roots), and the tree's own folder
//   as --path-raw hex; fails_when=a list leaves its phase, the CWD questions run outside the tree, or the hex question
//   goes; why_new=only `*` was asked, and every path was absolute (Red Team of A0); seam=none; no_claim=how the real CLI
//   matches and resolves (a stub echoes the words)
test(
  "name globs and paths as a person types them are asked: from inside the tree, as given, and as hex",
  async () => {
    const r = await pending.same;
    const fixture = read(r, "head/fixture.txt");
    for (const q of ["find @ --name deb?an.iso", "find @ --name [!a-m]*.log", "find @ --name edge\\-*.bin", "find @ --name ["]) {
      for (const form of ["", " --json"]) expect(fixture, q).toContain(`== ${q}${form}\nexit 0\n`);
    }
    expect(fixture).toContain(
      "== find @ --name x\\\nexit 2\n-- stdout\n-- stderr\nfreespace: bad --name pattern: pattern ends with a lone backslash\n",
    );
    const cwd = section(fixture, "== in @/projects", "== ls @/projects/media-link");
    expect(cwd).toContain("== ls ..\nexit 0\n-- stdout\nargs ls ..\n");
    expect(cwd).toContain("== ls nope\nexit 2\n");
    // the text answers of the six that answer, and none outside them, ran in the tree
    expect(cwd.match(/^cwd <work>\/tree\/projects$/gm)).toHaveLength(6);
    expect(fixture.match(/^cwd /gm)).toHaveLength(6);
    expect(fixture).toContain("== ls @/odd/dir�\nexit 0\n");
    expect(fixture).toContain("== ls --path-raw zz --json\nexit 2\n");
    expect(fixture).toContain("== ls --path-raw 2f6e6f7065\nexit 2\n");
    expect(fixture).toMatch(/^== ls --path-raw [0-9a-f]{20,}\nexit 0\n/m);
  },
  RUN_MS,
);

// Value: protects=each database in a folder the CLI keeps is listed by schema, user_version and rows per table, read
//   only, after each part; fails_when=db_of() is dropped or opens the database for writing (a journal would then show in
//   the next listing); why_new=only names, types and modes were listed (Red Team of A0); seam=none; no_claim=the real
//   snapshot.db's tables (a stub keeps one)
test(
  "each database the CLI keeps is listed: schema, user_version and rows per table",
  async () => {
    const r = await pending.same;
    const fresh = read(r, "head/fresh.txt");
    const db = (rows: number) =>
      `-- snapshot.db\nCREATE TABLE calls(id INTEGER PRIMARY KEY, command TEXT);\nuser_version 0\ncalls rows ${rows}\n`;
    // two scans, then four refreshes that work
    expect(fresh).toContain(db(2));
    expect(fresh).toContain(db(6));
    expect(fresh).not.toMatch(/-journal /);
  },
  RUN_MS,
);
