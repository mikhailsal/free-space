#!/usr/bin/env bash
# Query latency on a full snapshot (T24, the latency targets): each query
# cold (after drop_caches) and warm (right after), --json to /dev/null.
#
# Usage: sudo FREESPACE_SNAPSHOT_DIR=/var/tmp/freespace-bench \
#             [FREESPACE=./target/release/freespace] scripts/bench-queries.sh > queries.tsv
# Prints TSV rows: query, cache, wall_seconds, peak_rss_kb, exit.
set -u
FREESPACE=${FREESPACE:-./target/release/freespace}
export FREESPACE_SNAPSHOT_DIR=${FREESPACE_SNAPSHOT_DIR:-/var/tmp/freespace-bench}
export FREESPACE_CONFIG=${FREESPACE_CONFIG:-/nonexistent}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ "$(id -u)" -ne 0 ]; then
  echo "run as root: drop_caches needs it" >&2
  exit 1
fi

drop() { sync; echo 3 > /proc/sys/vm/drop_caches; sleep 2; }

QUERIES=(
  "status"
  "ls /"
  "top /"
  "top / --min-size 1G"
  "top / --kind dir --older-than 180d"
  "top / --kind dir --older-than 180d --min-size 100M"
  "diff /"
  "find / --name *.iso"
  "find / --name *"
  "find / --older-than 1y"
  "find / --kind dir --name node_modules"
)

printf 'query\tcache\twall_s\tpeak_rss_kb\texit\n'
for q in "${QUERIES[@]}"; do
  read -ra args <<< "$q"
  for cache in cold warm; do
    [ "$cache" = cold ] && drop
    # no globbing: the patterns go to freespace as written
    /usr/bin/time -f '%e %M %x' -o "$TMP/time" "$FREESPACE" "${args[@]}" --json > /dev/null 2> "$TMP/err"
    read -r wall rss code < <(tail -n 1 "$TMP/time")
    printf '%s\t%s\t%s\t%s\t%s\n' "$q" "$cache" "$wall" "$rss" "$code"
  done
done
