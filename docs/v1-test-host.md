# V1 test-host operations

The owner identifies the local machine as a test host and authorizes choosing or
switching backend as useful. Snapshot history is disposable; old databases may be
deleted if capacity requires it. Most expensive experiments remain on GitHub.
One local scan and one small-folder refresh have already completed; these are not
permission for an unlimited local performance matrix.

## Current evidence and selection

The installed configuration selects v2. Fresh v1 results are isolated under
`/var/lib/freespace/calibration-20261031`. The observed v1 full scan took 1779.09 s;
refreshing 2 entries took 66.61 s and peaked at 1822.36 MiB process RSS. Both snapshots
report 2 inaccessible temporary subtrees. See the JSON receipts under
`docs/benchmark-results/local-v1-*2026-10-31*` for exact limits and provenance.

Use the explicit backend and directory for individual tests; a global switch is
unnecessary for reading the isolated snapshot:

```bash
sudo env FREESPACE_SNAPSHOT_DIR=/var/lib/freespace/calibration-20261031 \
  /usr/local/bin/freespace --backend v1 status --json
```

Do not expose the legacy v1 snapshot in `/var/lib/freespace` as the new result:
it predates the calibration. The experimental v2 store remains separate in
`/var/lib/freespace/v2`. A backend change by itself performs no history conversion.

## Before a global switch or disposal

Produce a concrete manifest from the actual host immediately before execution:

1. Record binary hash/version, effective config and environment, snapshot paths,
   ownership/ACL, unit/drop-ins, timer state and next activation. Preserve small
   binary/configuration recovery material; a database archive is not required.
2. Check filesystem bytes and inodes, current/previous/output/staging/temp and
   coexisting copies. First-generation calibration peak is not a steady-state
   budget: retained generations must be included separately.
3. Quiesce scheduled/manual writers and serving readers. Confirm no matching scan
   or refresh remains. Check open file descriptors under the exact snapshot root;
   unlinking a database held by a reader may release no blocks.
4. Validate canonical paths, device identity, mount boundaries and an explicit
   allowlist of generated files. Reject symlinks, path escapes, unexpected layout
   or active mounts. Never use uninstall as a data migration.
5. If disposal is needed, remove only the allowlisted generated results/state/temp,
   preserving binary, config and unrelated files. Recheck actual available blocks
   after removal. A backup on the same filesystem consumes the same scarce space.
6. Use an explicitly empty/verified v1 directory, or the already validated isolated
   v1 result for test-only selection. Preserve its incomplete diagnostics. Before
   serving it to ordinary users, establish the expected access/ownership and unit
   write paths; regenerate the unit if the configured snapshot directory changes.
7. Verify effective backend, root count, status, sampled queries and publication.
   A freshly reset history has no previous snapshot; do not force a second scan
   solely to manufacture a diff. Existing calibration generation2 has a previous
   snapshot because the separately authorized refresh created one.

## Failure and recovery

Before disposal, an abort may restore the original configuration/timer state.
After disposal, keep writers/timers stopped until verified regeneration or a
chosen recovery is ready. Do not automatically restart v2 into a capacity failure.
The recovery promise is binary/config restoration plus a new scan if needed;
there is no database-history rollback after authorized deletion.

Rehearse the reset, open-reader, capacity-failure and retry paths on a disposable
GitHub runner before automating this sequence. That W2 rehearsal is a separate
future campaign; creating this runbook does not claim it has run.
