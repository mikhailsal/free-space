#!/bin/sh
# A filesystem mounted inside a scanned root (T4 of
# docs/designs/code-quality-gates.md, D0). Needs root for the tmpfs and the
# bind mount:
#   cargo build && sudo scripts/mount-fs.sh target/debug/freespace
# 1. the scan stops at the mount: it shows as a mount point of 0 bytes, ls
#    marks it and says it was not scanned; a bind mount of another folder of
#    the same filesystem is a mount point of 0 bytes too, and the walk does
#    not go into it;
# 2. refresh of the mount point is a usage error that says why, and records
#    nothing in state.json;
# 3. refresh of its parent walks up to the mount again and keeps it a mount;
# 4. refresh of a sibling folder keeps both mounts.
set -eu

[ "$#" -eq 1 ] || { echo "usage: $0 path/to/freespace" >&2; exit 2; }
bin=$(realpath "$1")
t=$(mktemp -d)
cleanup() {
    # a second signal must not cut the unmount short
    trap '' HUP INT QUIT PIPE TERM
    # a mount that will not unmount stays, and rm must not walk into it
    left=0
    for d in "$t/root/a/b" "$t/root/a/m"; do
        umount "$d" 2> /dev/null || ! mountpoint -q "$d" || left=1
    done
    if [ "$left" = 0 ]; then
        rm -rf --one-file-system "$t"
    else
        echo "cleanup: a mount under $t is still mounted, $t left in place" >&2
        exit 1
    fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 131' QUIT
trap 'exit 141' PIPE
trap 'exit 143' TERM
fail() {
    echo "FAIL: $*" >&2
    # a crash trace is in the snapshot dir, which cleanup removes
    cat "$t"/snap/crash-*.log >&2 2> /dev/null || true
    exit 1
}

mkdir -p "$t/root/a/m" "$t/root/a/b" "$t/root/c" "$t/other"
head -c 65536 /dev/zero > "$t/root/a/f"
head -c 4096 /dev/zero > "$t/root/c/g"
mount -t tmpfs -o size=16m tmpfs "$t/root/a/m"
head -c 1048576 /dev/zero > "$t/root/a/m/inside"
# a folder of the same filesystem, bound a second time inside the root
head -c 1048576 /dev/zero > "$t/other/inside"
mount --bind "$t/other" "$t/root/a/b"
export FREESPACE_CONFIG="$t/none.toml" FREESPACE_SNAPSHOT_DIR="$t/snap"
unset FREESPACE_FAULT FREESPACE_PAUSE
a="$t/root/a"

# the mounts as ls --json shows them: kind, size, apparent size, filesystem, items
mount_row() {
    "$bin" ls "$a" --json | jq -c '.entries[] | select(.name == "m") | [.kind, .allocated, .apparent, .mount.fs_type, .items]'
}
bind_row() {
    "$bin" ls "$a" --json | jq -c '.entries[] | select(.name == "b") | [.kind, .allocated, .apparent, .items]'
}
want='["mount",0,0,"tmpfs",0]'
want_bind='["mount",0,0,0]'

echo "1. the scan stops at the mount"
"$bin" scan "$t/root" 2> "$t/err" || fail "scan: $(cat "$t/err")"
[ "$(mount_row)" = "$want" ] || fail "scan: $(mount_row)"
[ "$(bind_row)" = "$want_bind" ] || fail "scan, bind: $(bind_row)"
"$bin" ls "$a" | grep -q '  m  \[mount tmpfs, not scanned here\]$' || fail "ls: $("$bin" ls "$a")"
note=$("$bin" ls "$a/m" | tail -n 1)
[ "$note" = "Not scanned here: separate filesystem (tmpfs)." ] || fail "ls m: $note"

echo "2. refresh of the mount point is refused"
code=0
"$bin" refresh "$a/m" --json > "$t/out.json" 2> "$t/err" || code=$?
[ "$code" = 2 ] || fail "refresh m: exit $code: $(cat "$t/err")"
msg="$a/m is a mount point outside this root's filesystem; refresh is per filesystem"
jq -e --arg m "$msg" '.error.code == "USAGE" and .error.message == $m' "$t/out.json" > /dev/null ||
    fail "refresh m: $(cat "$t/out.json")"
# a refusal records nothing; step 3's refresh would clear last_error, so check it here
"$bin" status --json | jq -e '.generation == 1 and .last_error == null' > /dev/null ||
    fail "status after the refusal: $("$bin" status --json)"

echo "3. refresh of the parent keeps the mount"
"$bin" refresh "$a" 2> "$t/err" || fail "refresh: $(cat "$t/err")"
[ "$(mount_row)" = "$want" ] || fail "refresh: $(mount_row)"
"$bin" status --json | jq -e '.generation == 2 and .last_error == null' > /dev/null ||
    fail "status: $("$bin" status --json)"

echo "4. refresh of a sibling keeps the mounts"
head -c 8192 /dev/zero > "$t/root/c/h"
"$bin" refresh "$t/root/c" 2> "$t/err" || fail "refresh c: $(cat "$t/err")"
[ "$(mount_row)" = "$want" ] || fail "refresh c: $(mount_row)"
[ "$(bind_row)" = "$want_bind" ] || fail "refresh c, bind: $(bind_row)"
"$bin" ls "$a" | grep -q '  m  \[mount tmpfs, not scanned here\]$' || fail "ls after refresh c: $("$bin" ls "$a")"
"$bin" status --json | jq -e '.generation == 3 and .last_error == null' > /dev/null ||
    fail "status after refresh c: $("$bin" status --json)"
echo "ok: mounts inside a root are mount points of 0 bytes, and a refresh that does not reach them keeps them"
