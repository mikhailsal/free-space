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
