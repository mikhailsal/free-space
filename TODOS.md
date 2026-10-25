# TODOS

## Deferred from /plan-ceo-review 2026-10-22 (freespace)

- [ ] **TUI (ratatui) — v1.1.** Deferred by D3 (HOLD-1). Context: same screen as web (table allocated/share/items/modified, footer keys ↑↓ → ← t r a), built on `core` query layer; no core changes expected. Trigger: first time you need interactive browsing over ssh on a headless host. Source: docs/designs/free-space-disk-analyzer.md, ARCH-1.
- [ ] **Scheduler backends launchd / Windows Task Scheduler / Termux cron.** Deferred by D4 (HOLD-2). Context: `core` keeps the `Scheduler` trait and the systemd timer backend; add a backend file per OS when the first non-Linux host appears (~1 file each). Source: docs/designs/free-space-disk-analyzer.md line 128.
- [ ] **`--apparent-only` scan flag.** Deferred by D5 (HOLD-3). Context: both st_size and st_blocks are stored per entry; revisit only if a filesystem makes st_blocks measurably expensive. Source: docs/designs/free-space-disk-analyzer.md CLI contract.
- [ ] **Low-free-space alerts.** Deferred by D8 (EXP-3). Context: after the scheduled scan, compare free% against `low_free_pct` (config), write `status.alerts[]`, optional `notify-send`. Trigger: after a week of nightly scans shows the real growth curve. Source: docs/designs/free-space-disk-analyzer.md CEO review.
- [ ] **`--host <ssh-alias>` wrapper and `status --hosts` fan-out.** Deferred by D10 (EXP-5). Context: thin `ssh alias freespace <cmd> --json` wrapper in cli, same JSON contract, error codes for missing binary / missing snapshot / ssh failure. Trigger: binary installed on 2+ hosts. Until then SKILL.md documents the manual ssh form. Source: docs/designs/free-space-disk-analyzer.md CEO review, motivation line 16.

## Deferred from /plan-eng-review 2026-10-22 (freespace)

- [ ] **`serve` idle timeout for `gui`.** Deferred by D8/D14 (user chose Ctrl-C lifetime). Context: exit after N minutes without `/api/*` requests, page heartbeat = existing `/api/status` poll, `--keep-alive` to disable. Trigger: the first forgotten `serve` process found in `ps` holding a token. Source: docs/designs/free-space-disk-analyzer.md, eng ledger R6.
- [ ] **Unix socket with uid check for `serve` on multi-user hosts.** Deferred by plan (v2) and D14. Context: replace loopback+token with `$XDG_RUNTIME_DIR/freespace.sock` (0600) and `SO_PEERCRED`; the web page is unchanged, only `api.ts` transport. Trigger: first host with more than one interactive user. Source: docs/designs/free-space-disk-analyzer.md, serve section residual risk.

## Deferred from /plan-design-review 2026-10-22 (freespace)

### Dark theme (D14 / D19)
- **What:** second value block for `web/src/tokens.css` under `prefers-color-scheme: dark` (+ manual toggle), re-checked contrast for every fg/bg pair, `color-scheme: light dark`.
- **Why:** gui is opened at night on a dev box; a white page is a flash in a dark terminal session.
- **Pros:** semantic variables already isolate the change to one CSS block and a contrast test; no markup changes.
- **Cons:** doubles the colour surface to verify (share bar, warn/danger hues, selection, scrollbar); needs a second screenshot pass in `/design-review`.
- **Context:** D14 chose light only for v1 (decision `ea8cbf40-0442-4c0e-a013-72a9db1a4630`, marker `gstack-shortcut(dec-ea8cbf40)` in tokens.css). Trigger: first complaint about the night-time white flash.
- **Depends on:** T32 (tokens.css + contrast test).

### Single-column phone layout (D16 / D20)
- **What:** under 480 px: one column per row (name + size line + share bar), inspector as a full-height sheet, mode switcher as a bottom bar, breadcrumbs collapsed to "… / parent / current"; spec recorded in design-review Pass 6.
- **Why:** Termux + phone browser is an explicit v2 target; the v1 rule only guarantees "no breakage" down to 360 px.
- **Pros:** spec already written with breakpoints and component changes; a11y contract (44-px targets, focus order) carries over unchanged.
- **Cons:** a second layout to keep in sync with every table change; needs Playwright runs at 360/390 px added to T35.
- **Context:** D16 chose "spec v2, v1 no-break" (decision `8e0d4823-69ef-4edb-8517-be3c746edf2d`, marker `gstack-shortcut(dec-8e0d4823)` in responsive.css). Trigger: Termux build lands or the user opens gui from a phone regularly.
- **Depends on:** T35 (responsive/a11y contract), T26 (states).

## Found by the benchmark 2026-10-23 (T24)

- [ ] **Per-folder file totals for `top` under 300 ms warm.** `top /` on 7.77 M entries answers in 0.68 s warm (1.31 s cold) against the 300 ms target, and untouched folders in 0.39 s. The answer's total covers all 5.9 M files, so every row is read; one core needs 2.0 s and 8 threads on 4 physical cores give 2.9× (docs/benchmarks.md, "Missed"). Context: three columns per directory (files, file allocated, file apparent) filled in the post-walk aggregation pass would answer the unfiltered total in O(1) and the page through `entries_alloc`; filtered totals still need the pass. Schema change (snapshot regenerated, ~+15 B per entry, refresh must refold them up the parent chain). Trigger: a user who finds Top files slow, or a host with 2× the entries. Source: docs/benchmarks.md, core/src/queries/pass.rs.

## Deferred from /plan-eng-review 2026-10-23 (code quality gates)

