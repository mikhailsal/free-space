#!/bin/bash
set -euo pipefail
binary=$1 scratch=$2 shim=$3 caller_ns=$4
current_ns=$(readlink /proc/self/ns/mnt)
[[ "$current_ns" != "$caller_ns" ]] || { echo 'namespace isolation failed' >&2; exit 1; }
mount --make-rprivate /
[[ $(findmnt -n -o PROPAGATION /) == private ]] || { echo 'root propagation is not private' >&2; exit 1; }
owner=$(stat -c %u "$scratch") group=$(stat -c %g "$scratch")
volume=$scratch/volume
mkdir "$volume"
cleanup() {
  trap '' HUP INT QUIT PIPE TERM
  if mountpoint -q "$volume"; then umount "$volume"; fi
  chown "$owner:$group" "$volume"
}
trap cleanup EXIT
mount -t tmpfs -o size=768m,mode=0700,nosuid,nodev tmpfs "$volume"
[[ $(findmnt -n -o FSTYPE --target "$volume") == tmpfs ]]
[[ $(findmnt -n -o TARGET --target "$volume") == "$volume" ]]
[[ $(findmnt -n -o PROPAGATION --target "$volume") == private ]]
# Both identities are supplied to Rust and verified again before filling.
VOLUME_NAMESPACE=$current_ns VOLUME_CALLER_NAMESPACE=$caller_ns \
VOLUME_ROOT=$volume VOLUME_SHIM=$shim \
  timeout --kill-after=5s 90s "$binary" --exact real_volume_namespace_child --nocapture
