// scripts/coverage-functions.sh, the per-function table of refactor-guard's coverage job: lines are mapped and counted
// as `llvm-cov show` marks them (a region starting on the line, else the segment the line starts inside; skipped and gap
// regions start nothing), each line goes to the innermost function spanning it (a span is the function's own code
// regions in its own file, nothing else), copies of a function from two binaries count once, and names come out
// demangled without crate hashes.
import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const repo = realpathSync(join(import.meta.dir, "../../.."));
const script = join(repo, "scripts/coverage-functions.sh");

// Rust v0 names as rustc emits them under -C instrument-coverage; the second is the same function from another binary.
const OUTER = "_RNvNtCsgDdOOZv2MQV_9freespace6status11doctor_hint";
const OUTER_COPY = "_RNvNtCs7HjjErTzRo4_9freespace6status11doctor_hint";
const CLOSURE = "_RNCNvNtCsgDdOOZv2MQV_9freespace6status11doctor_hint0B5_";
const RUN = "_RNvNtCsgDdOOZv2MQV_9freespace6status3run";

const status = "/repo/cli/src/status.rs";
const other = "/repo/cli/src/other.rs";
// A region is [line, col, end line, end col, count, file id, expanded file id, kind]; kind 0 is code, 3 a gap.
const outerRegions = [
  [1, 1, 1, 20, 3, 0, 0, 0],
  [2, 5, 2, 9, 0, 0, 0, 0],
  [3, 1, 3, 9, 7, 1, 0, 0],
  [10, 1, 10, 2, 3, 0, 0, 0],
  [30, 1, 30, 9, 7, 1, 0, 0], // code of another file (a macro's definition): not this function's span
  [31, 1, 31, 9, 7, 0, 0, 1], // an expansion
  [32, 1, 32, 9, 7, 0, 0, 3], // a gap
];
// A segment is [line, col, count, has count, region entry, gap].
const statusSegments = [
  [1, 1, 0, true, true, false], // line 1: two regions start, the larger count wins (covered)
  [1, 10, 0, false, false, false],
  [1, 12, 3, true, true, false],
  [1, 20, 0, false, false, false],
  [2, 5, 0, true, true, false], // line 2: a region that never ran
  [2, 9, 0, false, false, false], // line 3: starts after a segment without a count (not mapped)
  [4, 1, 2, true, true, false], // line 4: the closure
  [5, 3, 2, true, false, false], // line 5: inside the closure's region (covered); its count runs on into line 6
  [6, 1, 0, false, true, false], // line 6: a skipped region starts (cfg'd out): not mapped
  [6, 9, 0, false, false, false],
  [7, 1, 4, true, true, true], // line 7: a gap region starts nothing (not mapped)
  [8, 10, 0, false, false, false], // line 8: starts inside the gap, which has a count (covered)
  [10, 1, 3, true, true, false], // line 9 not mapped; line 10 covered
  [10, 2, 0, false, false, false],
  [30, 1, 7, true, true, false], // lines 30-32: mapped, but in no function's span
  [33, 1, 0, false, false, false],
];
const exported = {
  data: [
    {
      files: [
        { filename: status, segments: statusSegments },
        {
          filename: other,
          segments: [
            [1, 1, 5, true, true, false],
            [1, 9, 0, false, false, false],
          ],
        },
      ],
      functions: [
        { name: OUTER, filenames: [status], regions: outerRegions },
        { name: OUTER_COPY, filenames: [status], regions: outerRegions.map((r) => r.map((v, i) => (i === 4 ? 0 : v))) },
        { name: CLOSURE, filenames: [status], regions: [[4, 1, 5, 3, 2, 0, 0, 0]] },
        { name: "gap_only", filenames: [status], regions: [[20, 1, 20, 5, 0, 0, 0, 3]] },
        { name: RUN, filenames: [other], regions: [[1, 1, 1, 9, 5, 0, 0, 0]] },
      ],
    },
  ],
};

let tmp = "";
let json = "";

beforeAll(() => {
  tmp = realpathSync(mkdtempSync(join(tmpdir(), "coverage-functions-")));
  json = join(tmp, "coverage.json");
  writeFileSync(json, JSON.stringify(exported));
});

