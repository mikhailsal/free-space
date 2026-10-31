# Corrected staged refresh smoke: failed before measurement

Run [41240352711](https://github.com/mikhailsal/free-space/actions/runs/41240352711) at candidate `a4ffffb791652a5b7fbd3da04533e5cc143c4414` failed during the build job’s benchmark contract and repository gates. `test_bench_cleanup.CleanupTests.test_interruption_status_retained` raised `subprocess.TimeoutExpired` after five seconds. The log does not establish why the fixture timed out. Of 112 Python tests, one errored. The build job ran for 35 seconds and the paired job was skipped.

No Rust workspace or CLI tests ran, no benchmark binaries or measurement artifacts were produced, and the semantic, resource, cleanup and reserve gates were not evaluated. There are no wall time, RSS, cgroup or disk deltas and no no-regression or promotion conclusion. The declared diagnostic policy was `none`; no profile-v2 result is claimed. No retry was performed.

The accompanying archive contains the unmodified GitHub run metadata and log; the JSON record identifies their SHA-256 hashes.
