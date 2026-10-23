#!/usr/bin/env bash
# Baseline scan benchmark (T1, Next Steps 1): du, gdu, ncdu, duc on one root,
# cold (after drop_caches) and warm (immediately after a cold run).
# Usage: sudo scripts/bench-baseline.sh [ROOT] > results.tsv
# Prints TSV rows: tool, config, cache, wall_seconds, peak_rss_kb, exit
set -u
ROOT=${1:-/}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ "$(id -u)" -ne 0 ]; then
  echo "run as root: drop_caches and a full scan of $ROOT need it" >&2
  exit 1
fi

drop() { sync; echo 3 > /proc/sys/vm/drop_caches; sleep 2; }

run() { # tool config cache cmd...
  local tool=$1 config=$2 cache=$3; shift 3
  [ "$cache" = cold ] && drop
  /usr/bin/time -f '%e %M %x' -o "$TMP/time" "$@" >/dev/null 2>"$TMP/err"
  read -r wall rss code < "$TMP/time"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$tool" "$config" "$cache" "$wall" "$rss" "$code"
}

printf 'tool\tconfig\tcache\twall_s\tpeak_rss_kb\texit\n'
run du    "-sx"          cold du -sx "$ROOT"
run du    "-sx"          warm du -sx "$ROOT"
run gdu   "-x -m 8"      cold gdu -n -p -s -x -m 8 "$ROOT"
run gdu   "-x -m 8"      warm gdu -n -p -s -x -m 8 "$ROOT"
run gdu   "-x -m 2"      cold gdu -n -p -s -x -m 2 "$ROOT"
run gdu   "-x -m 1"      cold gdu -n -p -s -x -m 1 "$ROOT"
run ncdu  "-x -0 -o"     cold ncdu -x -0 -o "$TMP/ncdu.out" "$ROOT"
run ncdu  "-x -0 -o"     warm ncdu -x -0 -o "$TMP/ncdu.out" "$ROOT"
run duc   "index -x"     cold duc index -x -d "$TMP/duc.db" "$ROOT"
rm -f "$TMP/duc.db"
run duc   "index -x"     warm duc index -x -d "$TMP/duc.db" "$ROOT"
