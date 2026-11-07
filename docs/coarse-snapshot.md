# Coarse snapshots (experimental)

The coarse backend answers the same questions from a snapshot that keeps only the entries
that matter. Everything smaller than a threshold is folded into `[small items]` rows whose
sizes are exact sums, so totals never change; only the individual small files and folders
are no longer listed. The snapshot is kilobytes instead of gigabytes, a scan needs a few
tens of megabytes of RAM, and a refresh of a folder rescans that folder, not the disk.

v1 stays the default. The coarse backend is opt-in, keeps its snapshots in
`<snapshot_dir>/coarse/` and never reads or writes v1's or v2's files.

## Using it

For one command, pass the backend; for the nightly timer, save it in the config:

<!-- no-run -->
```sh
sudo freespace --backend coarse install-timer
freespace --backend coarse ls /home
sudo freespace --backend coarse refresh /home/example/Downloads
```

`install-timer` writes `backend = "coarse"` and `coarse_ppm` to
`/etc/freespace/config.toml`; after that every command, `gui` and `serve` included, uses the
coarse snapshot. `--backend v1` shows the other one. The folding threshold is a share
of the size of the filesystem, in parts per million:

<!-- no-run -->
```sh
sudo freespace --backend coarse scan --coarse-ppm 500
```

The default, 1000, is 0.1 % (440 MB on a 440 GB disk). Smaller values keep more entries
and make the snapshot larger. `--cross-mounts` is not available: a coarse scan stops at
every other filesystem and shows it as a `mount` entry, so name it as a root of its own
(`freespace scan / /mnt/data`). A root that is a mount point inside another root appears
twice, as in v1: as a `mount` entry of 0 bytes in the outer tree and as the root of its own
tree. `--workers` and `--low-memory` do not apply either: the walk is one thread and never
stages entries on disk.

## What is kept and what is folded

A scan keeps:

- every scan root;
- every entry whose subtree takes at least the threshold of disk space;
- every folder with more than 4096 entries below it, so that the small entries inside can
  be refreshed a few thousand at a time;
- every unreadable folder and every mount point, however small, and the folders that lead
  to any kept entry.

Everything else is folded into the kept folder that holds it. The entries of a folder are
spread over 4096 slots by a hash of their name, and neighbouring slots are grouped into
buckets of at most 4096 entries. Each non-empty bucket becomes one row, `[small items]` or
`[small items 0-1023]` when a folder has several, with the exact allocated and apparent
bytes, the number of entries and the oldest scan time of what it holds.

Hardlinks are counted as in v1: the path that sorts first carries the bytes, the others
show 0, even when they are folded.

## What the answers say about it

Every answer of a coarse snapshot carries `folded_below`, the size under which entries are
folded, and `ls` marks the stand-in rows with `folded: true` (`items` counts the entries
inside). Terminal output prints a note before the numbers, as it does for an incomplete
scan.

- **Totals are exact.** A folder's `allocated`, `apparent` and `items` equal what a full
  scan reports, folded entries included.
- **`ls` shows the kept entries and the buckets.** The buckets sort among them by size.
- **`top` and `find` see only kept entries.** A folded file is not listed and not matched
  by `find --name`; report counts as "at least" and do not conclude that a name does not
  exist. The `[small items]` rows are notes on what was folded, not entries: no filter
  matches them and no count includes them. `top --kind dir --older-than` works on kept
  folders.
- **A folded path is `PATH_NOT_FOUND`**, and the message says it may be folded; `fix` names
  the nearest kept folder, whose `ls` shows the bucket.
- **Counts differ in one place.** `status` shows the rows the snapshot file keeps as its
  `entries`; a folder's `items` and the `entries` of `scan --json` count everything on disk.
- **`diff` compares kept entries.** An entry that grows past the threshold appears as new
  and its old bucket shrinks; the totals of every folder still add up.

## Refresh

`refresh <path>` finds the smallest part of the snapshot that holds the path and rescans
that part:

