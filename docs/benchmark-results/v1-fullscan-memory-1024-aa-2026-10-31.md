# Full-scan 1 GiB A/A control: build failure

The [GitHub run](https://github.com/mikhailsal/free-space/actions/runs/41242956623) failed in the build job before the paired job started. The Python benchmark contract test `test_bench_cleanup.CleanupTests.test_interruption_status_retained` raised `subprocess.TimeoutExpired` after five seconds; the Python suite reported 110 tests run with one error. The cause of that cleanup delay has not been established. Rust tests and measured workers did not run, and GitHub reported no uploaded artifacts.

The planned 1,024 MiB worker rung therefore has **no valid measurement**: there are no worker cgroup, OOM, pressure, observer, semantic, or performance receipts. This failure is not evidence that the application exceeded 1 GiB. Profile-v2 is refresh-only and no paired scan ran. Do not descend the ordinary memory ladder on this incomplete run.

The build job consumed 41 runner-seconds; the paired job was skipped. Source was `e0669801ef5dc0fb8cf3bef89f72aaec06489bf7`. The [receipt](v1-fullscan-memory-1024-aa-2026-10-31.json), [GitHub log, metadata and dispatch evidence](v1-fullscan-memory-1024-aa-2026-10-31.tar.gz), and [file hash index](v1-fullscan-memory-1024-aa-2026-10-31.sha256) preserve the available failure evidence. No retry was performed.
