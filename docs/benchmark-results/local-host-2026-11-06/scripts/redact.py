#!/usr/bin/env python3
"""Redact private home-directory paths from benchmark records before they enter this public repository.

Usage: redact.py SRC DST [--drop KEY ...]   (run it inside a checkout of this repository)

SRC is a JSON file or a JSONL file (one object per line). Every string value, and every key, at any depth has the paths
below the running user's home directory replaced by `<home-path-redacted>`, except the ones below this repository's own
checkout, which keep their part inside the repository under `<repo>` (where the checkout sits on the host is as private
as the rest of the home directory). `--drop KEY` removes that key at the top level (used to remove the list of the
biggest folders, whose names are private). Numbers, timings and structure are copied unchanged, so the committed records
reproduce every figure in the report. The unredacted originals stay on the host under /tmp/fsbench/results.
"""
import json
import os
import re
import subprocess
import sys

HOME = os.path.expanduser('~')
REPO = subprocess.run(['git', 'rev-parse', '--show-toplevel'], capture_output=True, text=True, check=True).stdout.strip()
# the longer root first: a checkout below the home directory is matched as the checkout
ROOTS = '|'.join(re.escape(root) for root in sorted({HOME, REPO}, key=len, reverse=True))
PATTERN = re.compile('(?:' + ROOTS + r")(?:/[^\s\"'<>,;:)\]}]*)?")


def scrub(text):
    def replace(match):
        path = match.group(0)
        if path == REPO or path.startswith(REPO + '/'):
            return '<repo>' + path[len(REPO):]
        return '<home-path-redacted>'
    return PATTERN.sub(replace, text)


def walk(value):
    if isinstance(value, str):
        return scrub(value)
    if isinstance(value, list):
        return [walk(v) for v in value]
    if isinstance(value, dict):
        return {scrub(k): walk(v) for k, v in value.items()}
    return value


def main():
    args = sys.argv[1:]
    src, dst = args[0], args[1]
    drop = [args[i + 1] for i, a in enumerate(args) if a == '--drop']
    text = open(src, encoding='utf-8').read()
    try:
        docs = [json.loads(text)]
        jsonl = False
    except json.JSONDecodeError:
        docs = [json.loads(line) for line in text.splitlines() if line.strip()]
        jsonl = True
    out = []
    for doc in docs:
        doc = walk(doc)
        for key in drop:
            doc.pop(key, None)
        out.append(doc)
    with open(dst, 'w', encoding='utf-8') as f:
        if jsonl:
            for doc in out:
                f.write(json.dumps(doc, ensure_ascii=False) + '\n')
        else:
            f.write(json.dumps(out[0], indent=1, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
