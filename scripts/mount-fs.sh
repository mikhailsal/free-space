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

# Re-exec before creating any fixture: even SIGKILL cannot propagate our
# mounts to the caller. No --fork: preserve the pid and signal traps.
# Check the namespace identity so a failed/no-op launcher cannot run mounts.
if [ "${1:-}" = --private-mount-namespace ]; then
    [ "$#" -eq 2 ] || exit 2
    current_namespace=$(readlink /proc/self/ns/mnt)
    parent_namespace=$(readlink "/proc/$PPID/ns/mnt")
    init_namespace=$(readlink /proc/1/ns/mnt)
    if [ "$current_namespace" = "$parent_namespace" ] || [ "$current_namespace" = "$init_namespace" ]; then
        echo "refusing to mount in the caller's mount namespace or init's" >&2
        exit 1
    fi
    shift
else
    [ "$#" -eq 1 ] || { echo "usage: $0 path/to/freespace" >&2; exit 2; }
    exec unshare --mount --propagation private -- \
        sh "$0" --private-mount-namespace "$@"
fi

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

# Start from ordinary directories, so their cached kind is Dir, then cover
# both the target and a cached descendant with mounts after the scan.
echo "5. mounts created after the scan retain the original root boundary"
umount "$a/m"
umount "$a/b"
mkdir -p "$a/m/child" "$a/m/missing" "$a/b/child" "$a/b/missing" "$t/other/child"
"$bin" scan "$t/other" "$t/root" 2> "$t/err" || fail "late baseline scan"
"$bin" scan "$t/other" "$t/root" 2> "$t/err" || fail "late baseline previous"
mount -t tmpfs -o size=16m tmpfs "$a/m"
mkdir "$a/m/child"
head -c 2097152 /dev/zero > "$a/m/child/foreign"
mount --bind "$t/other" "$a/b"
# A refusal must preserve even a previous failure, not merely keep null.
jq '.last_error = {ts: 1, op: "scan", message: "retained failure"}' "$t/snap/state.json" > "$t/state"
cp "$t/state" "$t/snap/state.json"

seal() {
    # Hashes protect contents; inode/size identity protects publication/rotation.
    sha256sum "$t/snap/snapshot.db" "$t/snap/snapshot.prev.db" "$t/snap/state.json"
    stat -c '%i %s' "$t/snap/snapshot.db" "$t/snap/snapshot.prev.db"
}
refuse_late() {
    target=$1
    before=$(seal)
    code=0
    "$bin" refresh "$target" --json > "$t/out.json" 2> "$t/err" || code=$?
    [ "$code" = 2 ] || fail "late refresh $target: exit $code: $(cat "$t/err")"
    msg="$target is a mount point outside this root's filesystem; refresh is per filesystem"
    jq -e --arg m "$msg" '.error.code == "USAGE" and .error.message == $m' "$t/out.json" > /dev/null ||
        fail "late refresh: $(cat "$t/out.json")"
    [ "$(seal)" = "$before" ] || fail "late refusal changed snapshot or state"
    [ ! -e "$t/snap/progress.json" ] || fail "late refusal left progress behind"
}
for target in "$a/m" "$a/m/child" "$a/m/missing" "$a/b" "$a/b/child" "$a/b/missing"; do
    refuse_late "$target"
done
# None of the foreign names or bytes may enter the original cached subtree.
"$bin" ls "$a/m/child" --json | jq -e '.entries | length == 0' > /dev/null || fail "late tmpfs imported foreign entries"
"$bin" ls "$a/b/child" --json | jq -e '.entries | length == 0' > /dev/null || fail "late bind imported foreign entries"
a_allocated=$(( $(stat -c %b "$a") * 512 + $(stat -c %b "$a/f") * 512 ))
a_apparent=$(( $(stat -c %s "$a") + $(stat -c %s "$a/f") ))
check_late_totals() {
    "$bin" ls "$t/root" --json | jq -e --argjson allocated "$a_allocated" --argjson apparent "$a_apparent" \
        '.entries[] | select(.name == "a") | .allocated == $allocated and .apparent == $apparent and .items == 3' > /dev/null ||
        fail "late mount aggregate includes foreign bytes/items"
}
"$bin" refresh "$a" 2> "$t/err" || fail "late parent refresh"
check_late_totals
[ "$(mount_row)" = "$want" ] || fail "late parent tmpfs row"
[ "$(bind_row)" = "$want_bind" ] || fail "late parent bind row"
"$bin" refresh "$t/root/c" 2> "$t/err" || fail "late sibling refresh"
check_late_totals
[ "$(mount_row)" = "$want" ] || fail "late sibling tmpfs row"
[ "$(bind_row)" = "$want_bind" ] || fail "late sibling bind row"
"$bin" status --json | jq -e '.last_error == null' > /dev/null || fail "successful retry did not clear failure"
# An explicit root, including a live mounted filesystem, is its own boundary.
"$bin" refresh "$t/root" 2> "$t/err" || fail "configured root refresh"
"$bin" scan "$a/m" 2> "$t/err" || fail "mounted configured root scan"
"$bin" refresh "$a/m" 2> "$t/err" || fail "mounted configured root refresh"

# Cross-mounts permits a late foreign filesystem but still stops subtree binds.
echo "6. cross-mounts keeps the walker's distinct bind policy"
umount "$a/m"
umount "$a/b"
"$bin" scan "$t/root" --cross-mounts 2> "$t/err" || fail "cross-mounts baseline"
"$bin" scan "$t/root" --cross-mounts 2> "$t/err" || fail "cross-mounts previous"
mount -t tmpfs -o size=16m tmpfs "$a/m"
head -c 2097152 /dev/zero > "$a/m/foreign"
mount --bind "$t/other" "$a/b"
refuse_late "$a/b"
refuse_late "$a/b/child"
"$bin" refresh "$a/m" 2> "$t/err" || fail "cross-mounts late tmpfs refresh"
"$bin" ls "$a/m" --json | jq -e '.entries | any(.name == "foreign" and .apparent == 2097152)' > /dev/null ||
    fail "cross-mounts did not import the permitted foreign file"
echo "ok: late mounts preserve scan policy and refusal leaves both snapshots and state intact"
