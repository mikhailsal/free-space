# Qualification harness large-folder smoke: admission failure

The [GitHub run](https://github.com/mikhailsal/free-space/actions/runs/41232507959) failed before creating the dedicated filesystem, fixture, or measured commands. The paired runner reported approximately 14 GiB available on `/`, below the script's 42 GiB free-space requirement. The uploaded artifact contains its host-disk and cleanup logs; there are no paired metrics, strict summary, or profile to validate. This is an infrastructure admission failure and says nothing about worker RAM success or product behavior.

The build succeeded: 307 Rust tests passed, zero failed and one was ignored; 90 Python tests passed. The target-and-epoch cache restored as a full match. The downloaded baseline and candidate binaries both hash to `ed2a1cb138b1e48e70a388a4a8fda3e79fcfd2e37fd0c061d41d2d6363f8e980`, matching the earlier smoke binaries. Build and paired jobs consumed 167 and 12 runner-seconds, respectively. The binaries themselves are excluded from the archive; their build metadata is preserved.

[Receipt](v1-qualification-smoke-large-folder-2026-10-31.json), [original failure artifacts and supporting logs](v1-qualification-smoke-large-folder-2026-10-31.tar.gz), and [file hash index](v1-qualification-smoke-large-folder-2026-10-31.sha256) preserve the available evidence. No retry or performance qualification is implied.
