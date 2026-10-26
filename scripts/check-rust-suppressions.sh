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


def code_without_comments_or_literals(source: str) -> str:
    """Keep source offsets while hiding tokens inside comments and literals."""
    code = list(source)
    length = len(source)

    def hide(start: int, end: int) -> None:
        for offset in range(start, end):
            if code[offset] != "\n":
                code[offset] = " "

    offset = 0
    while offset < length:
        if source.startswith("//", offset):
            end = source.find("\n", offset)
            end = length if end < 0 else end
            hide(offset, end)
            offset = end
            continue
        if source.startswith("/*", offset):
            depth = 1
            end = offset + 2
            while end < length and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            hide(offset, end)
            offset = end
            continue

        raw = None
        for prefix in ("br", "cr", "r"):
            if source.startswith(prefix, offset):
                quote = offset + len(prefix)
                while quote < length and source[quote] == "#":
                    quote += 1
                if quote < length and source[quote] == '"':
                    raw = (quote, quote - offset - len(prefix))
                    break
        if raw is not None:
            quote, hashes = raw
            terminator = '"' + "#" * hashes
            close = source.find(terminator, quote + 1)
            end = length if close < 0 else close + len(terminator)
            hide(offset, end)
            offset = end
            continue

        if source[offset] == '"':
            end = offset + 1
            while end < length:
                if source[end] == "\\":
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            hide(offset, min(end, length))
            offset = min(end, length)
            continue
        if source[offset] == "'":
            # Consume one character or escape, then require its closing quote.
            # This leaves lifetimes such as 'a visible to the attribute parser.
            end = offset + 1
            if end < length and source[end] == "\\":
                end += 1
                if source.startswith("u{", end):
                    close = source.find("}", end + 2)
                    end = length if close < 0 else close + 1
                elif source.startswith("x", end):
                    end += 3
                else:
                    end += 1
            else:
                end += 1
            if end < length and source[end] == "'":
                end += 1
                hide(offset, end)
                offset = end
            else:
                offset += 1
            continue
        offset += 1
    return "".join(code)


violations = 0
for raw in tracked.split(b"\0"):
    if not raw or not raw.endswith(b".rs"):
        continue
    path = Path(raw.decode(sys.getfilesystemencoding(), errors="surrogateescape"))
    if not path.is_file() or path.is_symlink():
        continue
    source = path.read_text(errors="replace")
    code = code_without_comments_or_literals(source)
    for match in re.finditer(r"#\s*!?\s*\[", code):
        start = match.start()
        depth = 0
        end = match.end() - 1
        while end < len(code):
            if code[end] == "[":
                depth += 1
            elif code[end] == "]":
                depth -= 1
                if depth == 0:
                    end += 1
                    break
            end += 1
        attr = re.sub(r"\s+", "", code[start:end])
        # Rust accepts raw identifiers in lint paths, for example
        # clippy::r#too_many_lines. The r# prefix does not change the lint.
        attr = re.sub(r"(?<!\w)r#(?=[A-Za-z_])", "", attr)
        for lint_attr in re.finditer(r"(?<!\w)(?:allow|expect)\(", attr):
            args_start = lint_attr.end()
            pos = args_start
            parens = 1
            while pos < len(attr) and parens:
                if attr[pos] == "(":
                    parens += 1
                elif attr[pos] == ")":
                    parens -= 1
                pos += 1
            args = attr[args_start:pos - 1] if parens == 0 else attr[args_start:]
            hit = next((lint for lint in blocked if re.search(rf"(?<!\w){re.escape(lint)}(?!\w)", args)), None)
            if hit is None and re.search(r"(?<!\w)warnings(?!\w)", args):
                hit = "warnings"
            if hit is None:
                continue
            line = source.count("\n", 0, start) + 1
            print(f"check-rust-suppressions: {path}:{line}: forbidden {hit} suppression", file=sys.stderr)
            violations += 1
            break

raise SystemExit(1 if violations else 0)
PY
