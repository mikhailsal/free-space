#!/bin/sh
# A filesystem mounted inside a scanned root (T4 of
# docs/designs/code-quality-gates.md, D0). Needs root for the tmpfs:
#   cargo build && sudo scripts/mount-fs.sh target/debug/freespace
# 1. the scan stops at the mount: it shows as a mount point of 0 bytes, ls
#    marks it and says it was not scanned;
# 2. refresh of the mount point is a usage error that says why;
# 3. refresh of its parent walks up to the mount again and keeps it a mount.
set -eu

[ "$#" -eq 1 ] || { echo "usage: $0 path/to/freespace" >&2; exit 2; }
bin=$(realpath "$1")
t=$(mktemp -d)
cleanup() {
    umount "$t/root/a/m" 2>/dev/null || true
    rm -rf "$t"
}
trap cleanup EXIT INT TERM
fail() {
    echo "FAIL: $*" >&2
    exit 1
}

mkdir -p "$t/root/a/m"
head -c 65536 /dev/zero > "$t/root/a/f"
mount -t tmpfs -o size=16m tmpfs "$t/root/a/m"
head -c 1048576 /dev/zero > "$t/root/a/m/inside"
export FREESPACE_CONFIG="$t/none.toml" FREESPACE_SNAPSHOT_DIR="$t/snap"
unset FREESPACE_FAULT FREESPACE_PAUSE
a="$t/root/a"

# the mount as ls --json shows it: kind, size, filesystem, items
mount_row() {
    "$bin" ls "$a" --json | jq -c '.entries[] | select(.name == "m") | [.kind, .allocated, .mount.fs_type, .items]'
}
want='["mount",0,"tmpfs",0]'

echo "1. the scan stops at the mount"
"$bin" scan "$t/root" 2> "$t/err" || fail "scan: $(cat "$t/err")"
[ "$(mount_row)" = "$want" ] || fail "scan: $(mount_row)"
"$bin" ls "$a" | grep -q '  m  \[mount tmpfs, not scanned here\]$' || fail "ls: $("$bin" ls "$a")"
note=$("$bin" ls "$a/m" | tail -n 1)
[ "$note" = "Not scanned here: separate filesystem (tmpfs)." ] || fail "ls m: $note"

echo "2. refresh of the mount point is refused"
code=0
"$bin" refresh "$a/m" --json > "$t/out.json" 2>/dev/null || code=$?
[ "$code" = 2 ] || fail "refresh m: exit $code"
msg="$a/m is a mount point outside this root's filesystem; refresh is per filesystem"
jq -e --arg m "$msg" '.error.code == "USAGE" and .error.message == $m' "$t/out.json" > /dev/null ||
    fail "refresh m: $(cat "$t/out.json")"
"$bin" status --json | jq -e '.generation == 1 and .last_error == null' > /dev/null ||
    fail "status after the refusal: $("$bin" status --json)"

echo "3. refresh of the parent keeps the mount"
"$bin" refresh "$a" 2> "$t/err" || fail "refresh: $(cat "$t/err")"
[ "$(mount_row)" = "$want" ] || fail "refresh: $(mount_row)"
"$bin" status --json | jq -e '.generation == 2 and .last_error == null' > /dev/null ||
    fail "status: $("$bin" status --json)"
echo "ok: a mount inside a root is a mount point of 0 bytes, refreshed only from above"
