# Compact-stage changed-full smoke: failed setup

Run 41251341238 at source `601ce381ac229923d2c791fd8e7c510f7b3f1dfc` failed during the build job before compilation or measurement. The `Install musl tools` step ran `sudo timeout 180 apt-get update -q` and exited 124 after 180 seconds. The log shows repeated ignored Azure Ubuntu mirror requests; some HTTPS Ubuntu index requests succeeded. This establishes an apt-update timeout, while its underlying network cause remains unconfirmed.

The build job consumed 197 runner seconds; the paired job was skipped and consumed zero. No artifacts were uploaded. Rust and Python tests, EXPLAIN, candidate binary identification, mutation and snapshot validation, resource checks, and benchmarks did not run. This is invalid as performance or semantic evidence and does not qualify the candidate. The preregistered 100k-file, one-pair changed-full case remains unmeasured.

The archive preserves the dispatch, complete job log and metadata, artifact inventory, and exact workflow source. All 5 indexed payloads were verified. Archive SHA-256: `f8aa2ae2f7abc28d80c59cd4bf03b45a252a9cfd02e697d7610e78bb3574a14b`.
