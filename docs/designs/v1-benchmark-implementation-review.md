# W1 implementation review

Provider: Claude Code; requested/reported model claude-opus-5-5.
Reviewed commit a113927, implementation-only diff with existing-helper context.
No tools/repository access. This is a review with findings, not a clean pass of
subsequent amendments.

# W1 v1-vs-v1 benchmark review

I reviewed only the supplied diff. Line numbers marked `~` are counted from the hunks and are approximate. I could not see the existing `measured_command` row construction, `flush_caches`, `snapshot_integrity`, the hidden `paths:` lines, or how `bench-representative.sh` creates the backing image. Findings that depend on those are marked conditional.

## [P2] A failed experiment produces no `summary.json` or step summary

`scripts/bench_v1_pair.py:~162-182` (`host`) re-raises in `except Exception: ... raise`. `summarize(output)`, `summary.json` and `summary.md` all come after the `try/finally`, so any of these leaves only `status.json`:

- seed-scan failure,
- worker OOM or nonzero exit (`container()` raises),
- reserve kill,
- copied-seed digest mismatch.

Separately, `scripts/bench-representative.sh` runs `cat "$output/measurements/summary.md" >> "$GITHUB_STEP_SUMMARY"` after `run_owned`, under `set -e`. So even when `host` returns 1 for invalid evidence, the INVALID summary never reaches the step summary.

This contradicts `docs/v1-benchmark.md` ("writes descriptive summary.json … returns nonzero for invalid evidence"). Invalid runs are exactly the ones the operator must account for against the 6-hour cap.

**Fix:**
- Record the failure, run `summarize` and write both files inside `finally`, then return nonzero.
- In the shell script, capture the status, append the summary, then `exit "$status"`.

## [P2] Pair equality compares a measured value, so it can invalidate correct runs

`scripts/bench_v1_summary.py:~170`: `ignored = {'id','arm','binary_sha256','source_revision'}`. Every other spec key must match across arms, including `initial_snapshot_allocated_bytes`.

That value is measured per arm in `bench_v1_pair.py:~143` from a fresh `copytree`, and it is not configuration. On ext4, `st_blocks` includes extent-tree blocks, and a multi-GB copy written into space freed by the previous arm's `rmtree` can fragment differently. A one-block difference would mark a 2-hour local-scale pair as "configuration or seed mismatch". The seed is already checked by `seed_sha256`.

**Fix:** Exclude measured setup fields from the equality check, and report them separately.

## [P2] Semantic digest ordering is not deterministic for some tables and is strict about generated IDs

`scripts/bench_v1_runtime.py:~76-78` sets `primary = "id" if table == "entries" else "entry_id" if "entry_id" in columns else columns[0]`, then uses `ORDER BY "{primary}"`.

- **Non-unique sort keys:** For `roots`, `mounts`, `meta` and any multi-row table keyed by a non-unique column, row order is unspecified. The digest then depends on physical layout and query plan rather than content. Two arms that are semantically identical can produce different `semantic_sha256` and become "observer mismatch".
- **Entry IDs are hashed exactly:** Ordering and hashing entries by `id` treats rowid assignment as semantics. The docs say "Generated IDs … normalized", but `normalize` only normalizes `meta.snapshot_id`. A candidate that changes traversal or insert order with identical content will always be INVALID. If that strictness is intended, the doc needs to say so; otherwise digest by path or parent key.
- **NULL comparison crash:** In `normalize`, `value > cutoff` raises `TypeError` when `scanned_at` or `min_scanned_at` is NULL. That surfaces as an observer failure instead of a clear diagnostic.

**Fix:** `ORDER BY` all columns, or a declared unique key per table. Guard the comparison with `isinstance(value, (int, float))`.

## [P2] Single-pair stages always run baseline first, with no counterbalance

`bench_v1_pair.py:~133`: `arms = list(binaries) if pair % 2 else reversed(...)`. Smoke and local-scale use `BENCH_PAIRS=1`, so baseline always runs first.

Page-cache dropping does not remove all order effects. The second arm writes into filesystem space just freed by the first arm. If the loop backing file is sparse or lazily allocated on the host, only the first arm pays host-side block allocation. That would give a systematic candidate-favoring delta in elapsed time, and possibly in disk-peak timing, in exactly the single expensive full-scale pair.

**Fix (any of):**
- Preallocate the backing image (verify in the hidden setup).
- Add an unmeasured warm-up copy and delete.
- Accept only even pair counts for any stage used in comparisons.

Also state the order in `summary.md`.

## [P2] Summary validator is never tested against what `measured_command` actually writes (conditional)

`summarize` requires these `metrics.json` keys:

- `arm`, `pair`, `binary_sha256`, `source_revision`, `seed_sha256`, `memory_mib` (the `bound_keys`)
- `exit_code`, `timed_out`, `sampler_errors`, `cgroup_after`, `time.peak_rss_kib`

`scripts/tests/test_bench_v1_summary.py` builds `metric = {**spec, ...}` by hand. The diff to `bench-v2-container.py` does not show `measured_command` copying the new v1 spec fields (`arm`, `pair`, `binary_sha256`, `source_revision`, `seed_sha256`) into `row`. Those fields did not exist for v2.

If `row` is not built from the full spec, every real run is INVALID. That would only be discovered after spending runner time.

**Fix:** Add a contract test that runs `measured_command` with a stubbed CLI, or extracts its row builder, on a v1 spec, and feeds the output to `summarize`.

## [P3] Scratch `rmtree` runs before `status.json` and summary are written

`bench_v1_pair.py:~171` runs `shutil.rmtree(scratch)` over the 7.26M-file fixture before `status.json` and the summary are written. The volume is torn down by `bench-representative.sh` anyway.

