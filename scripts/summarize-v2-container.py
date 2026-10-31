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
            and row.get("observer_validated", True) is True
            and not row.get("timed_out") and not row.get("sampler_errors")
            and not int(events.get("oom", 0)) and not int(events.get("oom_kill", 0)))


def read_evidence(path, directory, issues):
    try:
        value = json.loads(path.read_text())
        if not isinstance(value, dict):
            raise ValueError("expected a JSON object")
        return value
    except (OSError, ValueError) as error:
        issues.append(f"missing/unreadable evidence: {path.relative_to(directory)}: {error}")
        return {}


def completion_evidence(directory, paths, issues):
    """Require terminal receipts, including when a CLI finished before interruption."""
    specs = sorted(directory.rglob("*.spec.json"))
    manifests = sorted(directory.rglob("manifest.json"))
    roots = {path.parent for path in specs + manifests} | {path.parent.parent for path in paths}
    if not specs:
        issues.append("no arm specifications found")
    strict_observers = set()
    for root in sorted(roots):
        status = read_evidence(root / "status.json", directory, issues)
        if status.get("success") is not True or status.get("completed") is not True or status.get("failures"):
            issues.append(f"{(root / 'status.json').relative_to(directory)}: incomplete/failed experiment")
        manifest = read_evidence(root / "manifest.json", directory, issues)
        if manifest.get("measurement_protocol") == "host-shared-fixture-cold-arm-integrity-v4":
            strict_observers.add(root)
    expected_metrics = set()
    for path in specs:
        spec = read_evidence(path, directory, issues)
        ident = spec.get("id")
        operations = spec.get("operations")
        if not isinstance(ident, str) or not isinstance(operations, list) or not operations:
            issues.append(f"invalid arm specification: {path.relative_to(directory)}")
            continue
        for index, operation in enumerate(operations):
            expected = path.parent / f"{ident}-{index}-{operation}" / "metrics.json"
            expected_metrics.add(expected)
            if not expected.exists():
                issues.append(f"missing command metrics: {expected.relative_to(directory)}")
        end = read_evidence(path.parent / (ident + ".arm-end.json"), directory, issues)
        try:
            events = dict(line.split() for line in end["memory.events"].splitlines())
            healthy = not int(events.get("oom", 0)) and not int(events.get("oom_kill", 0))
        except (KeyError, ValueError):
            healthy = False
        if not healthy:
            issues.append(f"missing/invalid final cgroup state: {ident}")
        state = read_evidence(path.parent / (ident + ".docker-state.json"), directory, issues)
        if (state.get("Status") != "exited" or state.get("ExitCode") != 0
                or state.get("Running") is not False or state.get("OOMKilled") is not False
                or state.get("Error")):
            issues.append(f"missing/failed final Docker state: {ident}")
    for path in paths:
        if path not in expected_metrics:
            issues.append(f"command has no matching arm specification: {path.relative_to(directory)}")
    # A combined-profile wrapper also has a status receipt without a manifest.
    for path in sorted(directory.rglob("status.json")):
        if path.parent not in roots:
            status = read_evidence(path, directory, issues)
            if status.get("success") is not True:
                issues.append(f"{path.relative_to(directory)}: incomplete/failed wrapper")
    return strict_observers


def observer_complete(path, row, directory, issues, strict):
    observation = read_evidence(path.parent / "dbstat.json", directory, issues)
    databases = observation.get("databases")
    complete = (observation.get("logical_content_unchanged") is True
                and isinstance(databases, dict) and bool(databases)
                and not observation.get("errors")
                and observation.get("validated", not strict) is True
                and row.get("observer_validated", not strict) is True)
    if isinstance(databases, dict):
        complete = complete and all(isinstance(details, dict) and not any(
            key in details for key in ("error", "unavailable", "dbstat_error"))
            for details in databases.values())
    if not complete:
        issues.append(f"incomplete/failed observer: {path.parent.relative_to(directory)}")
    return complete


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    groups = defaultdict(list)
    issues = []
    paths = sorted(args.directory.rglob("metrics.json"))
    if not paths:
        issues.append("no command metrics found")
    strict_observers = completion_evidence(args.directory, paths, issues)
    for path in paths:
        row = read_evidence(path, args.directory, issues)
        if not all(key in row for key in ("arm", "keep_previous", "files", "operation", "elapsed_seconds")):
            issues.append(f"incomplete command metrics: {path.relative_to(args.directory)}")
            continue
        if not observer_complete(path, row, args.directory, issues, path.parent.parent in strict_observers):
            row["observer_validated"] = False
        match = re.search(r"-(\d+)-(scan|refresh)$", row.get("id", ""))
        ordinal = int(match[1]) + 1 if match else 0
        key = (row.get("scan_profile", "disk"), row["arm"], row["keep_previous"], row["files"], ordinal, row["operation"])
        groups[key].append(row)
    print("Values: median [min..max]; times seconds, memory/disk MiB. Disk peaks are sampled lower bounds.")
    scopes = sorted({row.get("memory_peak_scope", "unspecified in artifact")
                     for rows in groups.values() for row in rows})
    print("Cgroup peak scope from raw artifacts: " + "; ".join(scopes) + ". RSS is per CLI.")
    print("\n| scan profile | arm | keep | files | command | valid/rows | elapsed | RSS | cgroup peak | sampled current peak |")
    print("|---|---|---|---:|---|---:|---|---|---|---|")
    for key, rows in sorted(groups.items()):
        profile, arm, keep, files, ordinal, operation = key
        accepted = [row for row in rows if valid(row)]
        count = len(accepted)
        if count != len(rows):
            issues.append(f"invalid command rows: {key}: {len(rows) - count}")
        rss = [row["time"]["peak_rss_kib"] for row in accepted if "peak_rss_kib" in row.get("time", {})]
        cg = [int(row["cgroup_after"]["memory.peak"]) for row in accepted if "memory.peak" in row.get("cgroup_after", {})]
        current = [row["sampled_memory_peak"] for row in accepted if "sampled_memory_peak" in row]
        print(f"| {profile} | {arm} | {str(keep).lower()} | {files} | {ordinal}:{operation} | {count}/{len(rows)} | "
              f'{span([row["elapsed_seconds"] for row in accepted])} | {span(rss, 1024)} | '
              f"{span(cg, 1048576)} | {span(current, 1048576)} |")
    for key, rows in sorted(groups.items()):
        profile, arm, keep, files, ordinal, operation = key
        accepted = [row for row in rows if valid(row)]
        print(f"\nscan_profile={profile}, {arm}, keep_previous={str(keep).lower()}, files={files}, command={ordinal}:{operation}")
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