afterAll(() => {
  rmSync(tmp, { recursive: true, force: true });
});

const run = (...args: string[]) => Bun.spawnSync([script, ...args], { stdout: "pipe", stderr: "pipe" });

// Value: protects=a function's span is its own file's code regions only: code of another file, an expansion and a gap
//   lying outside the code do not stretch it; fails_when=`.[5] == 0` or `.[7] == 0` is dropped or loosened (the
//   doctor_hint row grows from 4 lines, 3 covered to 5-7 and 4-6); why_new=every excluded region sat inside the span, so
//   dropping a filter changed no row; seam=none; no_claim=real llvm-cov exports (a hand-made JSON of the same shape)
test("one row per function of the files asked for: lines and covered lines as llvm-cov counts them", () => {
  const r = run(json, "cli/src/status.rs", "cli/src/other.rs", "cli/src/missing.rs");
  expect(r.stderr.toString()).toBe("");
  expect(r.exitCode).toBe(0);
  expect(r.stdout.toString()).toBe(
    [
      "cli/src/other.rs\t1\tfreespace::status::run\t1\t1",
      "cli/src/status.rs\t1\tfreespace::status::doctor_hint\t4\t3",
      "cli/src/status.rs\t4\tfreespace::status::doctor_hint::{closure#0}\t2\t2",
      "",
    ].join("\n"),
  );
});

test("a file is matched by its whole path or a whole trailing part, never by a longer name", () => {
  expect(run(json, status).stdout.toString()).toContain("/repo/cli/src/status.rs\t1\tfreespace::status::doctor_hint\t4\t3\n");
  expect(run(json, "rc/status.rs").stdout.toString()).toBe("");
});

test("without a file to report: usage on stderr, exit 2", () => {
  const r = run(json);
  expect(r.exitCode).toBe(2);
  expect(r.stderr.toString()).toContain("usage:");
});

// A real export: `cargo llvm-cov --json --output-path llvm-cov.json` in fixtures/covprobe (cargo-llvm-cov 0.9.1), its
// absolute path then replaced by /src/covprobe. The crate has a closure on its parent's first line, a nested fn, a
// generic function run for two types, a macro and a function nothing calls.
const probe = join(import.meta.dir, "fixtures/covprobe/llvm-cov.json");

// Value: protects=on a real export the rows give each line `llvm-cov show` marks once (29 lines, 24 run, as its text
//   view of the crate shows), the closure keeps its first line from its parent, both copies of the generic share one
//   row, named by the least mangled; fails_when=the mapping rules drift from llvm-cov's on segment shapes rustc really
//   emits, or the per-function count of the export's summary is taken for the file total; why_new=the tests above read
//   a JSON written by hand; seam=none; no_claim=other llvm-cov versions than the one that made the fixture
test("on a real export: each line llvm-cov show marks counts once, under the innermost function", () => {
  const r = run(probe, "src/lib.rs");
  expect(r.stderr.toString()).toBe("");
  expect(r.exitCode).toBe(0);
  const rows = r.stdout.toString();
  expect(rows).toBe(
    [
      "src/lib.rs\t1\tcovprobe::sum_even\t3\t3",
      "src/lib.rs\t2\tcovprobe::sum_even::{closure#0}\t5\t5",
      "src/lib.rs\t12\tcovprobe::wrap::<&str>\t4\t4",
      "src/lib.rs\t13\tcovprobe::wrap::helper\t3\t3",
      "src/lib.rs\t20\tcovprobe::never_called\t5\t0",
      "src/lib.rs\t33\tcovprobe::uses_macro\t3\t3",
      "src/lib.rs\t41\tcovprobe::tests::t\t6\t6",
      "",
    ].join("\n"),
  );
  // The export's own file total counts per function, so there the closure's first line counts twice: 30 and 25.
  const summary = JSON.parse(readFileSync(probe, "utf8")).data[0].files[0].summary.lines;
  expect([summary.count, summary.covered]).toEqual([30, 25]);
});
