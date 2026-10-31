# Staged-refresh prototype: smoke stopped by correctness gate

The [first W5 smoke](https://github.com/mikhailsal/free-space/actions/runs/41239317973) failed its existing CLI regression before the benchmark job. Source `9a1c9f4f7fc9b262b14df288b3df67ab26e80877` is retained as failed evidence. There are no measurements or uploaded artifacts from this attempt.

The refresh command succeeded, but its staged output omitted the refresh-history rows used by status. `StagedTree::write_inner` wrote the metadata row and never called the separate `writer::insert_refreshes` helper. The existing `refresh_republishes_and_diff_sees_changes` test caught the missing refreshed path. Source reviews had not found this omission; runtime tests blocked measurement.

The proposed correction reuses that helper inside the same transaction and adds a public regression retaining old history through three refreshes, including a repeated path. It is a new source revision, requiring review and a separate admission before any further smoke. The failed source will not be retried. Local checks compile metadata only; Rust tests run on GitHub.

This attempt used 74 runner-seconds in the build job; the paired job was skipped. The receipt and hash-verified archive preserve the GitHub metadata and failure log. No speed, RAM, disk, or prototype qualification is claimed.
