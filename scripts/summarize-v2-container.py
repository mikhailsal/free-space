#!/usr/bin/env python3
"""Summarize downloaded v2-balance raw artifacts; no commands or scans run.

Usage: python3 scripts/summarize-v2-container.py DOWNLOADED_ARTIFACT_DIRECTORY
Values are median [min..max]. Bytes are reported in MiB; RSS in MiB. Each scan
ordinal is separate so fresh-seed cost is not mixed with repeated-scan cost.
Exit 1 indicates failed/incomplete raw results, including missing command rows.
"""

import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import statistics


def span(values, scale=1, digits=3):
    if not values:
        return "unavailable"
    values = [value / scale for value in values]
    middle, low, high = statistics.median(values), min(values), max(values)
    return f"{middle:.{digits}f} [{low:.{digits}f}..{high:.{digits}f}]"


def valid(row):
    events = dict(line.split() for line in row.get("cgroup_after", {}).get("memory.events", "").splitlines())
    return (row.get("validated") is True and row.get("exit_code") == 0
            and not row.get("timed_out") and not row.get("sampler_errors")
            and not int(events.get("oom", 0)) and not int(events.get("oom_kill", 0)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    groups = defaultdict(list)
    issues = []
    paths = sorted(args.directory.rglob("metrics.json"))
    if not paths:
        issues.append("no command metrics found")
    for path in paths:
        row = json.loads(path.read_text())
        match = re.search(r"-(\d+)-(scan|refresh)$", row.get("id", ""))
        ordinal = int(match[1]) + 1 if match else 0
        key = (row["arm"], row["keep_previous"], row["files"], ordinal, row["operation"])
        groups[key].append(row)
    for path in sorted(args.directory.rglob("*.spec.json")):
        spec = json.loads(path.read_text())
        for index, operation in enumerate(spec.get("operations", [])):
            expected = path.parent / f'{spec["id"]}-{index}-{operation}' / "metrics.json"
            if not expected.exists():
                issues.append(f"missing command metrics: {expected.relative_to(args.directory)}")
    for path in sorted(args.directory.rglob("status.json")):
        status = json.loads(path.read_text())
        if not status.get("success"):
            issues.append(f'{path.relative_to(args.directory)}: incomplete/failed; {status.get("failures", [])}')
    print("Values: median [min..max]; times seconds, memory/disk MiB. Disk peaks are sampled lower bounds.")
    scopes = sorted({row.get("memory_peak_scope", "unspecified in artifact")
                     for rows in groups.values() for row in rows})
    print("Cgroup peak scope from raw artifacts: " + "; ".join(scopes) + ". RSS is per CLI.")
    print("\n| arm | keep | files | command | valid/rows | elapsed | RSS | cgroup peak | sampled current peak |")
    print("|---|---|---:|---|---:|---|---|---|---|")
    for key, rows in sorted(groups.items()):
        arm, keep, files, ordinal, operation = key
        accepted = [row for row in rows if valid(row)]
        count = len(accepted)
        if count != len(rows):
            issues.append(f"invalid command rows: {key}: {len(rows) - count}")
        rss = [row["time"]["peak_rss_kib"] for row in accepted if "peak_rss_kib" in row.get("time", {})]
        cg = [int(row["cgroup_after"]["memory.peak"]) for row in accepted if "memory.peak" in row.get("cgroup_after", {})]
        current = [row["sampled_memory_peak"] for row in accepted if "sampled_memory_peak" in row]
        print(f"| {arm} | {str(keep).lower()} | {files} | {ordinal}:{operation} | {count}/{len(rows)} | "
              f'{span([row["elapsed_seconds"] for row in accepted])} | {span(rss, 1024)} | '
              f"{span(cg, 1048576)} | {span(current, 1048576)} |")
    for key, rows in sorted(groups.items()):
        arm, keep, files, ordinal, operation = key
        accepted = [row for row in rows if valid(row)]
        print(f"\n{arm}, keep_previous={str(keep).lower()}, files={files}, command={ordinal}:{operation}")
        print("\n| component | peak logical | peak allocated | end logical | end allocated |")
        print("|---|---|---|---|---|")
        for component in ("total", "db", "wal", "stage", "shm", "other"):
            values = []
            for metric, quantity in (("sampled_disk_peak", "logical_bytes"),
                                     ("sampled_disk_peak", "allocated_bytes"),
                                     ("endpoint_disk", "logical_bytes"),
                                     ("endpoint_disk", "allocated_bytes")):
                values.append(span([row[metric][component][quantity] for row in accepted
                                    if quantity in row.get(metric, {}).get(component, {})], 1048576))
            print(f'| {component} | {" | ".join(values)} |')
        print()
    if issues:
        print("INVALID / INCOMPLETE RESULTS:")
        for issue in issues:
            print(f"- {issue}")
    else:
        print(f"All {len(paths)} command rows valid; no missing expected command metrics.")
    return 1 if issues else 0


if __name__ == "__main__":
    raise SystemExit(main())