At local scale this is plausibly many minutes inside the 120-minute step. If the step timeout fires there, all evidence is lost. It also counts against the forecast and runner-hour cap.

**Fix:** Write status and summary first. Then either delete only the per-run directories or rely on volume teardown.

## [P3] Build provenance records constants, not observations

`.github/workflows/representative-bench.yml` (build-record heredoc) hardcodes `'flags': ''` and `'profile': 'release --locked'`.

- The baseline is built via `--manifest-path`, so its `[profile.release]` comes from the baseline `Cargo.toml`.
- `.cargo/config.toml` and `rust-toolchain.toml` are resolved from the current directory, which is HEAD.

So a profile change (LTO, codegen-units) or a config `rustflags` difference would be recorded as "identical flags". `bench_v1_pair.py` then trusts that equality.

**Fix:**
- Record `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS`.
- Record `cargo config get` output from each source tree.
- Record a hash of each tree's `[profile.release]` section.

## [P3] Capacity admission undercounts the retained seed

`bench_v1_pair.py:~81-82` passes `seed_bytes=estimated_entries * 256 * 2`. That is 256 B per snapshot DB, while `estimate_capacity` assumes 512 B per snapshot for every other copy, and the seed holds two full DBs.

Admission can therefore pass, spend the time generating the fixture, and then fail at the per-arm check or the reserve kill.

**Fix:** Use `2 * 512` per entry, consistent with the other components, or measure the seed size.

## [P3] Reserve kill depends on process-group setup that the diff doesn't show (conditional)

`bench-v2-container.py:~180` calls `os.killpg(proc.pid, SIGKILL)`. This only works if `Popen` uses `start_new_session=True`/`process_group`. Otherwise it raises `ProcessLookupError` or kills the wrong group, the CLI keeps filling the filesystem, and the only signal is a sampler error.

There is no test for the reserve-exhausted path.

## [P3] Time admission is self-attested and the budget is only manual

- `forecast_seconds` is a free-form operator number checked against a fixed `120*60`. Nothing ties it to a screening receipt, despite the doc's "screening-based forecast" wording.
- Push-triggered smoke runs (up to a 60-minute envelope each) draw from the 6-runner-hour cap with no dispatch or record step.
- With `cancel-in-progress: false`, a later push replaces a pending queued local-scale dispatch in the same group.

**Fix:** Document these limits, or drop the push trigger for the campaign.

## [P3] Missing focused tests

- `bench_v1_fixture.prepare`: `expected_entries` and `directories` accounting checked against an `os.walk` count (including hardlinks) at around 2,100 files, plus the `mutate` subtree count of 100.
- `normalize`: the `meta.scanned_at` cutoff boundary (equal versus greater), and NULL timestamps.
- Workflow stage mapping and the `admit_time` call path.

## [P3] Python additions have no lint or size gate

- `host()` (~125 lines) and `summarize()` (~130 lines) exceed the global 80-line function default.
- Python is not in the size count and has no ruff/mypy gate in `scripts/check.sh` or CI.

The project rules say to add a language to the count only together with its linter. Splitting these functions, or agreeing a Python gate with the owner, is needed before they grow further.

## Conclusion

The changes are measurement infrastructure only. Nothing here demonstrates or qualifies an optimization. The P2 items should be fixed before relying on any screen or local-scale result, and the conditional items need verification against the parts of the code not included in the diff.

## Parent dispositions

- Failure summaries: fixed. Host records errors and still writes invalid results;
  shell retains the return code, publishes available diagnostics and revalidates
  after outer cleanup. Added an early-admission-failure test.
- Initial allocation equality: fixed. Measured copy allocation is no longer
  treated as pair configuration. Seed content hashes remain exact.
- Semantic ordering: deterministic full-column tie breaks added. Exact DFS entry
  IDs are intentional per v1 compatibility contract; documentation now says
  generated snapshot IDs, not every entry ID. Invalid NULL timestamps now have
  explicit diagnostics and boundary tests.
- Order bias: backing image was already physically preallocated; initial stages
  are identical-binary controls, not promotion. Three-pair screening alternates
  AB/BA; a single local-scale pair cannot establish qualification.
- Metrics/spec binding and reserve-process-group concerns: source confirms
  dict(spec,...) and start_new_session=True. First live smoke validated the real
  writer/observer receipts. No missing-spec-field defect was reproduced.
- Teardown: terminal status is saved before cleanup; millions of fixture names
  remain on the exclusively owned filesystem until bounded outer teardown.
  A final outer cleanup receipt is now mandatory (protocol v2).
- Provenance: observe environment flags, per-source Cargo config and release
  profile, use the pinned toolchain, and compare those build inputs.
- Retained seed estimate: use the same 512 bytes/entry allowance for both seed
  generations; expand the v1 preallocated filesystem to40GiB with42GiB host check.
- Time limits: operator accounts every job in the campaign ledger, including push
  smoke/failures. Forecast is an explicit operator estimate, not automatically
  derived or a guarantee. No queued dispatch is counted as completed evidence.
- Tests: added mixed-fixture count/hardlink/mutation checks, timestamp-boundary
  and NULL checks, and failure-summary/cleanup regressions.
- Python function-limit claim: repository CLAUDE.md specifies80-line limits for
  Rust and TypeScript, not a generic Python limit. Existing Python harness files
  predate this work and are not in check-size. No lint/size gate was bypassed or
  weakened. Python gets bytecode checks and focused tests in this workflow.

Remote protocol-v2 verification is still required; initial v1-protocol artifacts
are retained only as pre-hardening evidence.
