# V1 leaf fast-path review

Provider Claude Code; requested/reported claude-opus-5-5. Review input was
leaf source/diff through commit3f3a057 plus full writer/tree/visitor context.
No repository tools, no performance claims. Later test additions are parent
dispositions, not independently re-reviewed output.

# Review: v1 leaf fast path (`core/src/scan/staged/write.rs`)

**Verdict: no P1 or P2 correctness regressions found.** There is one P3 test-coverage note. I checked this only against the supplied diff and source. The `entries` schema column order and the `staged.rs` fixture were not provided, so the assumptions below depend on them.

## Why the change preserves output

**1. Skipping the SELECT cannot drop rows silently.**
- `write.rs` `write_root` now gates `next_child` on `parent.kind == Kind::Dir`. The existing staged visitor already uses the same gate (`is_directory: raw.node.kind() == Kind::Dir`), so this is an established traversal invariant, not a new one.
- If a non-Dir raw row ever did have children, they would no longer be visited. `write_inner` would then fail on `next_id != self.count` with "staging traversal did not visit every entry", and `write()` removes the partial file. The failure is visible, never a wrong snapshot.
- Preorder IDs are unchanged. Children are only skipped where none exist, so `next_id` advances exactly as before.
- The loop runs the same number of times, since every frame still pops in its own iteration. `guard.tick()` cadence is therefore unchanged.

**2. The skipped UPDATE for a leaf wrote the same values as its INSERT.** For a non-Dir frame, `agg` is `SubtreeAgg::leaf(...)` computed in `enter` after `attribute_link`, and nothing merges into it. So the old UPDATE would have written:

| Column | UPDATE value (skipped) | INSERT value |
|---|---|---|
| allocated / apparent | `agg.allocated` / `agg.apparent` = `node.*` after ALIAS zeroing | `node.allocated` / `node.apparent` after `attribute_link` |
| items | 0 | literal `0` |
| errors | `node.errors as u64` | `node.errors` |
| min_scanned_at | `scanned_at` | `?12` (`scanned_at`) |
| newest_mtime | `agg.stored_newest_mtime()` | `?13`, the same expression |
| subtree_end | `id + 0` | `?1` (`id`) |

- This assumes positional INSERT `(…,?10,0,?11,?12,?1,?12,?13)` maps to `nlink, items, errors, scanned_at, subtree_end, min_scanned_at, newest_mtime`. The schema was not supplied. The new `invalid_leaves` query in the test checks this mapping directly, so a mismatch would fail the test.
- The `errors` column cannot differ by saturation. `node.errors` is a `u32`, so widening it to `u64` and storing it back is lossless.

**3. Parent merging and saturation are unchanged.**
- `parent.agg.merge(&frame.agg)` and the `store`/`of` round trip still run for every popped frame.
- The ALIAS zeroing in `attribute_link` still happens before `SubtreeAgg::leaf`, so parent totals keep full-path hardlink attribution.
- The `hardlinks` row still records the pre-zeroing sizes, as before.

**4. Directories are unaffected.**
- DENIED directories, empty directories and roots of kind Dir all still take both the child query and the UPDATE.
- An empty Dir still gets an UPDATE with `items=0, subtree_end=id`, and the new test asserts this.

**5. Frame size grows negligibly.** `Kind` is a fieldless enum (1 byte, plus padding), and there is one frame per active depth.

**6. The tests do what they claim.**
- The future-mtime test checks the clamp within one snapshot (`mtime>scanned_at AND newest_mtime=scanned_at` on each row) and compares no times across scans.
- The `set_modified` call on a read-only `File` is fine on Linux: `futimens` permission depends on ownership, not on the fd's open mode.
- In `invalid_leaves`, the `CASE … ELSE 0` branch correctly expects the 0 sentinel for Symlink, Other and Mount leaves.

## Findings

**[P3] No staged-writer coverage for two non-Dir shapes that now skip the UPDATE** (`core/tests/staged.rs`)

1. **A non-Dir root**, for example a regular file passed as a scan root. This is the only case where the bottom frame (`stack.pop()` with an empty stack, returning `frame.agg.errors`) is a leaf. Before this change its INSERT values were always followed by an UPDATE.
2. **A STAT_ERROR leaf** (and Mount, where the CI environment allows one). Its `errors` comes from the INSERT alone now.

Both should be correct by the reasoning in point 2. They are the leaf paths whose INSERT values were previously always overwritten, so a small `assert_equivalent` case would close the gap:
- a file root;
- an entry whose `lstat` fails, if this can be reproduced portably.

This is optional and does not block the change.

## Not verified (no access)

- The `entries` column order behind the positional INSERT. The new test covers it.
- The fixture behind `aliases == 2` and the `"empty"` directory.
- Whether `scan(&[&root], &disk, true)` actually selects the staged writer path.
- File size limits for `write.rs` and `staged.rs`. You report these pass.
- Performance: none claimed, none assessed.

## Parent dispositions and later proof

- The schema mapping is confirmed against core/src/snapshot/schema.rs.
- The optional non-directory-root assumption is not a reachable normal input:
  scan/walk.rs normalize_root explicitly rejects non-directories. Added a test
  that both scanner modes retain that rejection. No API scope was expanded.
- Added a synthetic STAT_ERROR unit fixture inside the writer module. It mutates
  a tiny staged leaf to File/Other/Symlink with errors1 and zero sizes, verifies
  output fields, subtree/freshness and parent/returned error counts. It avoids
  racing lstat, external load generators or a new public test API.
- Future file times are asserted within a single snapshot; comparing clamped
  newest_mtime across separate scans would depend on their observation seconds.
- The complete Rust contract suite passed on GitHub for measured commitf76a7cd.
  Later edge-test additions executed successfully in
  [runa2ac533](https://github.com/mikhailsal/free-space/actions/runs/41199678638):
  318 passed, 0 failed, one existing ignored test. The STAT_ERROR leaf and
  non-directory-root rejection cases both passed. Normal hooks also passed
  clippy/format/size. The production fast-path logic is unchanged by those tests.
- Pair validity and quantitative promotion gates still control acceptance.
  Source review alone establishes no speed, disk or memory improvement.
- The candidate is not promoted: the 1m paired median gain was14.29% and one
  7.3m pair gained11.92%, below the owner's fixed15% speed-only floor. See the
  [campaign report](../benchmark-results/v1-w1-w3-campaign-2026-10-31.md).
