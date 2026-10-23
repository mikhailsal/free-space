# Changelog

All notable changes to this project. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/). The snapshot format and the `--json` answers (`skill/schema.json`) are part of the public interface: a breaking change to either bumps the major version once 1.0 is out.

## [Unreleased]

## [0.1.1] - 2026-10-23

### Fixed

- The nightly scan skipped `/home` and `/root`. The unit's `ProtectSystem=strict` mounts them (and the snapshot folder) onto themselves inside the unit, and the walk stopped at every same-device mount. It now stops only at mounts that show another part of the filesystem, so nothing is counted twice and those folders are walked. After upgrading, run `sudo systemctl start freespace.service` for a full snapshot now.
- `doctor` asked for 2 % of the used space per snapshot before the first one existed (25 G on a 408 G disk whose snapshot is 1 G). It now estimates from the inodes in use, as `scan` does.
- The first scan showed no progress percent when one of the roots was a FAT partition such as `/boot/efi`.

## [0.1.0] - 2026-10-23

First public release.

### Added

- `scan`: parallel walk of every real filesystem (or the given roots), one snapshot per scan in SQLite, published with a rename so readers never see a partial file; hardlinks counted once; unreadable folders reported as `incomplete` with their paths.
- `ls`, `top`, `find`, `diff`, `status`: answers from the snapshot, not the disk, as tables or `--json` (`ls`, `diff` and `status` in milliseconds; `top` and `find` read a 7.7 M-entry snapshot in parallel in under 1.7 s cold); `top --kind dir --older-than` lists folders by their newest file; `find` totals every match and counts nested matches once.
- `refresh`: rescan one folder into the current snapshot.
- `install-timer`: nightly systemd timer at idle priority, read access for the sudo user through an ACL (group fallback), first scan right away; `--uninstall` removes everything it added.
- `gui` and `serve`: the snapshot in the browser on 127.0.0.1 with a per-run token, keyboard driven, with Browse, Top files, Find and Changes views; over ssh `gui` prints the tunnel line.
- `doctor`: checks the config, timer, unit, binary, `PATH`, access, freshness, last error, free space and skill, and prints a fix for each failing check.
- `skill`: the Claude Code skill (`SKILL.md` plus `schema.json`); `schema` prints the JSON Schema of every answer.
- Error contract: every error has a stable code, exit number, `fix`, `recovery` hint and a `docs` link into the README.
- `install.sh`: one-line install, upgrade and rollback (`--version X`) from GitHub Releases with sha256 checks; static musl builds for x86_64 and aarch64.

[Unreleased]: https://github.com/mikhailsal/free-space/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/mikhailsal/free-space/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/mikhailsal/free-space/releases/tag/v0.1.0
