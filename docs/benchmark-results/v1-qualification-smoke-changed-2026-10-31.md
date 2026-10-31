# Qualification harness changed-case smoke

The [GitHub smoke run](https://github.com/mikhailsal/free-space/actions/runs/41232072330) validated the v4 paired protocol for one refresh pair on a 100,000-file fixture, with a 10,000-entry subtree, the `changed` rename/size/mtime recipe, a 1,024 MiB worker and 4,096 MiB setup and observer. Strict recomputation and the prospective profile-v2 structure checks passed. The original HWM monotonic contract did not pass; its raw anomalies remain in the archive.

Both arms used musl binary SHA-256 `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`, also observed in the first smoke. The log records an exact restore of the target-and-epoch cache key. This is observed binary identity for these runs, not a general reproducibility guarantee. The build reported 307 Rust tests passed, zero failed and one ignored, and 90 Python tests passed. Build and paired jobs consumed 148 and 92 runner-seconds, respectively.

This is a protocol smoke, not evidence of a qualified performance gain or minimum RAM. [Receipt](v1-qualification-smoke-changed-2026-10-31.json), [original artifacts and supporting evidence](v1-qualification-smoke-changed-2026-10-31.tar.gz), and [file hash index](v1-qualification-smoke-changed-2026-10-31.sha256) preserve the raw receipts and validation outputs.