| The path is | What is rescanned |
|---|---|
| a kept folder | that folder and everything below it |
| folded into a bucket | the entries of that bucket only (a few thousand) |
| a scan root | the whole root |
| gone from disk | the kept entries under it are dropped and the folder or bucket that held it is rescanned |

The result is merged into the current snapshot, so a refresh costs what the rescanned part
costs. The command prints a note when it rescanned more than the path. A part that contains
hardlinked files is the exception: their bytes belong to whichever path sorts first, which
may be outside the part, so such a refresh rescans the whole scan root (the note says so).
If a folder is gone and a scan root was inside it, the refresh stops and asks for a full
scan.

Refresh publishes a new generation and never rotates `prev`, as in v1.

## Cost

Measured with a prototype of this design (the shipped backend has the same algorithm but
has not been timed on a whole disk), on one busy machine: an ext4 disk of 474 GB (441 GiB)
with 9.78 million entries, single runs, mostly cold caches. v1 ran as `scan --low-memory
--workers 1`, the command that machine's configuration implies. Read the percentages as
"about 20 % either way".

| | v1 | coarse (0.1 %) |
|---|---|---|
| snapshot | 1.37 GiB | 3.0 MiB |
| disk written by a scan | 15.6 GB (3.0 GiB scratch while it runs) | 3.3 MB |
| scan, wall time | 2685 s | 3265 s (+22 %) |
| scan, peak RSS | 24 MiB | 32 MiB |
| refresh of a folder | 66.6 s, 1.8 GiB RSS | 0.3 to 2.4 s, 14 MiB RSS (a whole-root rescan when the part holds hardlinks) |

So the gain is the footprint on disk and the refresh, not the scan: the walk takes as long
as v1's, twice, because the first pass finds the hardlink owners and the second folds the
tree. On that disk 25 % of the folded entries and half of the kept folders sit where a
refresh finds hardlinks and rescans its root, which takes about as long as a scan.

The process memory of a scan does not grow with the number of entries, only with the
hardlinked inodes (160 to 173 bytes each) and the depth of the tree. In a cgroup that also
held the page cache and the kernel's directory caches, a scan of one million entries
completed at 48 MiB and one with 300 000 hardlinked inodes at 128 MiB. Millions of
distinct hardlinked inodes (backup trees, container layer stores) need correspondingly
more.

What the small snapshot gives up is names. At 0.1 % on that disk, 70.8 % of the bytes sat
in folded entries and only the largest files and folders could be named. A smaller
threshold names more and costs little (estimates from the entries of a v1 snapshot of that
disk, size as a floor):

| `--coarse-ppm` | Threshold on that disk | Snapshot, about | Bytes under a name |
|---|---|---|---|
| 1000 (default) | 474 MB | 3 MiB | 29 % |
| 100 | 47 MB | 4.5 MiB | 51 % |
| 10 | 4.7 MB | 15 MiB | 70 % |
| 3 | 1.4 MB | 37 MiB | 77 % |

## Limits

- At most 200 000 kept entries per snapshot; a scan that would keep more fails with a
  `USAGE` error that suggests a larger `--coarse-ppm`.
- The walk is one thread, so a scan on a fast SSD is slower than v1's parallel walk.
- Each root folds below the threshold of its own filesystem; `folded_below` reports the
  largest of them.
- `maintain` is v2 only. `doctor` checks the coarse snapshot like v1's.

## Format

A coarse snapshot is a v1 snapshot file: the same tables, the same `schema_version`, read
by the same queries. Three side tables describe the folding: `coarse_meta` (format, ppm),
`coarse_roots` (threshold and capacity per root) and `coarse_buckets` (the slot range,
totals and row of every bucket). `refresh` rebuilds its model from these tables, verifies
that every folder's totals still add up, and writes the merged result as a new snapshot.
The stand-in rows have kind `other` and the flag `BUCKET`, which only `ls` lists.
