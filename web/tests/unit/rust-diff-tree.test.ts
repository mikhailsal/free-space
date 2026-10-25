// The made-up trees of scripts/rust-diff.sh (scripts/rust-diff/tree.sh), at the default width: what CONTRIBUTING says
// they hold, after each change too, and that the listing tree_check compares sees a change.
import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { repo } from "./rust-diff-stub";

let tmp = "";
beforeAll(() => {
  tmp = realpathSync(mkdtempSync(join(tmpdir(), "rust-diff-tree-")));
});
afterAll(() => {
  Bun.spawnSync(["chmod", "-R", "u+rwx", tmp]);
  rmSync(tmp, { recursive: true, force: true });
});

// Value: protects=the made-up trees hold what CONTRIBUTING says they hold, after each change too: every folder its own
//   date, the wide part's 340 folders, hard links inside a folder, across folders and into the folder refresh rewalks,
//   a sparse file, a file and a folder whose names are not UTF-8, a folder nobody may read; after drift_tree a folder
//   that is a file and one under a symlink; and the listing that tree_check compares sees a change of mtime, mode, size
//   or link count; fails_when=settle_dirs gives folders one date (`--sort mtime` then compares only tie-breaks), make_wide
//   loses a level or its links, a blob stops being sparse, drift_tree loses a change, or tree_listing drops a field;
//   why_new=the guard runs use one folder per level and assert answers, not the tree; seam=none; no_claim=what the CLI
//   makes of the tree (the guard runs compare that)
// The tree at the default width, made, changed and drifted: a few seconds.
test("the trees hold distinct folder dates, hard links, a sparse file, odd names, a locked folder and the drift", async () => {
  const tree = join(tmp, "shape");
  const sh = `. "$1/scripts/rust-diff/tree.sh"
dates() { echo "$1 dirs $(find "$tree" -type d 2>/dev/null | wc -l) dates $(find "$tree" -type d -printf '%T@\\n' 2>/dev/null | sort -u | wc -l)"; }
make_tree; dates made
echo "wide $(find "$tree/wide" -mindepth 1 -type d | wc -l)"
echo "iso links $(stat -c %h "$tree/media/iso/debian.iso") $(stat -c %h "$tree/logs/debian.link")"
echo "wide links $(find "$tree/wide" -name 'link*.dat' -links 2 | wc -l)"
echo "sparse $(stat -c '%s %b' "$tree/media/sparse.img")"
[ -e "$tree/odd/$(printf 'bad\\377name.bin')" ] && [ -d "$tree/odd/$(printf 'dir\\376')" ] && echo "odd names"
echo "locked $(stat -c %a "$tree/locked")"
tree_listing > "$tree.listing"
touch -d "2020-01-01 00:00 UTC" "$tree/top.bin"; chmod 600 "$tree/logs/app.log"; truncate -s 1 "$tree/media/edge-new.bin"
ln "$tree/projects/app/src/main.rs" "$tree/projects/app/main.link"
tree_listing | diff "$tree.listing" - | grep '^>' || true
change_tree; dates changed
drift_tree; dates drifted
echo "drifted $(stat -c %F "$tree/empty") $(readlink "$tree/projects/app") $(stat -c %F "$tree/projects/app.real/src")"
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
  expect(out).toContain("\nodd names\n");
  expect(out).toContain("\ndrifted regular file app.real directory\n");
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
}, 60_000);
