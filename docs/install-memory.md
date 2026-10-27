# Installation memory policy

`install-timer` automatically selects `low_memory` when the configuration does
not yet contain that key, or when invoked with `--low-memory auto`. It persists
a Boolean so later installations preserve the selection. Explicit `true` and
`false` override the estimate. Standalone `scan` keeps its existing defaults.

The installer reads Linux `MemAvailable`, bounded by total RAM and the remaining
capacity of visible cgroup memory limits. It considers both cgroup v2 and the v1
memory controller, including visible parent limits and their current usage.
It reserves at most one quarter of that available capacity for the estimated
ordinary scan working set.

The working-set estimate is 64 MiB plus 384 bytes per entry for v1, or 1,024 bytes
per entry for v2. These are conservative policy constants, not measured peak
predictions. The entry estimate uses filesystem inode counters, once per device,
without walking the directory tree. Counters cover the filesystem, including
entries outside a selected subtree, so a small subtree can select disk staging.
Unsupported filesystems, unavailable counters or memory information, and arithmetic
overflow select disk staging conservatively.

Disk staging uses one walker and trades additional scan time and temporary disk
space for a smaller process working set. Filesystem cache still consumes memory.
Neither mode imposes a memory limit or guarantees that a scan fits: path lengths,
tree shape, concurrent activity and allocations can change the result. Detection
uses the installer's current environment; a future systemd service can have a
different memory limit. Recalculate in the intended environment or set the mode
explicitly when those budgets differ.

The JSON installation report includes `low_memory` and `low_memory_reason`.
The terminal report also explains the selection. To recompute after adding roots:

```sh
sudo freespace install-timer --roots / /home --low-memory auto
```

To preserve an ordinary scan even on a small machine:

```sh
sudo freespace install-timer --low-memory false
```
