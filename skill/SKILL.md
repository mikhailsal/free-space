---
name: freespace
description: Find what is using disk space on this Linux machine and help the user free it. Use when the user asks what eats their disk, why the disk is full, what grew recently, which old folders or big files they can delete, or how much a kind of file (ISOs, node_modules, caches) takes in total. Reads a nightly snapshot through the `freespace` CLI with `--json`; never scans the disk itself.
---

# freespace: disk usage from a ready snapshot

`freespace` keeps a snapshot of the whole filesystem tree, refreshed every night by a systemd timer. Every command below reads that snapshot, answers in milliseconds and changes nothing on disk. Always pass `--json` and parse stdout. The JSON Schema of every answer is in `schema.json` next to this file (`freespace schema --json` prints the same).

## Start every session with status

```
freespace status --json
```

- `age_seconds`: how old the data is. Tell the user the age when you report sizes ("as of 7 h ago").
- `incomplete: true` with `unknown_subtrees`: some folders could not be read, so sizes under them are unknown. Say this **before** any numbers.
- `scanning` set: a scan is running. Wait: poll `freespace status --json` every 10 s, up to 30 min. Do not start another scan (it fails with `LOCK`).
- `last_error` set or `age_seconds` > 93600 (26 h): run `freespace doctor --json` and relay the failing check's `fix` to the user.

## The three questions, in this order

1. **What grew?** `freespace diff / --json`, then `freespace diff <the biggest grower> --json` to descend. On the first day there is no previous snapshot: exit 3 `NO_PREV`. Skip to question 2.
2. **What is big and untouched?** `freespace top / --kind dir --older-than 180d --limit 20 --json` (folders whose newest file is older than 180 days; none counted twice) and `freespace top / --min-size 1G --json` (largest files).
3. **Where exactly?** `freespace ls <path> --json` lists children, biggest first. Repeat on the biggest child until the answer is specific.

For "how much do all X take": `freespace find / --name '*.iso' --json` or `freespace find /home --kind dir --name node_modules --json`. `total.allocated` covers every match, not just the listed page; matches inside another match are counted once (`nested: true`).

Flags shared by `ls`, `top`, `find`: `--sort allocated|apparent|mtime`, `--limit N` (default 30, max 10000). Dates: `180d`, `4w`, `6mo`, `1y` or `2024-01-31`. Sizes: `500M`, `1G`.

## Reading the numbers

- `allocated` is bytes on disk (what deleting frees); `apparent` is the file length. Folders carry subtree totals.
- `newest_mtime` on a folder is the newest file inside; `0` means unknown (empty folder or unreadable). Unknown dates pass `--older-than`, so check such rows with `ls` before calling them old.
- `path_raw` appears when a name is not valid UTF-8. Pass it back with `--path-raw <hex>` instead of the path.
- `hardlink_alias: true`: this file's size is counted at `hardlink_owner`. `nlink > 1` means deleting one link frees nothing until every link is gone.
- `kind: "mount"`: another filesystem is mounted there and was not counted here.

## Before suggesting any deletion

The user decides. Never delete anything without their explicit yes to the exact list.

1. Show a table: path, `allocated` (human units), last modified, and why it is safe to delete (cache that rebuilds, old download, build output, duplicate).
2. The snapshot can be up to a day old. Check each path still exists and still has that size: `stat -c '%s %y' <path>` or `du -sh <path>`.
3. Prefer the native cleaner when one exists: `docker system prune`, `pip cache purge`, `uv cache clean`, `npm cache clean --force`, `cargo clean` (in a project), `journalctl --vacuum-size=500M`, `apt-get clean`, emptying the trash.
4. Warn about system paths (`/usr`, `/var/lib`, `/etc`, `/boot`, `/snap`): deleting there can break the system.
5. Only after the user says yes to the list: delete exactly those paths (`rm -rf -- <path>`; `sudo` only for paths the user cannot write and only if they agree).
6. After deleting, run `df -h <mount>` and report what was freed. If `df` did not move, files may still be open: `sudo lsof +L1` lists deleted-but-open files.
7. To refresh one folder's numbers after cleaning: ask the user to run `sudo freespace refresh <path>`.

## Errors

On failure, stdout carries `{"error": {"code", "exit", "message", "fix", "recovery": {"action", "side_effects", "max_retries"}, ...}}`. Key on `error.code`, not on the exit number, and follow `recovery.action`:

- `NO_SNAPSHOT` (8): freespace is not installed or has not scanned yet. If `status` shows `scanning`, wait as above; otherwise ask the human to run the command in `fix`.
- `SNAPSHOT_ACCESS` (9): this user cannot read the snapshot. Ask the human to run `sudo freespace install-timer`.
- `NO_PREV` (3): no previous snapshot for `diff`. Use `top` and `ls` instead.
- `ROTATED` (4): a new snapshot was published mid-query. Retry once.
- `PATH_NOT_FOUND` (2): the path is not in the snapshot; `fix` names the nearest existing parent.
- `USAGE` (2) from a remote host: compare `error.context.version` with your local `freespace --version`; a mismatch means the remote needs the installer rerun.
- `LOCK` (6): a scan or refresh is running. Wait and retry.
- Anything else: run `freespace doctor --json` and show the user the failing check and its `fix`.

- `GUI_NO_TTY` (2): you ran `freespace gui`. Use the `--json` commands above instead.

Never run `freespace gui` or `freespace serve`: they are the human's browser view and run until stopped. When the user wants to look for themselves, tell them to run `freespace gui` in their own terminal (over ssh: `ssh -t -L 8710:127.0.0.1:8710 <host> freespace gui --port 8710`). Never run `sudo` yourself without asking.
