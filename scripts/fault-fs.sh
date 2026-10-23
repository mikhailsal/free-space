#!/bin/sh
# Section 6 #2 and the chaos check on a real, small filesystem (T10). Needs
# root for the tmpfs and a debug build of freespace for the pause point:
#   cargo build && sudo scripts/fault-fs.sh target/debug/freespace
# 1. precheck refuses on a full disk and keeps both snapshots byte for byte;
# 2. a disk that fills up while the new snapshot is written keeps the
#    published one untouched, leaves no tmp file, and the next scan on freed
#    space succeeds.
set -eu

[ "$#" -eq 1 ] || { echo "usage: $0 path/to/debug/freespace" >&2; exit 2; }
bin=$(realpath "$1")
t=$(mktemp -d)
cleanup() {
    umount "$t/snap" 2>/dev/null || true
    rm -rf "$t"
}
trap cleanup EXIT INT TERM
fail() {
    echo "FAIL: $*" >&2
    ls -la "$t/snap" >&2 || true
    exit 1
}

mkdir -p "$t/root/a" "$t/snap"
i=0
while [ "$i" -lt 3000 ]; do
    echo "$i" > "$t/root/a/f$i"
    i=$((i + 1))
done
# room for the 512 MB reserve and a few small snapshots
mount -t tmpfs -o size=600m tmpfs "$t/snap"
export FREESPACE_CONFIG="$t/none.toml" FREESPACE_SNAPSHOT_DIR="$t/snap"
unset FREESPACE_FAULT FREESPACE_PAUSE

"$bin" scan "$t/root" 2>/dev/null
"$bin" scan "$t/root" 2>/dev/null
sums() { sha256sum "$t/snap/snapshot.db" "$t/snap/snapshot.prev.db"; }
before=$(sums)
leftovers() { find "$t/snap" -mindepth 1 \( -name '*.tmp' -o -name '*-journal' -o -name '.pause-*' \) -print; }

echo "1. precheck on a full disk"
fallocate -l 200m "$t/snap/fill"
code=0
"$bin" scan "$t/root" --json > "$t/out.json" 2>/dev/null || code=$?
[ "$code" = 1 ] || fail "precheck: exit $code"
jq -e '.error.code == "SPACE"' "$t/out.json" > /dev/null || fail "$(cat "$t/out.json")"
[ "$(sums)" = "$before" ] || fail "precheck changed a snapshot"
"$bin" status --json | jq -e '.generation == 2 and .last_error.op == "precheck"' > /dev/null ||
    fail "status after precheck: $("$bin" status --json)"
rm "$t/snap/fill"

echo "2. the disk fills up during the write"
FREESPACE_PAUSE=prechecked "$bin" scan "$t/root" --json > "$t/out.json" 2>/dev/null &
pid=$!
n=0
while [ ! -e "$t/snap/.pause-prechecked" ]; do
    sleep 0.05
    n=$((n + 1))
    [ "$n" -lt 400 ] || fail "the scan never reached the pause point"
done
avail=$(df --output=avail -B1 "$t/snap" | tail -1)
fallocate -l "$avail" "$t/snap/fill" 2> /dev/null || fallocate -l $((avail - 65536)) "$t/snap/fill"
rm "$t/snap/.pause-prechecked"
code=0
wait "$pid" || code=$?
[ "$code" = 1 ] || fail "write on a full disk: exit $code"
jq -e '.error.code == "SPACE"' "$t/out.json" > /dev/null || fail "$(cat "$t/out.json")"
[ "$(sums)" = "$before" ] || fail "a failed write changed a snapshot"
[ -z "$(leftovers)" ] || fail "left behind: $(leftovers)"
"$bin" status --json | jq -e '.generation == 2 and .last_error.op != null' > /dev/null ||
    fail "status after the failed write: $("$bin" status --json)"

rm "$t/snap/fill"
"$bin" scan "$t/root" 2>/dev/null
"$bin" status --json | jq -e '.generation == 3 and .last_error == null' > /dev/null ||
    fail "status after recovery: $("$bin" status --json)"
[ -z "$(leftovers)" ] || fail "left behind: $(leftovers)"
echo "ok: precheck and disk-full keep the published snapshots"
