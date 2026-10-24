#!/usr/bin/env bash
# Line coverage per function of the given source files, from `cargo llvm-cov
# report --json` (the coverage job of .github/workflows/refactor-guard.yml).
#
#   scripts/coverage-functions.sh <llvm-cov-export.json> <file>...
#
# Prints TSV, one row per function: file, first line, name, lines, covered.
# Lines are counted as `llvm-cov show` marks them (its LineCoverageStats over the
# file's segments), each line once, under the innermost function whose code
# spans it. The export's own file total counts per function instead, so there a
# line two functions share (a closure on its parent's line) counts twice. Names
# are demangled by c++filt (binutils), crate hashes dropped.
set -euo pipefail

[ "$#" -ge 2 ] || { echo "usage: $0 <llvm-cov-export.json> <file>..." >&2; exit 2; }
json=$1
shift
command -v jq >/dev/null || { echo "$0: jq is required" >&2; exit 2; }
command -v c++filt >/dev/null || { echo "$0: c++filt (binutils) is required" >&2; exit 2; }

# A segment is [line, col, count, has_count, is_region_entry, is_gap_region].
# shellcheck disable=SC2016 # $files, $f, $line and the rest are jq's
JQ_FUNCTIONS='
def starts_region: .[3] and .[4] and (.[5] | not);
# [line, count] of every mapped line: the segment active when the line starts
# (wrapped) and the regions starting on it decide, as in llvm-cov.
def mapped_lines:
  group_by(.[0])
  | reduce .[] as $g ({wrapped: null, prev: 0, out: []};
      $g[0][0] as $line
      | (if .wrapped != null and .wrapped[3] then .out += [range(.prev + 1; $line) as $l | [$l, .wrapped[2]]] else . end)
      | ([$g[] | select(starts_region) | .[2]]) as $starts
      | (($g[0][3] | not) and $g[0][4]) as $skipped
      | (if $skipped or ((.wrapped == null or (.wrapped[3] | not)) and ($starts | length) == 0) then .
         else .out += [[$line, ([(.wrapped // [0, 0, 0])[2]] + $starts | max)]] end)
      | .wrapped = $g[-1] | .prev = $line)
  | .out;
.data[0] as $d
| $d.files[]
| .filename as $path
| ($files | map(. as $f | select($path == $f or ($path | endswith("/" + $f)))) | first) as $file
| select($file != null)
# copies of a function (from two binaries, or a generic one per type) share a
# span, so they share a row
| ([$d.functions[] | select(.filenames[0] == $path)
    | [.regions[] | select(.[5] == 0 and .[7] == 0)] as $rs
    | select($rs | length > 0)
    | {start: ($rs | map(.[0]) | min), end: ($rs | map(.[2]) | max), name}]) as $fns
# a tie (copies of a generic function share a span) goes to the least mangled name
| [(.segments | mapped_lines)[] as [$line, $count]
    | ([$fns[] | select(.start <= $line and .end >= $line)] | min_by([.end - .start, -.start, .name])) as $fn
    | select($fn != null)
    | {key: "\($fn.start) \($fn.end)", fn: $fn, hit: ($count > 0)}]
| group_by(.key)[]
| [$file, .[0].fn.start, .[0].fn.name, length, (map(select(.hit)) | length)]
| @tsv'

files=$(jq -n '$ARGS.positional' --args "$@")
jq -r --argjson files "$files" "$JQ_FUNCTIONS" "$json" |
  c++filt |
  sed -E 's/\[[0-9a-f]+\]//g' |
  sort -t "$(printf '\t')" -k1,1 -k2,2n -k3,3
