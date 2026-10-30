#!/usr/bin/env bash
# Dedicated GitHub VM only. A separate ext4 filesystem supplies enough inodes.
set -euo pipefail
if [[ ${GITHUB_ACTIONS:-} != true || ${RUNNER_ENVIRONMENT:-} != github-hosted ]]; then
  echo 'This benchmark requires an ephemeral GitHub-hosted runner.' >&2
  exit 1
fi
case ${BENCH_IO:?} in native|limited) ;; *) exit 2 ;; esac
repo=$(pwd)
output="$repo/representative-results"
mkdir "$output"
volume=$(mktemp -d "$RUNNER_TEMP/representative-volume.XXXXXX")
mount_dir="$volume/mount"
loop_device=''
cleanup() {
  local status=$?
  if mountpoint -q "$mount_dir"; then sudo umount "$mount_dir" || status=1; fi
  if [[ -n $loop_device ]] && ! mountpoint -q "$mount_dir"; then
    sudo losetup -d "$loop_device" || status=1
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
df -h > "$output/host-disk.txt"
lsblk -o NAME,MAJ:MIN,SIZE,TYPE,ROTA,MODEL >> "$output/host-disk.txt"
# No host cache or device is changed outside this ephemeral runner.
# Sparse capacity is a ceiling, not a claim that the host has 48 GiB free.
available=$(df -B1 --output=avail "$volume" | tail -1)
if (( available < 32 * 1024 * 1024 * 1024 )); then
  echo 'Need at least 32 GiB free on the runner for the dedicated filesystem.' >&2
  exit 1
fi
truncate -s 48G "$volume/disk.img"
loop_device=$(sudo losetup --find --show --direct-io=on "$volume/disk.img")
sudo mkfs.ext4 -q -F -N 8500000 -m 0 -E lazy_itable_init=0,lazy_journal_init=0 "$loop_device"
mkdir "$mount_dir"
sudo mount "$loop_device" "$mount_dir"
sudo chown "$(id -u):$(id -g)" "$mount_dir"
findmnt -T "$mount_dir" -J > "$output/filesystem.json"
losetup -l -J > "$output/loop-devices.json"
df -i "$mount_dir" > "$output/inodes.txt"
image="representative-calibration:${GITHUB_RUN_ID}-${GITHUB_JOB}-${BENCH_IO}"
docker build -t "$image" scripts/bench-v2-container
chmod +x binaries/candidate
extra=()
if [[ $BENCH_IO == limited ]]; then
  extra=(--io-device "$loop_device" --read-bps 33554432 --write-bps 16777216 --read-iops 1000 --write-iops 500)
fi
python3 scripts/bench-disk-calibration.py --image "$image" --device "$loop_device" \
  --directory "$mount_dir" --output "$output" --profile "$BENCH_IO"
mkdir "$mount_dir/jobs"
export RUNNER_TEMP="$mount_dir/jobs"
python3 scripts/bench-v2-container.py --candidate binaries/candidate --current-backends \
  --output "$output/measurements" --steps "${BENCH_FILES:?}" --memory 4096 \
  --workers 1 --cpus 2 --scans "${BENCH_SCANS:?}" --repeats "${BENCH_REPEATS:?}" \
  --refreshes 3 --scan-profile disk --keep-previous true --timeout-seconds 7200 "${extra[@]}"
python3 scripts/summarize-v2-container.py "$output/measurements" | tee "$output/summary.md" >> "$GITHUB_STEP_SUMMARY"