### Speed check for scan-path PRs (TD1 / D24)
- **What:** in the `refactor-guard` workflow, build base and head release binaries and run 7 warm scans of each, interleaved, on one fixture tree; compare CPU time (task-clock) and per-phase seconds; fail when head is over base by more than 15%.
- **Why:** D18 guards the debt splits of the scan path (`walk.rs`, `tree.rs`, `refresh.rs`, `writer.rs`) by code review only. A slowdown without a visible clone or allocation would pass unnoticed.
- **Pros:** the recipe is measured already (`docs/designs/code-quality-gates.md`, Section 4); runs on GitHub's free compute, not the dev box; numbers per phase point at the slow step.
- **Cons:** shared runners are noisy (not measured; this VM showed ±12% CPU time over 10 runs), so the bar may need tuning or retries; adds two release builds (base and head; runner time not measured) to every scan-path PR.
- **Context:** D18 chose review only for the debt PRs (decision `f4cfa71c-e7e2-4165-b08e-983cbf073e49`). Trigger: a scan-path PR whose per-phase seconds in the scan log move visibly, or a report of slower scans. Start from the timing table in Section 4 of the plan and the `refactor-guard` job.
- **Effort:** S
- **Priority:** P3
- **Depends on:** `refactor-guard` workflow (D23).

## Found by /ship adversarial review 2026-10-24 (pre-gate runner)

- [ ] **Pre-gate guards: check what they trigger on.** Context: the `shell` guard fires on `.githooks/**` but runs `shellcheck install.sh scripts/*.sh`, which misses `.githooks/pre-push` (no `.sh`); `bun install --frozen-lockfile` runs only in the `web` guard, so a change to `scripts/test-files.sh` or `.gstack/pregate.json` alone runs `test-files.test.ts` in a fresh worktree without the frozen install; a run that selects both e2e specs and shots builds `web/dist` and starts the server twice. Fix: the gates PR moves the guards to `scripts/check.sh` parts (G6), whose `shell` part covers the hook; add `scripts/**` and `.gstack/pregate.json` to the `web` guard's `when`, or install in the runner; build once in the runner. **Depends on:** the gates PR. **Priority:** P3. Source: maintainability, red-team and performance specialists, PR 1 final review.
- [ ] **`scripts/test-files.sh`: a SIGKILL to the script alone, and a signal while a runner starts.** Context: (1) a SIGKILL sent to the script's pid alone (a manual `kill -9`, not the pre-gate, which kills the group) releases the lock while Playwright runs on, because the runners get fd 9 closed; the next run then clears `web/pregate-shots` and rebuilds `web/dist` under the orphan, and both write the same files. master behaves the same way. (2) A HUP, INT or TERM in the instant between `"$@" &` and `child=$!` finds no child: the script exits without stopping the runner, its EXIT trap removes `pregate-shots` under it, and the lock goes with it. Fix: (1) let Playwright keep fd 9 and close it only in the web server command of the Playwright configs (the server now stops when Playwright's pipe closes); check that no browser outlives Playwright holding it; (2) a `starting` flag set around the launch, for which `stop()` only records the signal, acted on once `child` is set. Trigger: a pre-gate run whose shots differ from a rerun's with no change, or a caller that SIGKILLs the script's pid. **Priority:** P3. Source: Codex adversarial review of the runner-hardening PR (2026-10-25); (2) also the Claude adversarial pass and that PR's Step 7 coverage gap.

## Found by /ship adversarial review 2026-10-24 (refactor-guard)

- [ ] **Run the guard spec on every PR that touches `web/`, not only on refactor PRs.** Context: `web/tests/guard/views.spec.ts` runs only when the `refactor` label is present, so a feature PR that changes a role name or a request breaks it silently, and the next refactor PR fails on its base take for a reason it did not cause; fixing it there means changing the harness that judges that PR. The rust half is done: since A0 the `rust` job of `ci.yml` runs `scripts/rust-diff.sh` with head against itself on every PR. Fix: in `ci.yml`'s `web` job, run the guard config once against head's build (shots only, no comparison). Trigger: the first false red of that kind, or with the gates PR. **Priority:** P2. Source: Claude adversarial review of PR 2.
- [ ] **rust-diff: `install-timer` as root.** Context: `scripts/rust-diff.sh` asks `install-timer` only without root (it refuses with exit 1), so the units, config, group, ACLs, completions and skill it writes as root are compared by nobody; the `install` job of `ci.yml` runs it on head only. Fix: under sudo on the runner, base then head: install with `--no-start` into a scratch snapshot dir, record what it wrote (unit text, config, modes, ACLs, the report with `next_run` masked), uninstall, record what is left; compare. It needs a script of its own: `scripts/rust-diff.sh` refuses to run as root. Trigger: lands in A1 of `docs/designs/code-quality-gates.md`, and must be in place before A4 splits `cli/src/install.rs`. **Priority:** P2. Source: /ship review of PR 2 (D11).

## Found by /ship review 2026-10-25 (rust-diff coverage, A0)

- [ ] **rust-diff: compare the snapshot databases' rows, and give a harness error its own exit code.** Context: (1) `db_of` in `scripts/rust-diff.sh` compares each database's schema, `user_version` and rows per table, not the rows themselves, so a debt PR that writes the same number of entries with another size, parent or name passes; (2) under `set -e` an unplanned failure of the harness itself (a `cp`, a `mkdir`) ends the job with exit 1, the code for "head answers differently", so a broken run reads as a behavior change. Fix: (1) dump each table ordered by its key with the live columns (times, ids, `allocated` of the files the drift touches) masked as the answers mask them, and compare the dumps; (2) an ERR trap that exits 5 with the failing line, and 5 in the header's and CONTRIBUTING.md's list of exit codes. Trigger: A0b, before A2 splits `core/src/scan/writer.rs`. **Priority:** P1. Source: red team of /ship review cycle 3 of A0.
