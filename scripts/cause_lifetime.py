"""Sampled disk lifetimes on the hosted diagnostic case; no payload FDs retained."""
import copy
import math
import os
from pathlib import Path
import stat

from cause_observe import (PinSet, _guard, _identity, _open_dir, _read, _stamp,
                           capture_cgroup, record_inventory)


class LifetimeWatch:
    """Poll from the host's existing cadence; finish never invents a final sample.

    Real proc/sys access is hosted-only. FixtureAccess + injected roots support
    tiny tests. Separate sampled maxima below must never be added together.
    """
    def __init__(self, snapshot, pins, host_pid, *, proc_root="/proc",
                 cgroup_root="/sys/fs/cgroup", fixture=None, interval_seconds=0.05):
        _guard([snapshot, proc_root, cgroup_root], fixture)
        if not isinstance(pins, PinSet) or Path(snapshot).absolute() != pins.root:
            raise ValueError("watcher needs the matching live PinSet")
        if not 0.01 <= interval_seconds <= 1:
            raise ValueError("poll interval outside bounded range")
        self.snapshot, self.pins = str(pins.root), pins
        self.proc_root, self.fixture = Path(proc_root), fixture
        self.group = capture_cgroup(host_pid, proc_root=proc_root, cgroup_root=cgroup_root,
                                    fixture=fixture)
        if not self.group["process_identity_stable"]:
            raise ValueError("host PID identity/membership was not stable")
        self.interval_ns = int(interval_seconds * 1e9)
        self.started, self.last_start = _stamp(), None
        self.states, self.transitions, self.errors = {}, [], []
        self.known_named = ("snapshot.db", "snapshot.prev.db", "snapshot.stage.tmp", "snapshot.prev.tmp")
        self.maxima = dict(combined=None, named_snapshot=None, open_deleted=None)
        self.counts = dict(polls=0, complete_polls=0, errors=0, races=0, limits=0,
                           missed_poll_intervals_estimate=0, maximum_gap_ns=0,
                           maximum_sample_duration_ns=0, descriptors_examined=0,
                           pids_examined=0, transition_records_dropped=0,
                           error_records_dropped=0)
        self.finished = None

    def _issue(self, kind, detail, **context):
        self.counts[kind] += 1
        self.counts["errors"] += kind != "errors"
        row = dict(kind=kind, detail=detail, **context)
        if len(self.errors) < 256:
            self.errors.append(row)
        else:
            self.counts["error_records_dropped"] += 1

    @staticmethod
    def _ticks(source):
        try:
            return source["raw"].rsplit(")", 1)[1].split()[19]
        except (KeyError, IndexError):
            return None

    def _descriptors(self):
        """Only exact cgroup.procs PIDs; bounded traversal of each fd directory."""
        rows, states = {}, {}
        ok, total_fds = True, 0
        group_fd = _open_dir(self.group["cgroup_path"])
        try:
            actual = os.fstat(group_fd)
            expected = self.group["group_identity"]
            if (actual.st_dev, actual.st_ino) != (expected["device"], expected["inode"]):
                raise ValueError("cgroup directory identity changed")
            membership = _read("cgroup.procs", dir_fd=group_fd)
        finally:
            os.close(group_fd)
        if not membership["available"]:
            raise ValueError("cgroup.procs unavailable: " + membership.get("error", ""))
        lines = membership["raw"].splitlines()
        if len(lines) > 128:
            self._issue("limits", "more than 128 cgroup PIDs")
            ok, lines = False, lines[:128]
        pids = []
        for line in lines:
            if not line.isdecimal() or int(line) <= 0:
                self._issue("errors", "invalid cgroup.procs PID", value=line)
                ok = False
            elif int(line) not in pids:
                pids.append(int(line))
        for pid in pids:
            self.counts["pids_examined"] += 1
            pid_dir, fd_dir = None, None
            try:
                pid_dir = _open_dir(self.proc_root / str(pid))
                before = _read("stat", dir_fd=pid_dir)
                member_before = _read("cgroup", dir_fd=pid_dir)
                expected_membership = self.group["membership"]["raw"]
                if not member_before["available"] or member_before["raw"] != expected_membership:
                    raise ValueError("listed PID no longer belongs to exact worker cgroup")
                fd_dir = os.open("fd", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 dir_fd=pid_dir)
                with os.scandir(fd_dir) as entries:
                    for count, entry in enumerate(entries, 1):
                        if count > 128 or total_fds >= 1024:
                            self._issue("limits", "descriptor bound exceeded", pid=pid)
                            ok = False
                            break
                        if not entry.name.isdecimal():
                            self._issue("errors", "non-numeric proc FD", pid=pid)
                            ok = False
                            continue
                        total_fds += 1
                        self.counts["descriptors_examined"] += 1
                        try:
                            target = os.readlink(entry.name, dir_fd=fd_dir)
                            if self.fixture is not None:
                                resolved = Path(target.removesuffix(" (deleted)"))
                                if not resolved.is_absolute():
                                    resolved = self.proc_root / str(pid) / "fd" / resolved
                                self.fixture.check(resolved)
                            s = os.stat(entry.name, dir_fd=fd_dir, follow_symlinks=True)
                            # A second stat/readlink detects observed descriptor reuse.
                            again = os.stat(entry.name, dir_fd=fd_dir, follow_symlinks=True)
                            if ((s.st_dev, s.st_ino) != (again.st_dev, again.st_ino)
                                    or os.readlink(entry.name, dir_fd=fd_dir) != target):
                                raise ValueError("descriptor identity changed during observation")
                            if not stat.S_ISREG(s.st_mode) or s.st_dev != self.pins.root_identity["device"]:
                                continue
                            key = f"{s.st_dev}:{s.st_ino}"
                            alias = dict(kind="container_fd", pid=pid, fd=int(entry.name), target=target)
                            if key in rows:
                                rows[key]["aliases"].append(alias)
                            else:
                                rows[key] = dict(observation=_identity(s), aliases=[alias], raced=False)
                            states[f"fd:{pid}:{entry.name}"] = dict(device=s.st_dev, inode=s.st_ino,
                                unlinked=s.st_nlink == 0, target=target)
                        except (FileNotFoundError, ProcessLookupError, ValueError) as error:
                            self._issue("races", str(error), pid=pid, fd=entry.name)
                            ok = False
                        except OSError as error:
                            self._issue("errors", str(error), pid=pid, fd=entry.name)
                            ok = False
                after = _read("stat", dir_fd=pid_dir)
                member_after = _read("cgroup", dir_fd=pid_dir)
                if (self._ticks(before) is None or self._ticks(before) != self._ticks(after)
                        or not member_after["available"] or member_after["raw"] != expected_membership):
                    self._issue("races", "PID identity or cgroup membership changed", pid=pid)
                    ok = False
            except (FileNotFoundError, ProcessLookupError, ValueError) as error:
                self._issue("races", str(error), pid=pid)
                ok = False
            except OSError as error:
                self._issue("errors", str(error), pid=pid)
                ok = False
            finally:
                if fd_dir is not None:
                    os.close(fd_dir)
                if pid_dir is not None:
                    os.close(pid_dir)
        return rows, states, ok, pids

    def _transition(self, present, before, after, named_complete, fd_complete):
        all_keys = set(self.states) | set(present)
        for key in sorted(all_keys):
            if key in present:
                current = present[key]
            elif (key.startswith("fd:") and fd_complete) or (not key.startswith("fd:") and named_complete):
                current = dict(state="absent")
            else:
                current = dict(state="unavailable", reason="incomplete sample")
            if self.states.get(key) != current:
                if len(self.transitions) < 2048:
                    self.transitions.append(dict(path=key, before=before, after=after,
                                                 previous=self.states.get(key), current=current))
                else:
                    self.counts["transition_records_dropped"] += 1
                self.states[key] = current
        if len(self.states) > 2048:
            self._issue("limits", "path identity state bound exceeded")
            self.states = dict(list(self.states.items())[-2048:])

    @staticmethod
    def _selected(entries, predicate):
        return {k: row for k, row in entries.items() if predicate(row)}

    def poll(self):
        if self.finished is not None:
            raise ValueError("watcher already finished")
        if self.counts["polls"] >= 50000:
            self._issue("limits", "50000 poll bound exceeded")
            return dict(available=False, error="poll bound exceeded")
        before = _stamp()
        if self.last_start is not None:
            gap = before["monotonic_ns"] - self.last_start
            self.counts["maximum_gap_ns"] = max(self.counts["maximum_gap_ns"], gap)
            self.counts["missed_poll_intervals_estimate"] += max(0, math.floor(gap / self.interval_ns) - 1)
        self.last_start = before["monotonic_ns"]
        self.counts["polls"] += 1
        entries, present, named_complete, fd_complete = {}, {}, False, False
        inventory = None
        try:
            inventory = record_inventory(self.snapshot, self.pins)
            entries = copy.deepcopy(inventory["entries"])
            named_complete = inventory["complete"]
            for issue in inventory["errors"]:
                self._issue("races" if issue.get("race") else "errors",
                            "named inventory error", observation=issue)
            for issue in inventory["excluded"]:
                self._issue("errors", "named inventory exclusion", observation=issue)
            for row in entries.values():
                if row["raced"]:
                    self._issue("races", "named/pin observation changed within traversal")
                for alias in row["aliases"]:
                    if alias["kind"] in ("named", "inspection_alias"):
                        key = alias["kind"] + ":" + alias["path"]
                    else:
                        key = "pin:" + alias["name"]
                    s = row["observation"]
                    present[key] = dict(device=s["device"], inode=s["inode"], unlinked=s["nlink"] == 0)
            if not inventory["root_path_matches"]:
                self._issue("races", "snapshot root pathname replaced")
            fd_rows, fd_states, fd_complete, pids = self._descriptors()
            present.update(fd_states)
            for key, row in fd_rows.items():
                if key in entries:
                    old = entries[key]
                    if old["observation"] != row["observation"]:
                        self._issue("races", "named/pin/FD observations changed within bracket", inode=key)
                        fd_complete = False
                        old["raced"] = True
                    old["aliases"].extend(row["aliases"])
                else:
                    entries[key] = row
        except (OSError, ValueError) as error:
            self._issue("errors", str(error))
            pids = None
        after = _stamp()
        duration = after["monotonic_ns"] - before["monotonic_ns"]
        self.counts["maximum_sample_duration_ns"] = max(self.counts["maximum_sample_duration_ns"], duration)
        complete = named_complete and fd_complete
        self.counts["complete_polls"] += complete
        if named_complete:
            for path in self.known_named:
                present.setdefault("named:" + path, dict(state="absent"))
        self._transition(present, before, after, named_complete, fd_complete)
        selections = dict(combined=entries,
            named_snapshot=self._selected(entries, lambda r: any(a["kind"] == "named" for a in r["aliases"])),
            open_deleted=self._selected(entries, lambda r: r["observation"]["nlink"] == 0))
        for category, selected in selections.items():
            if not selected and (inventory is None or not complete):
                continue  # Unknown coverage is not an observed zero/absence.
            allocated = sum(r["observation"]["allocated_bytes"] for r in selected.values())
            peak = self.maxima[category]
            if peak is None or allocated > peak["allocated_bytes"]:
                self.maxima[category] = dict(before=before, after=after, complete=complete, available=True,
                    allocated_bytes=allocated, entries=copy.deepcopy(selected),
                    semantics="maximum bracketed observed aggregate; simultaneity is not guaranteed",
                    pids=pids, inventory_brackets=None if inventory is None else dict(
                        before=inventory["before"], after=inventory["after"],
                        filesystem_before=inventory["filesystem_before"], filesystem_after=inventory["filesystem_after"]))
        return dict(before=before, after=after, complete=complete, pids=pids,
                    observed_inode_count=len(entries))

    def finish(self):
        if self.finished is None:
            self.finished = dict(schema=1, started=self.started, finished=_stamp(),
                scope="diagnostic retained-FD/hardlink pass; exact host cgroup PIDs, measured filesystem only",
                root_path=self.snapshot, root_identity=self.pins.root_identity,
                cgroup_path=self.group["cgroup_path"], group_identity=self.group["group_identity"],
                host_pid=self.group["pid"], interval_ns=self.interval_ns,
                available=self.counts["polls"] > 0, coverage_complete=(self.counts["polls"] > 0
                    and self.counts["complete_polls"] == self.counts["polls"]
                    and self.counts["errors"] == 0 and self.counts["transition_records_dropped"] == 0
                    and self.counts["missed_poll_intervals_estimate"] == 0),
                counts=dict(self.counts), transitions=copy.deepcopy(self.transitions),
                errors=copy.deepcopy(self.errors), maxima=copy.deepcopy(self.maxima),
                limits=dict(polls=50000, pids=128, descriptors_per_pid=128,
                            descriptors_per_poll=1024, transitions=2048, error_records=256),
                limitation="statvfs and per-inode observations are sampled lower bounds; bracketed aggregate inventories are not atomic or guaranteed simultaneous true-peak bounds; missed or unreadable sources never prove absence; separate maxima must not be summed; pin/alias intervention changes retention, so no ordinary peak/RAM/speed claim")
        return copy.deepcopy(self.finished)
