#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  printf 'check-rust-suppressions: run inside a Git work tree\n' >&2
  exit 1
}
cd "$repo_root"

python3 - <<'PY'
from pathlib import Path
import re
import subprocess
import sys

tracked = subprocess.run(["git", "ls-files", "-z"], capture_output=True, check=True).stdout
blocked = (
    "clippy::too_many_lines", "clippy::excessive_nesting", "clippy::all",
    "clippy::pedantic", "clippy::complexity", "clippy::restriction",
    "clippy::nursery", "clippy::cargo", "clippy::perf", "clippy::style",
    "clippy::correctness", "clippy::suspicious",
)
violations = 0
for raw in tracked.split(b"\0"):
    if not raw or not raw.endswith(b".rs"):
        continue
    path = Path(raw.decode(sys.getfilesystemencoding(), errors="surrogateescape"))
    if not path.is_file() or path.is_symlink():
        continue
    source = path.read_text(errors="replace")
    for match in re.finditer(r"(?m)^[ \t]*#\s*!?\s*\[", source):
        start = match.start()
        depth = 0
        end = match.end() - 1
        while end < len(source):
            if source[end] == "[":
                depth += 1
            elif source[end] == "]":
                depth -= 1
                if depth == 0:
                    end += 1
                    break
            end += 1
        attr = re.sub(r"\s+", "", source[start:end])
        if not ("allow(" in attr or "expect(" in attr):
            continue
        hit = next((lint for lint in blocked if lint in attr), None)
        if hit is None and re.search(r"(?:^|[,(])warnings(?:[,)])", attr):
            hit = "warnings"
        if hit is None:
            continue
        line = source.count("\n", 0, start) + 1
        print(f"check-rust-suppressions: {path}:{line}: forbidden {hit} suppression", file=sys.stderr)
        violations += 1

raise SystemExit(1 if violations else 0)
PY
