#!/usr/bin/env bash
# Scan benchmark (T1, Next Steps 1): du, gdu, duc, ncdu and freespace on one
# root, cold (after drop_caches) and warm (right after a cold run).
#
# Usage: sudo [TOOLS="du gdu duc ncdu freespace"] [FREESPACE=./target/release/freespace] \
#             [JSON_DIR=dir] scripts/bench-baseline.sh [ROOT] > results.tsv
# Prints TSV rows: tool, config, cache, wall_seconds, peak_rss_kb, exit.
# freespace writes its snapshots to $FREESPACE_SNAPSHOT_DIR (default
# /var/tmp/freespace-bench) and, with JSON_DIR, its --json report per run.
set -u
ROOT=${1:-/}
TOOLS=${TOOLS:-du gdu duc ncdu freespace}
FREESPACE=${FREESPACE:-./target/release/freespace}
export FREESPACE_SNAPSHOT_DIR=${FREESPACE_SNAPSHOT_DIR:-/var/tmp/freespace-bench}
export FREESPACE_CONFIG=${FREESPACE_CONFIG:-/nonexistent}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ "$(id -u)" -ne 0 ]; then
  echo "run as root: drop_caches and a full scan of $ROOT need it" >&2
  exit 1
fi

has() { case " $TOOLS " in *" $1 "*) return 0 ;; esac; return 1; }
drop() { sync; echo 3 > /proc/sys/vm/drop_caches; sleep 2; }

# run TOOL CONFIG CACHE OUTFILE CMD...
run() {
  local tool=$1 config=$2 cache=$3 out=$4 wall rss code
  shift 4
  [ "$cache" = cold ] && drop
  /usr/bin/time -f '%e %M %x' -o "$TMP/time" "$@" >"$out" 2>"$TMP/err"
  # time(1) prepends "Command exited with non-zero status N" on failure: keep the last line
  read -r wall rss code < <(tail -n 1 "$TMP/time")
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$tool" "$config" "$cache" "$wall" "$rss" "$code"
}

printf 'tool\tconfig\tcache\twall_s\tpeak_rss_kb\texit\n'
if has du; then
  run du "-sx" cold /dev/null du -sx "$ROOT"
  run du "-sx" warm /dev/null du -sx "$ROOT"
fi
if has gdu; then
  run gdu "-x -m 8" cold /dev/null gdu -n -p -s -x -m 8 "$ROOT"
  run gdu "-x -m 8" warm /dev/null gdu -n -p -s -x -m 8 "$ROOT"
  run gdu "-x -m 1" cold /dev/null gdu -n -p -s -x -m 1 "$ROOT"
fi
if has duc; then
  run duc "index -x" cold /dev/null duc index -x -d "$TMP/duc.db" "$ROOT"
  rm -f "$TMP/duc.db"
  run duc "index -x" warm /dev/null duc index -x -d "$TMP/duc.db" "$ROOT"
  rm -f "$TMP/duc.db"
fi
if has ncdu; then
  run ncdu "-x -0 -o" cold /dev/null ncdu -x -0 -o "$TMP/ncdu.out" "$ROOT"
  run ncdu "-x -0 -o" warm /dev/null ncdu -x -0 -o "$TMP/ncdu.out" "$ROOT"
  rm -f "$TMP/ncdu.out"
fi
if has freespace; then
  mkdir -p "$FREESPACE_SNAPSHOT_DIR"
  for w in 8 4 2 1; do
    out=/dev/null
    [ -n "${JSON_DIR:-}" ] && out="$JSON_DIR/freespace-w$w-cold.json"
    run freespace "scan --workers $w" cold "$out" "$FREESPACE" scan "$ROOT" --workers "$w" --json
    if [ "$w" = 8 ]; then
      [ -n "${JSON_DIR:-}" ] && out="$JSON_DIR/freespace-w$w-warm.json"
      run freespace "scan --workers $w" warm "$out" "$FREESPACE" scan "$ROOT" --workers "$w" --json
    fi
  done
fi
